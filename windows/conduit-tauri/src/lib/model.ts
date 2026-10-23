export type Page = "overview" | "notifications" | "links" | "transfers" | "settings" | "about";

export interface Connection {
  state: string;
  route: string;
  relay: string;
  deviceName: string;
  linked: boolean;
}

export interface Pairing {
  deviceId: string;
  deviceName: string;
  code: string;
  expiresMs: number;
  active: boolean;
}

export interface DesktopSettings {
  relays: string[];
  proxyMode: "system" | "manual" | "direct";
  manualProxy: string;
  receiveDir: string;
  trayIcon: boolean;
  autostart: boolean;
  explorerIntegration: boolean;
}

export interface NotificationEntry {
  id: string;
  app: string;
  title: string;
  body: string;
  timestamp: number;
  icon: string | null;
}

export interface SharedLinkEntry {
  id: string;
  url: string;
  title: string;
  source: string;
  timestamp: number;
}

export interface TransferEntry {
  id: string;
  name: string;
  direction: "inbound" | "outbound";
  bytes: number;
  total: number;
  state: string;
  detail: string;
  percentage: number;
  terminal: boolean;
}

export interface Snapshot {
  version: string;
  connection: Connection;
  pairing: Pairing;
  settings: DesktopSettings;
  notifications: NotificationEntry[];
  sharedLinks: SharedLinkEntry[];
  transfers: TransferEntry[];
  wallpaper: string | null;
  daemonAvailable: boolean;
  dataDir: string;
}

export const isNative = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const now = Date.now();
export const preview: Snapshot = {
  version: "0.1.10-preview",
  dataDir: "Demo content — preview mode",
  daemonAvailable: false,
  wallpaper: null,
  connection: { state: "Connected", route: "LAN", relay: "", linked: true, deviceName: "Preview phone" },
  pairing: { deviceId: "demo-only", deviceName: "Preview phone", code: "", expiresMs: 0, active: false },
  settings: { relays: ["conduit-us.example.net:41113", "conduit-wa.example.net:41113"], proxyMode: "system", manualProxy: "", receiveDir: "Downloads", trayIcon: true, autostart: true, explorerIntegration: true },
  notifications: [
    { id: "mock-1", app: "Messages", title: "Project update", body: "Latest build is ready for review.", timestamp: now - 3 * 60_000, icon: null },
    { id: "mock-2", app: "Calendar", title: "Team sync", body: "Meeting starts in 30 minutes", timestamp: now - 45 * 60_000, icon: null },
    { id: "mock-3", app: "Mail", title: "Release notes", body: "The new desktop preview is available.", timestamp: now - 100 * 60_000, icon: null },
  ],
  sharedLinks: [
    { id: "mock-link-1", title: "Conduit on GitHub", url: "https://github.com/nonlog/Conduit", source: "Preview phone", timestamp: now - 2 * 60_000 },
  ],
  transfers: [
    { id: "aabbcc01", name: "design-specification.pdf", direction: "inbound", bytes: 1_950_000, total: 1_950_000, state: "completed", detail: "", terminal: true, percentage: 100 },
    { id: "aabbcc02", name: "project-screenshots.zip", direction: "outbound", bytes: 840_000, total: 840_000, state: "completed", detail: "", terminal: true, percentage: 100 },
  ],
};

export const activeTransfers = (snapshot: Snapshot) => snapshot.transfers.filter((transfer) => !transfer.terminal);

export const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return bytes + " B";
  const units = ["KB", "MB", "GB", "TB"];
  let scaled = bytes;
  for (let index = 0; index < units.length; index++) {
    scaled /= 1024;
    if (scaled < 1024 || index === units.length - 1) {
      return (scaled >= 10 ? scaled.toFixed(1) : scaled.toFixed(2)) + " " + units[index];
    }
  }
  return "—";
};

export function relativeTime(timestamp: number) {
  const age = Math.max(0, Date.now() - timestamp);
  if (age < 60_000) return "Just now";
  if (age < 3_600_000) return Math.floor(age / 60_000) + "m ago";
  if (age < 86_400_000) return Math.floor(age / 3_600_000) + "h ago";
  return new Date(timestamp).toLocaleDateString();
}
