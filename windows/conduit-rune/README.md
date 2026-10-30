# Conduit / Rune native desktop

This is an actual Rust-native desktop UI built on [anomalyco/rune](https://github.com/anomalyco/rune),
not React in a WebView and not a screenshot or static mockup.

## Architecture and release safeguards

Rune currently has no stable release, so the dependency is pinned to a tested
upstream Git commit. The stable Windows artifact packages this native UI as
`conduit-rune.exe`, selected by the installer, daemon tray and legacy shortcut
delegation. The existing Tauri Nova desktop (`conduit-nova.exe`) remains an
explicit rollback; the WinUI `Conduit.exe` keeps the Windows Share Target sparse
MSIX identity and remains available through `--legacy-ui`.

The UI works with an already installed Conduit daemon (including custom
Scoop roots resolved through the `SCOOP` environment, `PATH`, startup registry
entry or installed application location). It does not create an independent
pairing identity; the daemon's existing LAN port is also its single-instance
gate. Portable builds with a sibling daemon use their own `data/` directory even
if a separate Scoop installation exists.

## Implemented native UI

- Overview with bounded phone wallpaper and live LAN/Relay connection state
- Pair, cancel pairing and confirm unpair
- File transfer history, cancel active transfers, native file picker and
  Explorer drag-and-drop send
- Notifications and shared links, with two-step history clearing
- Searchable transfer history, notifications and shared links
- Relay endpoints, proxy mode / SOCKS5, receive folder, tray configuration
- Native receive-folder picker and independent persistent System/Light/Dark theme
- Windows autostart and Explorer integration switches
- About page, native app metadata, open received files and data directory,
  direct Nova fallback and legacy WinUI rollback
- Native system dark/light colors, responsive scrollable content, keyboard
  navigation and one native top-level Rune window
- Event-driven watch of bounded daemon state files (no background UI polling)
- Single-instance activation, Explorer send-error forwarding and a native
  Windows GUI executable (never opens a command-line window)

The native UI does not duplicate the daemon's Noise/LAN/Relay transport or
touch `identity.bin`. Nova remains installed as a fallback because Rune is
pre-release. The daemon protocol, persisted pairing, transfer history and
Share Target remain unchanged. Notification app-icon parity, advanced transfer
presentation and real-device long-run checks remain quality improvements.

## Testing

The **Rune desktop preview** workflow produces an isolated ZIP with
`conduit-rune-preview.exe`. The main **Build** workflow also compiles Rune and
bundles it as the primary `conduit-rune.exe` alongside Nova, WinUI, daemon and
Explorer helper. GUI-subsystem, Rust native tests, packaging, Share Target and
installer rollback checks run in GitHub Actions.

For local lightweight formatting only:

```powershell
cargo fmt --manifest-path windows/conduit-rune/Cargo.toml --all -- --check
```

To restore the previous desktop without downgrading or touching pairing data,
launch `conduit-nova.exe` directly from the installed Conduit directory.
`Conduit.exe --legacy-ui` opens the original Windows Share Target host as a UI.

The original command window flash is also fixed in Nova's retained daemon
liveness check: it no longer shells out to Windows `tasklist.exe`.
