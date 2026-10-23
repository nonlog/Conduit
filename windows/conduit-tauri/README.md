# Conduit Desktop — Nova preview

Tauri 2 + React 19 + TypeScript + Vite + Tailwind v4 + official shadcn/ui Nova.
Lucide icons, Motion transitions, react-resizable-panels, dnd-kit sortable navigation and Zustand presentation state.

## Coexists with the released WinUI application

This is an independent preview-only desktop application, not a replacement for the Scoop-installed Conduit/WinUI or Explorer share target. It uses a different Windows app identifier and installer. It reuses the existing Rust daemon for authenticated connections, Noise, clipboard, notifications, relay routing and file transfers.

The Tauri backend reads the daemon's existing bounded state/history snapshots, watches specific files for updates and invokes only allowlisted daemon CLI operations. It never reads or writes identity.bin or peer.txt, does not implement its own encryption, and stores only theme/sidebar preferences in the WebView.

## Build and test

GitHub Actions is the only supported formal build path. Dispatch Tauri Desktop Preview or open a PR touching windows/conduit-tauri. The workflow checks TypeScript, lint, frontend unit tests, then builds and tests the Rust bridge, bundles the already-built matching-version daemon and uploads an NSIS preview installer. No release tag or Scoop manifest is changed.

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
- Single-instance Windows preview: opening the app again focuses the existing window rather than starting competing UI instances
- Settings writes preserve unrelated configuration lines and atomically replace config.txt before requesting daemon reload

Not yet swapped into production: Windows share-target and context menu are still served by the existing native daemon/WinUI package. The new preview does not register itself as the Windows share target or replace the Scoop installation.

Do not uninstall the released version or relocate/overwrite its persisted data to test the preview.
