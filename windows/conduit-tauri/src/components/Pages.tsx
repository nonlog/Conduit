import { useState, type ReactNode } from "react"
import { Activity, ArrowDownLeft, ArrowRight, ArrowUpRight, Bell, Check, CheckCircle2, CircleHelp, Clock3, ExternalLink, FileClock, FileText, FolderOpen, Globe2, HardDriveDownload, Laptop2, Link2, Monitor, PauseCircle, RotateCw, Send, Settings2, ShieldCheck, Smartphone, Trash2, Wifi, XCircle } from "lucide-react"
import { motion } from "motion/react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Separator } from "@/components/ui/separator"
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle, AlertDialogTrigger } from "@/components/ui/alert-dialog"
import type { DesktopSettings, NotificationEntry, SharedLinkEntry, Snapshot, TransferEntry, Page } from "@/lib/model"
import { activeTransfers, formatBytes, relativeTime } from "@/lib/model"
import { cn } from "@/lib/utils"
import { useUi } from "@/store/ui"
import { pageLabels } from "@/components/Sidebar"

export interface PageActions {
  send: () => Promise<void>
  run: (action: string, id?: string) => Promise<void>
  clear: (kind: "notifications" | "links") => Promise<void>
  save: (settings: DesktopSettings) => Promise<void>
  openLink: (url: string) => Promise<void>
  openFolder: (kind: "received" | "data") => Promise<void>
  openLegacyUi: () => Promise<void>
  refresh: () => Promise<void>
}

