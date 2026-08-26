# Conduit development guide

> **Current repository:** `D:\Workspace\Conduit`  
> **Primary targets:** Android API 29–36 and native Windows  
> **Read first:** [architecture.md](architecture.md) and [decisions.md](decisions.md)

This is the repeatable local workflow. Public-relay changes remain outward-facing operations;
the compatible role-aware migration was deployed on 2026-08-26, so future relay changes still
need explicit deployment intent and rollback discipline rather than being implied by a local test.

## Repository map

```text
android/                    Kotlin Android app and LSPosed module
windows/conduit-daemon/     Rust Windows background daemon
relay/                      Rust blind TCP rendezvous relay
proto/conduit.proto         Shared protobuf contract
fixtures/noise_xx.txt       Cross-language Noise XX transcript
docs/                       Architecture, decisions, progress, and backlog
```

`proto/conduit.proto` is the wire-format source of truth.  Generated outputs are build
artifacts: edit the `.proto`, never generated Java/Rust code.  The Noise transcript is
produced by the Rust `snow` reference test and replayed by the Android JVM test; do not edit
it by hand.

## Toolchain

The recorded versions and the reasons for the Android/Gradle pin are in
[decisions.md](decisions.md#toolchain-resolved-2026-08-24).  In short, a normal build needs:

| Tool | Required state |
| --- | --- |
| JDK | Java 21 |
| Android SDK | Installed and discoverable by Gradle; the working SDK is `D:\Android\Sdk` |
| Android build | Repository wrapper Gradle 8.14.3, AGP 8.13.2, Kotlin 2.4.10 |
| Rust | MSVC-targeted Rust toolchain compatible with workspace `rust-version = 1.98` |
| C/C++ toolchain | MSVC + Windows SDK; the established portable choice is Scoop `portable-build-tools` |
| Optional protocol utility | `protoc`; regular Gradle builds resolve their pinned Maven artifact themselves |
| Android device tooling | Android Platform Tools / `adb` |

### Install policy on this Windows machine

Use Scoop first, including configured third-party buckets.  Search before installing:

```powershell
scoop search rustup
scoop search portable-build-tools
scoop search protoc
```

Only then install a missing package, for example:

```powershell
scoop install rustup
scoop install portable-build-tools
scoop install protoc
```

Do **not** replace `portable-build-tools` with a large elevated Visual Studio Build Tools
installation unless Scoop has actually been ruled out.  The portable package supplies the
needed MSVC/SDK without the permanent installer and registry footprint.  If Cargo reports that
Git's `link.exe` shadowed the MSVC linker, use the persisted absolute
`CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` configuration described in the decisions record;
do not “fix” it by reshuffling `PATH` blindly.

Do not run the machine-wide Gradle inside `android/`.  The wrapper is intentionally pinned
because AGP 8.x is incompatible with recent Gradle 9.x releases.  Always invoke the wrapper.

## Build and test

Start at a clean, observed state:

```powershell
Set-Location D:\Workspace\Conduit
git status --short
```

### Android

```powershell
Set-Location D:\Workspace\Conduit\android
.\gradlew.bat assembleDebug testDebugUnitTest
```

Equivalent POSIX shell command:

```sh
cd android
./gradlew assembleDebug testDebugUnitTest
```

The debug APK is written to:

```text
android/app/build/outputs/apk/debug/app-debug.apk
```

The plain JVM tests cover the bounded history, file/image framing checks, notification payload
budget, the Noise reference transcript, and wire-session behaviour.  They are intentionally
not a substitute for a real Android notification-listener or clipboard-hook test.

### Windows daemon

```powershell
Set-Location D:\Workspace\Conduit
cargo test -p conduit-daemon
```

Some daemon toast tests are `#[ignore]` because they deliberately show a native toast and may
require a human click.  Run them only while at the desktop and treat them as interactive
platform checks, not CI:

```powershell
cargo test -p conduit-daemon -- --ignored
```

The photo-toast check expects a click to open Snipping Tool.  Do not run it unattended.

### Relay

```powershell
Set-Location D:\Workspace\Conduit
cargo test -p conduit-relay
```

The relay test result validates the source currently on disk; it never authorises a future
deployment by itself. Commit `86a2b86` is the currently deployed compatibility design: the relay
accepts legacy 47-byte clients and explicit-role 48-byte clients, while the installed Android and
Windows clients use explicit roles. See
[architecture.md](architecture.md#relay-preamble-deployed-contract-and-compatible-migration).

### Whole workspace formatting/checks

Use these before a Rust commit when the relevant source changed:

```powershell
Set-Location D:\Workspace\Conduit
cargo fmt --check
cargo test --workspace
```

Run Android and Rust checks independently after a protobuf contract change.  A successful
compile on just one side does not prove interoperation.

## Android device workflow

### Install a debug APK

Always inspect transports first.  Wireless ADB may disappear, and more than one device makes
an unqualified command ambiguous.

```powershell
adb devices -l
$serial = '<serial from adb devices -l>'
adb -s $serial install -r `
  D:\Workspace\Conduit\android\app\build\outputs\apk\debug\app-debug.apk
```

On Android versions that redact notification content from listeners, re-grant the permission
after **every reinstall**:

```powershell
adb -s $serial shell cmd appops set com.conduit.sync RECEIVE_SENSITIVE_NOTIFICATIONS allow
```

This AppOp only controls what Android exposes to `NotificationRelay`.  It is not the same as
the in-app *Hide notification content* switch, which is user-controlled and defaults off.

### Debug a direct socket

The foreground service must be started through the activity on Android 12+; a shell-started
foreground service can be rejected as a background start.  With USB forwarding available:

```powershell
adb -s $serial reverse tcp:41112 tcp:41112
adb -s $serial shell am start -n com.conduit.sync/.MainActivity `
  --es host 127.0.0.1 --ei port 41112
```

`MainActivity` forwards the host extra to `SyncService`.  Do not use this as evidence for mDNS
or relay routing: it intentionally bypasses both.

### Logs and app-private state

Logcat on the test phone rotates quickly.  Clear/capture/action/dump in one test round, or
write a temporary device-side log if a long observation is needed.

```powershell
adb -s $serial logcat -c
adb -s $serial logcat -v threadtime -s conduit.svc conduit.link conduit.notif conduit.photo
```

`run-as` can inspect the storage paths that matter without exposing their contents elsewhere:

```powershell
adb -s $serial shell run-as com.conduit.sync ls files
```

Expected persisted files include `identity.bin`, `peer-name.txt`, `settings.txt`, and
`history.json`.  On this test phone `SharedPreferences` silently fails to create/write its
directory, so do not migrate these stores back to preferences without reproducing and solving
that device behaviour.

## Manual verification checklist

Use real UI actions where platform ownership matters:

1. Compare the displayed Android and desktop Noise fingerprints during initial LAN pairing.
2. Copy text in both directions; include CRLF-originated Windows text to confirm echo
   suppression still normalises it.
3. Copy an image in both directions; exercise a `content://` source rather than only a local
   file URI.
4. Post, update, and dismiss a normal Android notification; verify one Windows toast updates
   in place and disappears.
5. Verify media, ongoing, group-summary, and silent notifications do not create desktop
   toasts.
6. Share a small file through the Android system share sheet; ensure it appears in the actual
   Windows Downloads folder, and that its toast opens the folder rather than the file.
7. Test camera-photo and screenshot capture independently when that path is in scope for the
   run. They use separate Android observers but share the bounded Windows capture-toast/Snipping
   Tool activation slot; neither is allowed to touch the clipboard.
8. For lifecycle work, record `opened/closed` on Android and `created/closed` plus Windows
   process resources before and after repeated real disconnect/reconnect cycles.

Do not infer a completed transfer or live session solely from a stale buffered daemon log.
Confirm current timestamps and filesystem state after the transfer has had time to finish.

## Git and change discipline

- Commits created by the coding agent use the official Codex identity at repository scope or per
  command; do not rewrite historical authorship:

  ```text
  user.name  = Codex
  user.email = codex@openai.com
  ```

- Do not push merely because local tests pass.  A push is outward-facing and needs explicit
  approval unless it was explicitly requested in the same context.
- The relay migration is deployed. Preserve legacy inference until old clients are deliberately
  retired; removing it is a later protocol change and should be coupled to M2 evidence.
- Keep commits narrow: protocol revisions must include both endpoint implementations, relay
  compatibility tests, and an explicit migration plan; a relay-only patch is incorrect.

## Updating the records

When implementation status changes, update:

- [architecture.md](architecture.md) for component/data-flow changes;
- [progress.md](progress.md) for dated evidence and caveats; and
- [backlog.md](backlog.md) when an item is completed, deferred, or newly discovered.

Do not turn a sample resource measurement or a one-off device success into milestone
completion.  M0 still requires its documented endurance evidence.
