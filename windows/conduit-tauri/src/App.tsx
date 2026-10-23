import { useCallback, useEffect, useRef } from "react"
import { Group, Panel, Separator as ResizeSeparator } from "react-resizable-panels"
import { motion, AnimatePresence } from "motion/react"
import { ArrowLeftRight, BellRing, Check, CloudOff, Command, Moon, RefreshCw, Search, Sun, X } from "lucide-react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Sidebar, pageLabels } from "@/components/Sidebar"
import { PageContents, type PageActions } from "@/components/Pages"
import { activeTransfers, isNative } from "@/lib/model"
import { fetchSnapshot, ensureDaemon, runAction, updateSettings, removeHistory, selectFileAndSend, sendDroppedFile, openLink, openFolder, watchDaemon, watchShellErrors } from "@/lib/bridge"
import { useUi } from "@/store/ui"
import "./App.css"

function useDesktopController(): PageActions {
  const setSnapshot = useUi((s) => s.setSnapshot)
  const setNotice = useUi((s) => s.setNotice)
  const setBusy = useUi((s) => s.setBusy)

  const refresh = useCallback(async () => {
    const data = await fetchSnapshot()
    setSnapshot(data)
  }, [setSnapshot])

  const perform = useCallback(async (operation: () => Promise<unknown>, success?: string) => {
    setBusy(true)
    try {
      await operation()
      if (success) setNotice(success)
      await refresh()
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error), true)
    } finally {
      setBusy(false)
    }
  }, [refresh, setBusy, setNotice])

  const send = useCallback(() => perform(async () => {
    const chosen = await selectFileAndSend()
    if (chosen) setNotice("File accepted. Track its progress on the Transfers page.")
  }), [perform, setNotice])

  return {
    send,
    refresh: () => perform(refresh),
    run: (action, id) => perform(() => runAction(action, id), action === "cancel-transfer" ? "Cancellation requested" : undefined),
    clear: (kind) => perform(() => removeHistory(kind), "History cleared"),
    save: (settings) => perform(() => updateSettings(settings), "Settings saved to the native daemon"),
    openLink: (url) => perform(() => openLink(url)),
    openFolder: (kind) => perform(() => openFolder(kind)),
  }
}

