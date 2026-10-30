# Conduit / Rune native desktop preview

This is an actual Rust-native desktop UI built on [anomalyco/rune](https://github.com/anomalyco/rune),
not React in a WebView and not a screenshot or static mockup.

## Status and safety boundary

**Independent preview; not the default Conduit desktop.** Rune currently has no
stable release, so the dependency is pinned to a verified upstream Git commit.
The existing Scoop installation, Tauri Nova interface, Windows Share Target
host (`Conduit.exe`), original daemon, encryption and pairing stay intact.
Do not replace `conduit-nova.exe` with the preview binary.

The preview works with an already installed Conduit daemon (including custom
Scoop roots resolved through the `SCOOP` environment, `PATH`, startup registry
entry or installed application location). It does not create an independent
pairing identity, and **must not run a second resident daemon alongside
the stable installation**. If there is no installed daemon, it reports an error.

## Implemented native UI

- Overview and live LAN/Relay connection state
- Pair, cancel pairing and confirm unpair
- File transfer history, cancel active transfers, native file picker and
  Explorer drag-and-drop send
- Notifications and shared links, with two-step history clearing
- Relay endpoints, proxy mode / SOCKS5, receive folder, tray configuration
- Windows autostart and Explorer integration switches
- Open received files, existing data folder, legacy WinUI rollback
- Native system dark/light colors, responsive scrollable content, keyboard
  navigation and one native top-level Rune window
- Event-driven watch of bounded daemon state files (no background UI polling)

The native UI does not duplicate the daemon's Noise/LAN/Relay transport or
touch `identity.bin`. Real-device validation, Windows notification icon parity,
advanced transfer presentation and a stable packaging/integration plan remain
gates before it can replace Nova.

## Testing

Run the **Rune desktop preview** GitHub Actions workflow on its PR; the Windows
artifact is a ZIP with `conduit-rune-preview.exe`. Launch the preview while your
existing Conduit Scoop daemon is running. The formal Rust tests, compilation and
Windows PE GUI subsystem check run **only in GitHub Actions**.

For local lightweight formatting only:

```powershell
cargo fmt --manifest-path windows/conduit-rune/Cargo.toml --all -- --check
```

The original shipped desktop executable remains the production rollback until
native UI functional parity and a successful long-run test are established.

The flashing command window on stable Nova startup is fixed independently:
its daemon liveness check no longer shells out to Windows `tasklist.exe`.
