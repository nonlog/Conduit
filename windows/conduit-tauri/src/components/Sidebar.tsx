import { useMemo } from "react"
import { DndContext, KeyboardSensor, PointerSensor, closestCenter, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core"
import { SortableContext, arrayMove, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ArrowLeftRight, Bell, ChartNoAxesCombined, ChevronLeft, ChevronRight, CircleHelp, FileClock, GripVertical, Link2, MonitorSmartphone, Settings2, Send, Smartphone } from "lucide-react"
import { motion } from "motion/react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useUi } from "@/store/ui"
import { initialNav } from "@/store/ui"
import type { Page } from "@/lib/model"
import type { Snapshot } from "@/lib/model"
import { cn } from "@/lib/utils"

export const pageLabels: Record<Page, string> = {
  overview: "Overview",
  notifications: "Notifications",
  links: "Shared links",
  transfers: "File transfers",
  settings: "Settings",
  about: "About",
}

const icons = {
  overview: ChartNoAxesCombined,
  notifications: Bell,
  links: Link2,
  transfers: FileClock,
  settings: Settings2,
  about: CircleHelp,
}

function Item({ id, current, count, collapsed, onClick }: {
  id: Page; current: boolean; count: number; collapsed: boolean; onClick: () => void
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id })
  const Icon = icons[id]
  return (
    <div ref={setNodeRef} style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn("group relative", isDragging && "z-50 opacity-75")}>
      {current && <motion.div layoutId="sidebar-active" className="absolute inset-0 rounded-lg bg-sidebar-accent" transition={{ type: "spring", stiffness: 430, damping: 37 }} />}
      <div className="relative flex items-center">
        <Tooltip>
          <TooltipTrigger render={
            <button aria-label={pageLabels[id]} aria-current={current ? "page" : undefined}
              className={cn("nav-item flex-1", current ? "text-sidebar-accent-foreground" : "text-muted-foreground")}
              onClick={onClick}>
              <Icon size={17} strokeWidth={1.9} aria-hidden="true"/>
              {!collapsed && <span className="truncate">{pageLabels[id]}</span>}
              {!collapsed && count > 0 && <Badge variant="secondary" className="ml-auto font-mono text-[10px]">{count}</Badge>}
            </button>
          }/>
          {collapsed && <TooltipContent side="right">{pageLabels[id]}</TooltipContent>}
        </Tooltip>
        {!collapsed && <button aria-label={"Move " + pageLabels[id]} title="Drag to reorder"
          className="nav-grip" {...attributes} {...listeners}>
          <GripVertical size={13} />
        </button>}
      </div>
    </div>
  )
}

export function Sidebar({ snapshot, onSend }: { snapshot: Snapshot; onSend: () => void }) {
  const page = useUi((s) => s.page)
  const setPage = useUi((s) => s.setPage)
  const collapsed = useUi((s) => s.collapsed)
  const setCollapsed = useUi((s) => s.setCollapsed)
  const navOrder = useUi((s) => s.navOrder)
  const setNavOrder = useUi((s) => s.setNavOrder)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }))
  const counts = useMemo<Record<Page, number>>(() => ({
    overview: 0,
    notifications: snapshot.notifications.length,
    links: snapshot.sharedLinks.length,
    transfers: snapshot.transfers.filter((t) => !t.terminal).length,
    settings: 0,
    about: 0,
  }), [snapshot])
  const sorted: Page[] = [...new Set(navOrder)].filter((id): id is Page => initialNav.includes(id))
  for (const id of initialNav) if (!sorted.includes(id)) sorted.push(id)

  function reorder(event: DragEndEvent) {
    const { active, over } = event
    if (over && active.id !== over.id) {
      const a = sorted.indexOf(active.id as Page)
      const b = sorted.indexOf(over.id as Page)
      if (a >= 0 && b >= 0) setNavOrder(arrayMove(sorted, a, b))
    }
  }

  return (
    <aside className={cn("sidebar-shell flex h-full flex-col gap-5 overflow-hidden px-3 py-4", collapsed && "items-center px-2")}>
      <div className={cn("flex w-full items-center gap-2 px-2", collapsed && "justify-center px-0")}>
        <div className="brand-mark flex size-8 shrink-0 items-center justify-center rounded-[11px]" aria-hidden="true">
          <ArrowLeftRight size={19} strokeWidth={2.6}/>
        </div>
        {!collapsed && <div className="min-w-0 flex-1"><div className="font-heading text-[15px] font-semibold tracking-[-0.035em]">Conduit</div>
          <div className="text-[10px] font-medium tracking-[0.16em] text-muted-foreground">DESKTOP</div></div>}
        <Button variant="ghost" size="icon-sm" onClick={() => setCollapsed(!collapsed)} aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}>
          {collapsed ? <ChevronRight size={15}/> : <ChevronLeft size={15}/>}
        </Button>
      </div>

      {!collapsed && <div className="rounded-xl border border-sidebar-border bg-background/75 p-3 shadow-[0_2px_8px_rgba(0,0,0,.025)]">
        <div className="mb-2 flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground">
          <MonitorSmartphone size={13}/> Connected device
        </div>
        <div className="flex items-center gap-2.5">
          <div className="flex size-9 items-center justify-center rounded-lg bg-muted"><Smartphone size={19}/></div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13px] font-semibold">{snapshot.connection.deviceName}</div>
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className={cn("status-dot", snapshot.connection.linked ? "status-dot-on" : "status-dot-off")}/>
              {snapshot.connection.state}
              {snapshot.connection.linked && <span className="truncate">· {snapshot.connection.route.toUpperCase() || "Connected"}</span>}
            </div>
          </div>
        </div>
      </div>}

      <div className="w-full flex-1 overflow-y-auto">
        {!collapsed && <div className="px-3 pb-2 text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">Workspace</div>}
        <nav aria-label="Workspace" className="flex flex-col gap-1">
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={reorder}>
            <SortableContext items={sorted} strategy={verticalListSortingStrategy}>
              {sorted.map((id) => <Item key={id} id={id} current={page === id} count={counts[id]}
                collapsed={collapsed} onClick={() => setPage(id)}/>)}
            </SortableContext>
          </DndContext>
        </nav>
      </div>

      <div className="w-full space-y-3">
        <Button onClick={onSend} disabled={!snapshot.connection.linked || !snapshot.daemonAvailable}
          className={cn("h-9 w-full", collapsed && "px-0")} title="Send a file">
          <Send size={15}/> {!collapsed && "Send a file"}
        </Button>
        {!collapsed && <div className="flex items-center justify-between px-2 text-[11px] text-muted-foreground">
          <span>Noise encrypted</span>
          <span>{snapshot.connection.linked ? (snapshot.connection.route || "—").toUpperCase() : "Offline"}</span>
        </div>}
      </div>
    </aside>
  )
}