function Desktop() {
  const page = useUi((s) => s.page)
  const search = useUi((s) => s.search)
  const setSearch = useUi((s) => s.setSearch)
  const snapshot = useUi((s) => s.snapshot)
  const busy = useUi((s) => s.busy)
  const notice = useUi((s) => s.notice)
  const setNotice = useUi((s) => s.setNotice)
  const collapsed = useUi((s) => s.collapsed)
  const theme = useUi((s) => s.theme)
  const setTheme = useUi((s) => s.setTheme)
  const searchRef = useRef<HTMLInputElement>(null)
  const mainScrollRef = useRef<HTMLElement>(null)
  const actions = useDesktopController()
  const actionsRef = useRef(actions)
  useEffect(() => { actionsRef.current = actions }, [actions])

  // The scroll container is shared between pages. Each navigation should start
  // at the new page header, even when the previous page was scrolled to the end.
  useEffect(() => { mainScrollRef.current?.scrollTo({ top: 0, behavior: "instant" }) }, [page])

  useEffect(() => {
    const matcher = window.matchMedia("(prefers-color-scheme: dark)")
    const update = () => document.documentElement.classList.toggle("dark",
      theme === "dark" || (theme === "system" && matcher.matches))
    update()
    matcher.addEventListener("change", update)
    return () => matcher.removeEventListener("change", update)
  }, [theme])

  useEffect(() => {
    let active = true
    let unlisten: (() => void) | undefined
    void watchShellErrors((message) => {
      if (!active) return
      // Present Explorer send errors in the same Transfers UI that owns the
      // history. An already-running instance must not drop launch feedback.
      useUi.getState().setPage("transfers")
      setNotice(message, true)
    }).then((off) => { if (active) unlisten = off; else off() })
      .catch((error) => { if (active) setNotice(String(error), true) })
    return () => { active = false; unlisten?.() }
  }, [setNotice])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault()
        searchRef.current?.focus()
      }
      if (event.key === "Escape" && document.activeElement === searchRef.current) {
        setSearch("")
        searchRef.current?.blur()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [setSearch])

  useEffect(() => {
    let active = true
    let unsubscribe: (() => void) | undefined
    // Native file watchers deliver low-latency updates. A slow reconciliation
    // additionally heals missed events, daemon restarts and network handoffs.
    const reconcile = isNative() ? window.setInterval(() => {
      if (!useUi.getState().busy) void fetchSnapshot()
        .then((snapshot) => { if (active) useUi.getState().setSnapshot(snapshot) })
        .catch(() => { /* keep last known state until the daemon is reachable */ })
    }, 5000) : undefined
    async function initialize() {
      if (isNative()) {
        try { await ensureDaemon() } catch (error) {
          if (active) setNotice(error instanceof Error ? error.message : String(error), true)
        }
      }
      if (!active) return
      try { await fetchSnapshot().then(useUi.getState().setSnapshot) }
      catch (error) { setNotice(error instanceof Error ? error.message : String(error), true) }
      if (!active) return
      try {
        unsubscribe = await watchDaemon(
          () => void actionsRef.current.refresh(),
          (paths) => {
            if (!useUi.getState().snapshot?.connection.linked) {
              setNotice("Connect your phone before dropping files.", true)
              return
            }
            if (paths.length > 12) {
              setNotice("Send up to 12 files at once.", true)
              return
            }
            void (async () => {
              for (const path of paths) {
                try { await sendDroppedFile(path) }
                catch (error) { setNotice(error instanceof Error ? error.message : String(error), true); break }
              }
              await actionsRef.current.refresh()
            })()
          },
        )
      } catch (error) { setNotice(error instanceof Error ? error.message : String(error), true) }
    }
    void initialize()
    return () => {
      active = false
      unsubscribe?.()
      if (reconcile !== undefined) window.clearInterval(reconcile)
    }
  }, [setNotice])

  const transfers = snapshot ? activeTransfers(snapshot) : []
  const showActiveStrip = transfers.length > 0 && page !== "transfers"
  return (
    <TooltipProvider>
      <div className="desktop-root">
        {!isNative() && <div className="preview-strip">Browser design preview · All content is sample data · Native controls are disabled</div>}
        <Group key={collapsed ? "collapsed" : "expanded"} orientation="horizontal" className="flex min-h-0 flex-1">
          <Panel id="nav" defaultSize={collapsed ? "72px" : "258px"} minSize={collapsed ? "72px" : "216px"} maxSize={collapsed ? "72px" : "355px"}>
            {snapshot ? <Sidebar snapshot={snapshot} onSend={() => void actions.send()} /> :
              <div className="sidebar-shell flex h-full items-start gap-2 p-5"><div className="brand-mark flex size-8 items-center justify-center rounded-lg"><ArrowLeftRight size={18}/></div><span className="font-semibold">Conduit</span></div>}
          </Panel>
          <ResizeSeparator className="resize-handle" />
          <Panel id="main" minSize="500px" className="flex min-w-0 flex-col bg-background">
            <header className="topbar">
              <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">Workspace</span><span className="text-border">/</span><span className="truncate">{pageLabels[page]}</span>
                {busy && <RefreshCw className="ml-1 animate-spin" size={12} aria-label="Working"/>}
              </div>
              <div className="flex items-center gap-2">
                {page !== "settings" && page !== "about" && <div className="relative hidden items-center md:flex">
                  <Search size={14} className="pointer-events-none absolute left-2.5 text-muted-foreground"/>
                  <Input ref={searchRef} aria-label="Search current section" value={search} onChange={(event) => setSearch(event.target.value)}
                    placeholder={"Search " + pageLabels[page].toLowerCase()} className="h-8 w-[190px] pl-8 pr-10 text-xs lg:w-[235px]"/>
                  <kbd className="pointer-events-none absolute right-2 rounded border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">Ctrl K</kbd>
                </div>}
                <Button size="icon-sm" variant="ghost" aria-label="Toggle light or dark theme"
                  title={theme === "dark" ? "Dark mode" : "Light mode"}
                  onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
                  {theme === "dark" ? <Moon size={16}/> : <Sun size={16}/>}
                </Button>
                <Button size="icon-sm" variant="ghost" aria-label="Refresh data" title="Refresh"
                  onClick={() => void actions.refresh()}><RefreshCw size={15}/></Button>
                <div className="relative ml-1 flex size-7 items-center justify-center rounded-full border bg-muted/60">
                  {snapshot?.connection.linked ? <Check size={14}/> : <CloudOff size={14}/>}
                </div>
              </div>
            </header>

            <main ref={mainScrollRef} className="main-scroll" id="content">
              {snapshot ? <AnimatePresence mode="wait">
                <motion.div key={page} initial={{ opacity: 0, y: 5 }} animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -3 }} transition={{ duration: 0.16, ease: "easeOut" }}>
                  <PageContents page={page} snapshot={snapshot} search={search} actions={actions}/>
                </motion.div>
              </AnimatePresence> : <div className="page-inner flex h-full items-center justify-center text-sm text-muted-foreground">
                <RefreshCw size={16} className="mr-2 animate-spin"/> Reading Conduit service…
              </div>}
            </main>

            {showActiveStrip && <div className="active-strip">
              <div className="flex min-w-0 flex-1 items-center gap-3">
                <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted"><Command size={16}/></div>
                <div className="min-w-0"><div className="truncate text-[12px] font-medium">{transfers[0].name}</div>
                  <div className="text-[11px] text-muted-foreground">{Math.round(transfers[0].percentage)}% · {transfers.length} active</div></div>
              </div>
              <div className="progress-track mx-3 hidden w-[100px] lg:block"><div className="progress-fill" style={{ width: transfers[0].percentage + "%" }}/></div>
              <Button variant="outline" size="xs" onClick={() => void actions.run("cancel-transfer", transfers[0].id)}>Cancel</Button>
              <Button variant="ghost" size="xs" onClick={() => useUi.getState().setPage("transfers")}>View all</Button>
            </div>}

            {notice && <div role={notice.error ? "alert" : "status"} className={"app-notice " + (notice.error ? "app-notice-error" : "")}>
              <BellRing size={16} className="shrink-0"/><p className="min-w-0 flex-1">{notice.message}</p>
              <button title="Dismiss notification" aria-label="Dismiss notification" onClick={() => setNotice(null)}><X size={15}/></button>
            </div>}
          </Panel>
        </Group>
      </div>
    </TooltipProvider>
  )
}

export default Desktop
