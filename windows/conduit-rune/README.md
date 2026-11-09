# Conduit / native Rust desktop

The Windows desktop is implemented in Rust with Rune. There is no React,
Tauri, JavaScript build, WebView frontend, or alternative-desktop launcher.
Rune remains pinned to the tested upstream Git revision in `Cargo.toml`.

## Runtime boundary

`conduit-rune.exe` is the only normal desktop entry point: Start menu, tray,
legacy shortcuts and Explorer send-error handoff all select it. The resident
Rust daemon continues to own Noise, LAN/Relay, clipboard and file transfers.
The on-demand UI watches bounded state files and does not poll in the background
or create a second identity or transport.

`conduit-share-target.exe` is retained as a **Windows Share Target host** with
its sparse MSIX identity. It is a small Rust process that receives Windows
`StorageItems` and forwards them through `conduit-send.exe`; it does not open a
second desktop UI or carry a WebView/.NET runtime.

Scoop's persisted data directory remains authoritative. Portable installations
with their own sibling daemon keep their own data directory. The retired standalone
preview layout is recognized only for preserving/migrating old data, never to
launch the retired frontend. Installers require Rune before changing a running
installation and remove only the exact obsolete frontend executable names after
verifying the new payload. They never delete pairing, identity or history files.

## Native interface

The overview has one page title, a compact device strip and recent transfer rows.
There are no dashboard statistics, repeated marketing headings or fallback-app
buttons. Navigation uses native vector icons and neutral dark/light surfaces.
Transfer rows show readable byte sizes and progress; cancellation remains
available while another send operation is waiting for a receipt.

Native file/folder pickers, drag-and-drop, searchable transfers/notifications/links,
QR pairing, system/light/dark appearance, relay/proxy settings, autostart and
Explorer integration are retained. The UI runs only while its window is open.

## Several computers and copied identities

The multi-desktop Android revision stores up to eight trusted desktop identities.
Settings > Devices shows per-PC connection state and an explicit file/share target;
forgetting a computer removes only that trust relationship. An old APK that has already
overwritten its single stored pairing cannot recover the previous ID automatically:
that computer needs to be paired again.

Rune's Pairing > Start pairing displays a locally generated QR and its six-digit
code for the temporary pairing window. Android Scan QR accepts the matching
`conduit://pair?code=XXXXXX` payload. Do not copy `identity.bin` between PCs:
copying the entire persisted data directory makes both installations the same
cryptographic computer.

For an already-copied installation, compare `conduit-daemon.exe pair status` on
both PCs. If the public desktop IDs match, stop the daemon on **only the copied
work PC**, ensure no transfers are running, and use:

```powershell
conduit-daemon.exe pair rekey-imported --confirm
```

The command backs up that copied installation's old identity and pairing before
creating a distinct identity. Restart and pair the work PC again. Never run this
repair on the original PC. The backup contains private key material: keep it
local and do not publish it. Updates never invoke this repair automatically.

## Verification and delivery

All formal builds and tests run on GitHub Actions, not a PC or VPS. The Rune
workflow runs native tests, verifies the PE GUI subsystem and renders 28 synthetic
acceptance images: seven pages/states, dark/light, and minimum/default window
sizes. The PNGs use Rune's actual renderer, not browser screenshots or mockups.
The Build workflow verifies native-only source contracts, full Windows packaging,
Share Target registration, stale-frontend cleanup and identity preservation on a
disposable runner. Preview binaries retain a distinct executable name solely for
isolated acceptance; the production package installs `conduit-rune.exe`.

Local lightweight checks only:

```powershell
cargo +1.98.0 fmt --manifest-path windows/conduit-rune/Cargo.toml --all -- --check
./scripts/test-native-only-desktop.ps1
./scripts/verify-version.ps1 -ExpectedVersion 0.1.15
```

Passing an earlier commit's CI does not validate a new revision. Review the new
native screenshots before replacing the installed desktop. Preserve the prior
verified package and data when preparing a rollback; there is no Nova fallback.
Real dual-PC acceptance and Android signing/data-preservation remain separate
release gates. Never uninstall the phone app merely to bypass a signer mismatch.
