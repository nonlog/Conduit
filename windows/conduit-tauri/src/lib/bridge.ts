import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow"
import { open } from "@tauri-apps/plugin-dialog"
import type { DesktopSettings, Snapshot } from "@/lib/model"
import { isNative, preview } from "@/lib/model"

function requireNative() {
  if (!isNative()) throw new Error("Preview mode: commands require the installed Tauri desktop application.")
}

export async function fetchSnapshot(): Promise<Snapshot> {
  if (!isNative()) return preview
  return invoke<Snapshot>("conduit_snapshot")
}

export async function ensureDaemon() {
  requireNative()
  return invoke<void>("ensure_daemon")
}

export async function runAction(action: string, transferId?: string) {
  requireNative()
  return invoke<string>("daemon_action", { action, transferId })
}

export async function updateSettings(settings: DesktopSettings) {
  requireNative()
  return invoke<void>("save_settings", { settings })
}

export async function removeHistory(kind: "notifications" | "links") {
  requireNative()
  return invoke<void>("clear_history", { kind })
}

export async function selectFileAndSend() {
  requireNative()
  const path = await open({ multiple: false, directory: false, title: "Send a file to your phone" })
  if (typeof path !== "string" || !path) return false
  await invoke<void>("send_file", { path })
  return true
}

export async function sendDroppedFile(path: string) {
  requireNative()
  await invoke<void>("send_file", { path })
}

export async function openLink(url: string) {
  requireNative()
  await invoke<void>("open_link", { url })
}

export async function openFolder(kind: "data" | "received") {
  requireNative()
  await invoke<void>("open_folder", { kind })
}

export async function openLegacyUi() {
  requireNative()
  await invoke<void>("open_legacy_ui")
}

export async function watchShellErrors(onError: (message: string) => void): Promise<UnlistenFn> {
  if (!isNative()) return () => {}
  const drain = async () => {
    const pending = await invoke<string[]>("take_launch_notices")
    for (const message of pending) onError(message)
  }
  // Subscribe before draining cold-start messages. Forwarded launches queue
  // their notice in the native process even if the WebView is still loading.
  const unlisten = await listen("conduit:launch-notice", () => { void drain().catch(console.error) })
  try { await drain() }
  catch (error) { unlisten(); throw error }
  return unlisten
}

export async function watchDaemon(onChanged: () => void, onDropped: (paths: string[]) => void): Promise<UnlistenFn> {
  if (!isNative()) return () => {}
  let throttle: ReturnType<typeof setTimeout> | undefined
  const unlisten = await listen("conduit:data-changed", () => {
    if (throttle) clearTimeout(throttle)
    throttle = setTimeout(onChanged, 150)
  })
  const unlistenDrop = await getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === "drop") {
      onDropped(event.payload.paths)
    }
  })
  return () => {
    unlisten()
    unlistenDrop()
    if (throttle) clearTimeout(throttle)
  }
}
