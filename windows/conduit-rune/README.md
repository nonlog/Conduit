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

## Multiple computers and migrated identities

One Android phone can trust several independent desktop Noise identities and maintain
simultaneous authenticated LAN/Relay sessions. On the phone, **Settings → Devices**
lists connection status for each PC, lets you select an explicit file/Direct Share
destination, and forgets only that PC. On Rune's **Pairing** page, click
**Start pairing** to show a locally generated QR code and the equivalent
six-digit code. Both expire after two minutes; use the Android **Scan QR**
action, or enter the digits manually. Existing pairings are never replaced
by adding another PC.

**Important when setting up a second PC:** do not copy the first PC's
`identity.bin` to it. Copying the entire Scoop `persist/conduit/data`
directory clones the private Noise identity, making both PCs appear as the
**same computer** and preventing real multi-device pairing. If a copy has
already happened, run `conduit-daemon.exe pair status` separately on both
PCs and compare their public `desktop_id` values. If they match, **only
on the second/work computer**:

1. Exit the Conduit daemon on the copied computer. Make sure no transfers
   are in progress.
2. Run `conduit-daemon.exe pair rekey-imported --confirm` from its installed
   Scoop directory. This requires the daemon to be stopped, creates a
   timestamped local backup of the old identity and pairing files, generates
   a unique device key, and removes that copied installation's old pairing.
   Your transfer history and configuration remain intact.
3. Restart the work computer's Conduit and pair again by QR or code on the
   Android app. **Do not run this repair on the original personal PC.**

The backup contains sensitive private key material; keep it only locally
under the original Windows account, never upload it, and delete it only
after confirming the new connection works. This repair is never invoked
silently during an update.

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