function Header({ page, subtitle, children }: { page: Page; subtitle: string; children?: ReactNode }) {
  return <div className="section-header">
    <div className="min-w-0"><div className="eyebrow">WORKSPACE / {pageLabels[page].toUpperCase()}</div>
      <h1 className="section-title">{pageLabels[page]}</h1>
      <p className="section-subtitle">{subtitle}</p></div>
    {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
  </div>
}

function Empty({ icon: Icon, title, description }: { icon: typeof Bell; title: string; description: string }) {
  return <div className="flex min-h-[240px] flex-col items-center justify-center gap-2 px-4 text-center">
    <div className="mb-2 flex size-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground"><Icon size={23} strokeWidth={1.7}/></div>
    <strong className="text-[14px] font-medium">{title}</strong><p className="max-w-[320px] text-xs leading-relaxed text-muted-foreground">{description}</p>
  </div>
}

function SummaryCard({ icon: Icon, label, value, detail, onClick }: { icon: typeof Bell; label: string; value: number; detail: string; onClick: () => void }) {
  return <button onClick={onClick} className="overview-metric group">
    <div className="flex items-center justify-between text-muted-foreground"><Icon size={17} strokeWidth={1.8}/><ArrowUpRight size={14} className="opacity-0 transition-opacity group-hover:opacity-100"/></div>
    <strong className="mt-3 block font-heading text-2xl font-semibold tracking-tight">{value}</strong>
    <span className="mt-1 block text-[13px] font-medium">{label}</span>
    <span className="mt-0.5 block text-xs text-muted-foreground">{detail}</span>
  </button>
}

function ConnectionPill({ snapshot }: { snapshot: Snapshot }) {
  return <Badge variant={snapshot.connection.linked ? "secondary" : "outline"} className="gap-1.5">
    <span className={cn("status-dot", snapshot.connection.linked ? "status-dot-on" : "status-dot-off")}/>
    {snapshot.connection.state}
  </Badge>
}

export function Overview({ snapshot, actions }: { snapshot: Snapshot; actions: PageActions }) {
  const setPage = useUi((s) => s.setPage)
  const active = activeTransfers(snapshot)
  return <div className="page-inner">
    <Header page="overview" subtitle="Your devices, working together.">
      <Button variant="outline" size="sm" onClick={() => actions.refresh()}><RotateCw size={14}/> Refresh</Button>
    </Header>
    <div className="connection-hero">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3.5">
          <div className="relative flex size-14 shrink-0 items-center justify-center overflow-hidden rounded-2xl border border-border/60 bg-background/80">
            {snapshot.wallpaper ? <img src={snapshot.wallpaper} alt="Phone wallpaper" className="h-full w-full object-cover"/> : <Smartphone className="text-muted-foreground" size={28} strokeWidth={1.6}/>}
          </div>
          <div className="min-w-0">
            <div className="mb-1.5 flex flex-wrap items-center gap-2"><span className="text-[11px] font-semibold uppercase tracking-[0.09em] text-muted-foreground">Paired device</span><ConnectionPill snapshot={snapshot}/></div>
            <div className="truncate font-heading text-[21px] font-semibold tracking-tight">{snapshot.connection.deviceName}</div>
            <div className="mt-1 flex items-center gap-1.5 text-[12px] text-muted-foreground">
              <Wifi size={13}/> {snapshot.connection.linked ? snapshot.connection.route.toUpperCase() || "Encrypted connection" : "Waiting for a connection"}
              {snapshot.connection.relay && <span>· Relay</span>}
            </div>
          </div>
        </div>
        <Button onClick={actions.send} disabled={!snapshot.connection.linked || !snapshot.daemonAvailable} size="lg" className="h-9"><Send size={16}/> Send a file</Button>
      </div>
      <div className="mt-6 flex items-center justify-between gap-3 border-t pt-4">
        <div className="flex items-center gap-2 text-[12px] text-muted-foreground"><ShieldCheck size={15} className="text-emerald-600 dark:text-emerald-400"/> Noise end-to-end encrypted</div>
        <button onClick={() => setPage("settings")} className="flex items-center gap-1 text-xs font-medium text-foreground hover:underline">Manage device <ArrowRight size={13}/></button>
      </div>
    </div>
    <div className="mb-3 mt-8 flex items-center justify-between"><h2 className="minor-heading">At a glance</h2><span className="text-xs text-muted-foreground">Recent activity</span></div>
    <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
      <SummaryCard icon={Bell} label="Notifications" value={snapshot.notifications.length} detail="Messages from your phone" onClick={() => setPage("notifications")}/>
      <SummaryCard icon={Link2} label="Shared links" value={snapshot.sharedLinks.length} detail="Ready to open on desktop" onClick={() => setPage("links")}/>
      <SummaryCard icon={FileClock} label="File transfers" value={snapshot.transfers.length} detail={active.length ? active.length + " in progress" : "History in one place"} onClick={() => setPage("transfers")}/>
    </div>
    {active.length > 0 && <div className="mt-7">
      <h2 className="minor-heading mb-3">In progress <Badge variant="secondary">{active.length}</Badge></h2>
      <Card className="p-0"><div className="space-y-3 p-4">
        {active.slice(0, 2).map((transfer) => <div className="space-y-1.5" key={transfer.id}>
          <div className="flex items-center justify-between gap-4 text-[13px]"><span className="truncate font-medium">{transfer.name}</span><span className="tabular-nums text-muted-foreground">{Math.round(transfer.percentage)}%</span></div>
          <div className="progress-track"><motion.div initial={false} animate={{ width: transfer.percentage + "%" }} className="progress-fill"/></div>
        </div>)}
        <button className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground" onClick={() => setPage("transfers")}>Manage active transfers <ArrowRight size={13}/></button>
      </div></Card>
    </div>}
    <div className="mt-8 flex items-center gap-3 text-xs text-muted-foreground"><Activity size={14}/> Connection and activity are provided by the existing Conduit daemon.</div>
  </div>
}

function ConfirmClear({ noun, onConfirm }: { noun: string; onConfirm: () => void }) {
  return <AlertDialog>
    <AlertDialogTrigger render={<Button variant="outline" size="sm" aria-label={"Clear " + noun}><Trash2 size={14}/> Clear history</Button>}/>
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Clear {noun}?</AlertDialogTitle>
        <AlertDialogDescription>This removes locally stored {noun} records. The connected phone and its data will not be changed.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter><AlertDialogCancel>Keep history</AlertDialogCancel>
        <AlertDialogAction variant="destructive" onClick={onConfirm}>Clear history</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
}

export function Notifications({ snapshot, search, actions }: { snapshot: Snapshot; search: string; actions: PageActions }) {
  const filtered = snapshot.notifications.filter((item) => [item.app, item.title, item.body].some((text) => text.toLowerCase().includes(search.toLowerCase())))
  return <div className="page-inner">
    <Header page="notifications" subtitle="Phone notifications, together on desktop.">
      {snapshot.notifications.length > 0 && <ConfirmClear noun="notifications" onConfirm={() => void actions.clear("notifications")}/>}
    </Header>
    <Card className="p-0">
      {filtered.length === 0 ? <Empty icon={Bell} title={search ? "No matching notifications" : "All caught up"} description="Notifications forwarded by your phone will appear here."/>
      : <div className="divide-y">
        {filtered.map((item: NotificationEntry) => <div key={item.id} className="record-row">
          <div className="record-icon">{item.icon ? <img src={item.icon} alt="" className="size-8 rounded-md object-cover"/> : <Bell size={17}/>}</div>
          <div className="min-w-0 flex-1"><div className="mb-1 flex flex-wrap items-center gap-2"><span className="font-medium">{item.app}</span><span className="text-[11px] text-muted-foreground">{relativeTime(item.timestamp)}</span></div>
            <p className="text-[13px] font-medium">{item.title}</p>
            {item.body && <p className="mt-1 text-[12px] leading-[1.55] break-words text-muted-foreground">{item.body}</p>}
          </div>
        </div>)}
      </div>}
    </Card>
  </div>
}

export function Links({ snapshot, search, actions }: { snapshot: Snapshot; search: string; actions: PageActions }) {
  const filtered = snapshot.sharedLinks.filter((link) => [link.title, link.url, link.source].some((text) => text.toLowerCase().includes(search.toLowerCase())))
  return <div className="page-inner">
    <Header page="links" subtitle="Continue reading where you left off.">
      {snapshot.sharedLinks.length > 0 && <ConfirmClear noun="shared links" onConfirm={() => void actions.clear("links")}/>}
    </Header>
    <Card className="p-0">{filtered.length === 0 ? <Empty icon={Link2} title={search ? "No matching links" : "No links shared yet"} description="Links shared from your phone will be collected here."/>
      : <div className="divide-y">
        {filtered.map((link: SharedLinkEntry) => <button onClick={() => actions.openLink(link.url)} key={link.id} className="record-row group w-full text-left" aria-label={"Open " + link.title}>
          <div className="record-icon"><Globe2 size={19}/></div>
          <div className="min-w-0 flex-1"><div className="truncate text-[13px] font-medium group-hover:underline">{link.title}</div>
            <div className="mt-1 truncate text-[12px] text-muted-foreground">{link.url}</div>
            <div className="mt-1.5 text-[11px] text-muted-foreground">{relativeTime(link.timestamp)} · {link.source}</div>
          </div>
          <ExternalLink className="shrink-0 text-muted-foreground" size={16}/>
        </button>)}
      </div>}
    </Card>
  </div>
}

function TransferState({ entry }: { entry: TransferEntry }) {
  const state = entry.state
  if (state === "completed") return <Badge variant="secondary" className="gap-1.5"><CheckCircle2 size={12} className="text-emerald-600"/> Completed</Badge>
  if (state === "failed") return <Badge variant="destructive" className="gap-1"><XCircle size={12}/> Failed</Badge>
  if (state === "cancelled") return <Badge variant="outline">Cancelled</Badge>
  if (state === "queued") return <Badge variant="outline" className="gap-1"><Clock3 size={12}/> Queued</Badge>
  if (state === "waiting confirmation") return <Badge variant="secondary">Confirming</Badge>
  return <Badge variant="secondary" className="gap-1"><span className="status-dot status-dot-on"/> Transferring</Badge>
}

function TransferRow({ entry, actions }: { entry: TransferEntry; actions: PageActions }) {
  const Icon = entry.direction === "inbound" ? ArrowDownLeft : ArrowUpRight
  return <div className="record-row gap-3">
    <div className="record-icon"><Icon size={19}/></div>
    <div className="min-w-0 flex-1">
      <div className="flex flex-wrap items-center justify-between gap-2"><div className="truncate text-[13px] font-medium" title={entry.name}>{entry.name}</div><TransferState entry={entry}/></div>
      <div className="mt-1.5 flex items-center justify-between gap-3 text-[11px] text-muted-foreground">
        <span>{entry.direction === "inbound" ? "Receiving" : "Sending"} · {formatBytes(entry.bytes)} of {formatBytes(entry.total)}</span>
        <span className="tabular-nums">{Math.round(entry.percentage)}%</span>
      </div>
      <div className="progress-track mt-2.5"><motion.div initial={false} animate={{ width: entry.percentage + "%" }} transition={{ duration: 0.25 }} className={cn("progress-fill", entry.state === "failed" && "bg-destructive")}/></div>
      {entry.detail && <p className="mt-2 text-[11px] text-muted-foreground">{entry.detail}</p>}
    </div>
    {!entry.terminal && <Button variant="outline" size="sm" aria-label={"Cancel " + entry.name} onClick={() => actions.run("cancel-transfer", entry.id)}><PauseCircle size={14}/> Cancel</Button>}
  </div>
}

export function Transfers({ snapshot, search, actions }: { snapshot: Snapshot; search: string; actions: PageActions }) {
  const [filter, setFilter] = useState<"all" | "active" | "history">("all")
  const filtered = snapshot.transfers.filter((transfer) => {
    const match = transfer.name.toLowerCase().includes(search.toLowerCase())
    return match && (filter === "all" || (filter === "active" ? !transfer.terminal : transfer.terminal))
  })
  const activeCount = activeTransfers(snapshot).length
  return <div className="page-inner">
    <Header page="transfers" subtitle="Files shared between your phone and computer."><Button size="sm" onClick={actions.send} disabled={!snapshot.connection.linked || !snapshot.daemonAvailable}><Send size={14}/> Send a file</Button></Header>
    <div className="mb-4 flex items-center gap-1 rounded-lg bg-muted/70 p-1" role="tablist" aria-label="Transfer filter">
      {(["all", "active", "history"] as const).map((key) => <button key={key} role="tab" aria-selected={filter === key} onClick={() => setFilter(key)}
        className={cn("filter-pill", filter === key && "filter-pill-selected")}>
        {key === "all" ? "All" : key === "active" ? "In progress" : "History"}
        <span className="ml-1.5 text-[10px] tabular-nums text-muted-foreground">{key === "all" ? snapshot.transfers.length : key === "active" ? activeCount : snapshot.transfers.length - activeCount}</span>
      </button>)}
    </div>
    <Card className="p-0">{filtered.length === 0 ? <Empty icon={FileClock} title={search ? "No matching files" : "No transfers to show"} description="Drag files onto this window, or use Send a file to transfer securely."/>
      : <div className="divide-y">{filtered.map((entry) => <TransferRow entry={entry} actions={actions} key={entry.id}/>)}</div>}
    </Card>
    <p className="mt-4 text-xs text-muted-foreground">Recent history is persisted by the original daemon. Closing this window does not cancel queued transfers.</p>
  </div>
}

function SettingLine({ icon: Icon, label, description, children }: { icon: typeof Settings2; label: string; description: string; children?: ReactNode }) {
  return <div className="setting-line">
    <div className="record-icon shrink-0"><Icon size={17}/></div>
    <div className="min-w-0 flex-1"><div className="text-[13px] font-medium">{label}</div><div className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</div></div>
    {children && <div className="shrink-0">{children}</div>}
  </div>
}

export function Settings({ snapshot, actions }: { snapshot: Snapshot; actions: PageActions }) {
  const original = snapshot.settings
  const settingsFingerprint = JSON.stringify(original)
  const [draft, setDraft] = useState<DesktopSettings>({ ...original })
  const [relayText, setRelayText] = useState(original.relays.join("\n"))
  const changed = JSON.stringify({ ...draft, relays: relayText.split(/\s+/).filter(Boolean) }) !== settingsFingerprint
  const set = <K extends keyof DesktopSettings>(key: K, value: DesktopSettings[K]) => setDraft((old) => ({ ...old, [key]: value }))
  const doSave = () => actions.save({ ...draft, relays: relayText.split(/[\r\n;,]+/).map((line) => line.trim()).filter(Boolean) })
  return <div className="page-inner">
    <Header page="settings" subtitle="Connection, pairing, system integration and preferences.">
      <Button size="sm" onClick={doSave} disabled={!changed || !snapshot.daemonAvailable}><Check size={14}/> Save settings</Button>
    </Header>
    <h2 className="minor-heading mb-3">Device pairing</h2>
    <Card className="mb-7 p-0"><div className="p-4">
      <div className="flex items-center justify-between gap-3"><div className="flex items-center gap-2"><Smartphone size={17}/><span className="text-[13px] font-medium">{snapshot.pairing.deviceName || "No paired phone"}</span></div><ConnectionPill snapshot={snapshot}/></div>
      {snapshot.pairing.active && <div className="mt-4 rounded-xl border bg-muted/60 px-4 py-3">
        <div className="text-xs font-medium text-muted-foreground">Temporary pairing code</div>
        <div className="mt-1 font-mono text-3xl font-semibold tracking-[.28em]">{snapshot.pairing.code}</div>
        <p className="mt-2 text-[11px] text-muted-foreground">Enter this code on the Android phone. Valid for two minutes.</p>
      </div>}
      <div className="mt-4 flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={() => actions.run(snapshot.pairing.active ? "pair-cancel" : "pair-start")}><Smartphone size={14}/>{snapshot.pairing.active ? "Cancel pairing" : "Pair a phone"}</Button>
        {snapshot.pairing.deviceId && <AlertDialog>
          <AlertDialogTrigger render={<Button size="sm" variant="destructive">Forget device</Button>}/>
          <AlertDialogContent>
            <AlertDialogHeader><AlertDialogTitle>Forget this phone?</AlertDialogTitle><AlertDialogDescription>Existing pairing is removed. Re-pairing will require a new code.</AlertDialogDescription></AlertDialogHeader>
            <AlertDialogFooter><AlertDialogCancel>Keep paired</AlertDialogCancel><AlertDialogAction variant="destructive" onClick={() => actions.run("pair-forget")}>Forget phone</AlertDialogAction></AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>}
      </div>
    </div></Card>
    <h2 className="minor-heading mb-3">Relay and routing</h2>
    <Card className="mb-7 p-0">
      <div className="space-y-2 p-4"><label htmlFor="relays" className="text-[13px] font-medium">Enabled relays</label>
        <p className="text-xs text-muted-foreground">One host:port per line. Blank means the daemon default list.</p>
        <textarea id="relays" className="field-textarea h-[84px] w-full font-mono text-[12px]" value={relayText} onChange={(event) => setRelayText(event.target.value)}/>
      </div>
      <Separator/>
      <div className="space-y-2 p-4"><label htmlFor="proxy" className="text-[13px] font-medium">Relay proxy</label>
        <select id="proxy" className="field-select w-full" value={draft.proxyMode} onChange={(event) => set("proxyMode", event.target.value as DesktopSettings["proxyMode"])}>
          <option value="system">Windows system proxy</option><option value="direct">Direct connection</option><option value="manual">Manual SOCKS5</option>
        </select>
        {draft.proxyMode === "manual" && <Input aria-label="Manual SOCKS5 endpoint" placeholder="socks5h://host:port" value={draft.manualProxy} onChange={(e) => set("manualProxy", e.target.value)}/>}
      </div>
    </Card>
    <h2 className="minor-heading mb-3">Windows integration</h2>
    <Card className="mb-7 p-0">
      <SettingLine icon={Monitor} label="Start at sign-in" description="Keep the native Conduit daemon available in the background.">
        <Switch checked={original.autostart} onCheckedChange={(checked) => actions.run(checked ? "autostart-enable" : "autostart-disable")} aria-label="Start at sign-in"/>
      </SettingLine>
      <Separator/>
      <SettingLine icon={FileText} label="Explorer integration" description="Add Send with Conduit to the Windows context menu.">
        <Switch checked={original.explorerIntegration} onCheckedChange={(checked) => actions.run(checked ? "explorer-enable" : "explorer-disable")} aria-label="Explorer integration"/>
      </SettingLine>
      <Separator/>
      <SettingLine icon={Bell} label="Tray notifications" description="Enable the native daemon's tray indicator.">
        <Switch checked={draft.trayIcon} onCheckedChange={(checked) => set("trayIcon", checked)} aria-label="Tray icon"/>
      </SettingLine>
      <Separator/>
      <SettingLine icon={HardDriveDownload} label="Received files" description={draft.receiveDir || "Windows Downloads folder"}>
        <Button variant="outline" size="sm" onClick={() => actions.openFolder("received")}><FolderOpen size={14}/> Open</Button>
      </SettingLine>
      <div className="px-4 pb-4 pt-1"><label htmlFor="folder" className="sr-only">Receive folder</label><Input id="folder" placeholder="Optional custom receive directory" value={draft.receiveDir} onChange={(event) => set("receiveDir", event.target.value)}/></div>
    </Card>
    <p className="text-xs text-muted-foreground">The native daemon owns all encryption keys. Changing a setting reloads its configuration without rewriting your pairing identity.</p>
  </div>
}

export function About({ snapshot, actions }: { snapshot: Snapshot; actions: PageActions }) {
  return <div className="page-inner"><Header page="about" subtitle="A small companion for the devices you already use."/>
    <div className="flex items-center gap-4 rounded-xl border bg-card p-6"><div className="brand-mark flex size-14 items-center justify-center rounded-2xl"><Laptop2 size={29}/></div>
      <div><div className="font-heading text-xl font-semibold">Conduit</div><p className="mt-1 text-xs text-muted-foreground">{snapshot.desktopChannel === "stable" ? "Nova Desktop" : "Desktop Preview"} · v{snapshot.version}</p></div>
    </div>
    <h2 className="minor-heading mb-3 mt-7">Under the hood</h2>
    <Card className="p-0">
      <SettingLine icon={ShieldCheck} label="Existing Rust daemon" description="Noise pairing, encrypted LAN/Relay transfer, clipboard and notification handling."><Badge variant="secondary">{snapshot.daemonAvailable ? "Detected" : "Unavailable"}</Badge></SettingLine>
      <Separator/>
      <SettingLine icon={Settings2} label="New desktop UI" description="Tauri 2, React, shadcn/ui Nova, Lucide, Motion, Zustand and resizable panels."><Badge variant="outline">{snapshot.desktopChannel === "stable" ? "Default" : "Preview"}</Badge></SettingLine>
      <Separator/>
      <SettingLine icon={FolderOpen} label="Existing app data" description={snapshot.dataDir}><Button size="sm" variant="outline" onClick={() => actions.openFolder("data")}>Open</Button></SettingLine>
    </Card>
    {snapshot.desktopChannel === "stable" ? <div className="mt-5 flex items-center gap-3"><Button variant="outline" onClick={() => actions.openLegacyUi()}>Open legacy desktop</Button><p className="text-xs text-muted-foreground">Windows Share Target uses the retained WinUI compatibility host. Your pairing and transfers remain with the same daemon.</p></div> :
      <p className="mt-5 max-w-xl text-xs leading-6 text-muted-foreground">This preview installs separately from Uno/WinUI. It does not replace the Windows share target, Scoop package, Explorer integration, or existing background process.</p>}
    <div className="mt-7 flex items-center gap-1.5 text-[12px] text-muted-foreground"><CircleHelp size={14}/> Report regressions through the existing Conduit repository.</div>
  </div>
}

export function PageContents({ page, snapshot, search, actions }: { page: Page; snapshot: Snapshot; search: string; actions: PageActions }) {
  switch (page) {
    case "overview": return <Overview snapshot={snapshot} actions={actions}/>
    case "notifications": return <Notifications snapshot={snapshot} search={search} actions={actions}/>
    case "links": return <Links snapshot={snapshot} search={search} actions={actions}/>
    case "transfers": return <Transfers snapshot={snapshot} search={search} actions={actions}/>
    case "settings": return <Settings key={JSON.stringify(snapshot.settings)} snapshot={snapshot} actions={actions}/>
    case "about": return <About snapshot={snapshot} actions={actions}/>
  }
}
