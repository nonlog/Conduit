# Conduit Desktop — Nova (stable and isolated preview)

Tauri 2 + React 19 + TypeScript + Vite + Tailwind v4 + official shadcn/ui Nova.
Lucide icons, Motion transitions, react-resizable-panels, dnd-kit sortable navigation and Zustand presentation state.

## Stable Windows desktop

The stable v0.1.11 Scoop build runs the Tauri Nova UI (`conduit-nova.exe`) by default. The existing `Conduit.exe` stays installed as the Windows Share Target compatibility host and a WinUI rollback option; its normal startup delegates to Nova, while `Conduit.exe --legacy-ui` explicitly opens the old control UI. The daemon, pairing, Noise encryption, LAN/Relay, clipboard, notifications and transfer queues are unchanged. Explorer sends continue to use the native `conduit-send.exe` helper.

Windows Share Target is a sparse MSIX identity registered against **`Conduit.exe`**. The legacy WinUI `ShareOperation` handler processes Windows share-sheet file requests in the stable package. Do not rename/remove that executable or the WinUI runtime dependencies simply because the default desktop is now Nova. The preserved host is an intentional compatibility boundary until a native WinRT alternative reaches functional parity.

To return to the legacy UI without touching device identity, history, services or installation, open **About → Open legacy desktop**, or execute `Conduit.exe --legacy-ui`. A full Scoop rollback should use an older known-good package and re-register the matching Share Target identity; do not delete the persisted `data` directory or reuse a newer sparse MSIX registration against an older XAML runtime.

## Separate Preview build

The isolated NSIS `Conduit Preview` variant uses a different Windows identifier and installer. It does not replace the Scoop version or register itself as a Windows Share Target. You can safely compare both desktop UIs while the same installed daemon remains resident.

The stable release ZIP can also be extracted as a portable bundle: Nova then uses its *sibling* daemon and the bundle's `data/` folder, even if Scoop is installed elsewhere. Scoop installations instead use the existing persisted `persist/conduit/data` directory; the standalone Preview uses `%LOCALAPPDATA%\\Conduit`. Never automatically cross these identities or silently start a second daemon over an already-active Windows session.

The Tauri backend reads the daemon's existing bounded state/history snapshots, watches specific files for updates and invokes only allowlisted daemon CLI operations. It never reads or writes identity.bin or peer.txt, does not implement its own encryption, and stores only theme/sidebar preferences in the WebView.

## Build and test

GitHub Actions is the only supported formal build path. `.github/workflows/build.yml` produces the stable `conduit-nova.exe` using `tauri.production.conf.json`, then packages it alongside WinUI and the Share Target into the usual Scoop-compatible Windows ZIP. A package regression verifies the Nova binary, fallback host, original daemon, signed sparse MSIX and absence of bundled identity/history. `.github/workflows/tauri-preview.yml` independently builds and tests the isolated Preview NSIS.

Local lightweight inspection (not a replacement for CI):
- bun install
- bun run check
- bun run lint
- bun test
- bun run dev — browser design preview with clearly marked sample data. Actions cannot access user data in this mode.

A Tauri development window requires native Windows Rust/MSVC and WebView2 prerequisites. Keep the existing daemon installed to use its production pairing identity while trying the preview.

## Current feature coverage

- Overview/connected device, active transfers summary, notifications with filtering and clearing, shared links, full transfer history/cancellation, settings and pairing controls
- Native open-file dialog and Windows file drop, daemon send and transfer progress
- Existing relay, proxy, tray, receive folder, Windows autostart/Explorer controls through the daemon
- Native light/dark/system theme, navigation reorder, sidebar resize and responsive layout
- Single-instance Windows desktop: opening Nova again focuses the existing window rather than starting competing UI instances
- Settings writes preserve unrelated configuration lines and atomically replace config.txt before requesting daemon reload
- Standalone NSIS installs bundle `conduit-daemon.exe`, the Explorer send helper and themed icons. Without Scoop, the daemon persists state in the user's existing `%LOCALAPPDATA%\\Conduit` directory, never under the replaceable installation resources directory; old bundled-preview state is salvaged without overwriting existing identity
- The bundled daemon tray opens the Nova window; Explorer helper failures are delivered to the existing Nova instance's Transfers page, with bounded native-only IPC

In a released Scoop installation, the Windows Share Target remains handled by the WinUI compatibility host and the Explorer verb is served by the installed daemon helper. The standalone Preview bundles its own Explorer helper and does not create an extra share target. In either mode, the Rust daemon remains the single authoritative transport process.

Do not uninstall the released version or relocate/overwrite its persisted data just to test the preview.
