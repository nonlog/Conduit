# conduit — decisions

Supersedes `research-synthesis.md` wherever they conflict. The synthesis stays because its
API-level detail is verified and expensive to re-derive (hard caps, Noise pinning, toast
AUMID mechanics, `CF_DIBV5` masks, the 7 risks). Everything below is a correction to it.

## Names and paths

| thing | value |
|---|---|
| repo root | `D:\Workspace\conduit` |
| Android appId | `com.conduit.sync` (changeable until first install; it's the LSPosed hook's match key) |
| Windows daemon | `conduit-daemon.exe` |
| Windows AUMID | `Conduit.Daemon` (constant forever — see synthesis §6) |
| crates | `conduit-daemon`, `conduit-relay`, `conduit-settings` |
| proto | `proto/conduit.proto` |
| settings | `%LOCALAPPDATA%\Conduit\conduit.toml` |

## Reversals of the synthesis (your calls)

1. **Relay ships in v1**, not "deferred indefinitely". Own minimal relay, not frp. Sequenced
   after LAN works (M2), because LAN is the thing that must be provably leak-free first.
2. **Settings are TOML**, not `settings.json`.
3. **Clipboard read uses an LSPosed hook, and the shell-UID `app_process` worker is dropped
   entirely.** The synthesis planned `FakeContext` + `WorkerService` + reflection into
   `ClipboardManager.mContext` + `UidObserver`/`linkToDeath` — a whole second always-on
   process, and synthesis risk #7 is that its hidden-API reflection breaks on API 34-36.
   The hook does the same job in ~30 lines with no extra process:

   ```
   hook com.android.server.clipboard.ClipboardService#clipboardAccessAllowed
     if (callingPackage == "com.conduit.sync") result = true
   ```

   Match the method by name, not by signature — the parameter list churns across releases.
   Nothing crosses a process boundary: system_server stops refusing, and the app's own
   `addPrimaryClipChangedListener` + `getPrimaryClip()` then work from background like any
   foreground app. One APK is both the app and the Xposed module.

   This deletes from the plan: `worker/` module, `FakeContext.java`, `WorkerService.java`,
   `WorkerStarter.kt`, `WorkerManager.kt`, `ShizukuHelper.kt`, the NDK dependency, setup
   step 3, and synthesis risk #7.
4. **AccessibilityService clipboard fallback is committed, not deferred indefinitely** — it
   lands as M3, for the non-rooted phone. Until then it is dead code, so it is not in v1.

## From the Phone Link teardown

See `../docs/` memory notes; the two findings that change design:

1. **No ICE / STUN / TURN, ever.** Phone Link's 13.5 MB `liblibnanoapi.so` runs a full ICE
   agent + TURN + URCP-over-UDP, and that machinery *is* the `libbasix / pDCT / udp(asio) /
   ICE Agent` leak this project exists to escape. Its stability and its leak are one design.
   Our relay is a dumb byte forwarder: both ends dial **outbound TCP** to it, it pairs them
   by `device_id` and copies opaque Noise frames. Outbound TCP traverses CGNAT with zero
   traversal logic. Microsoft builds ICE to avoid paying TURN bandwidth on video at billions
   of sessions; we move a few KB of clipboard a day, so relay bandwidth is free.
2. **Own liveness detection; never delegate it to the tunnel.** The frp/Sefirah and
   `127.0.0.1:15556` instability was not TCP's fault — nobody was probing. Phone Link has
   `KeepAlive.Interval` + `NoTrafficTimeout` + `GracePeriodTimeout` + `AutoReconnectTimeout`
   as first-class config. Ours: `TCP_KEEPIDLE 30 / KEEPINTVL 10 / KEEPCNT 3` **and** app-level
   `PING` at 60s idle with a 10s `PONG` deadline. Already in synthesis §4 — this is
   independent confirmation, so do not trade it away for simplicity.
3. **Network change suspends the session; it does not destroy it.** Phone Link has
   `ICELinkContextSuspended` / `Unsuspended` / `ICELocalInterfaceAdded`. Our version: on
   `onLost` mark the session suspended and debounce; on `onAvailable` try to resume before
   dialing fresh. This is both the stability win and the leak rule — one session object whose
   state changes, never a new object per network event.
4. **One keepalive timer for the whole process**, not one per connection (their
   `UDPKeepAliveAggregatorAdapter`). Trivial for us since we cap at one peer, but it is the
   rule if that ever changes.

## Other corrections

- `targetSdk 36` (test device is OnePlus CPH2573, Android 16 / SDK 36). Synthesis said 35.
- `minSdk 29` unchanged.
- `READ_CLIPBOARD_IN_BACKGROUND` is not reachable by any grant — it is `signature|role`, and
  on this device only `/system/app` preloads hold it (`com.microsoftsdk.crossdeviceservicebroker`,
  `com.oplus.linker`, `com.heytap.accessory`). Do not spend time re-testing `pm grant` or
  `appops set READ_CLIPBOARD`. The hook is the only non-privileged way in.

## Phases

| | scope | exit criteria |
|---|---|---|
| **M0** | LAN, text clipboard both directions, Noise XX, mDNS burst, metrics endpoint | synthesis §9 verbatim: 48h run, fd/handle/thread delta 0, `created==closed` |
| **M1** | image clipboard (chunked 64 KiB) + notifications → toast with update/remove | no new threads vs M0 |
| **M2** | relay: outbound TCP from both ends, pair by `device_id`, forward opaque frames | survives 5G↔hotspot flap without a session leak |
| **M3** | AccessibilityService clipboard fallback for non-rooted devices | works with LSPosed absent |

## Toolchain gap

`cargo`/`rustc` are **not installed** on this machine. Needed before any Windows or relay
work. `scoop install rustup` then `rustup default stable-msvc`. Android side is unblocked:
JDK 21 (Temurin) present, SDK at `D:\Android\Sdk`, Gradle arrives via the wrapper.
