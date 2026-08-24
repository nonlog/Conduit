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

## Toolchain (resolved 2026-08-24)

| thing | version | via |
|---|---|---|
| Rust | 1.98.0, `stable-x86_64-pc-windows-msvc` | `scoop install rustup` |
| MSVC + Windows SDK | 14.44.35207 / 10.0.26100.0 | `scoop install portable-build-tools` |
| protoc | 36.0 (Maven artifact `4.36.0`) | `scoop install protoc` |
| JDK | 21.0.11 Temurin | already present |
| Android SDK | `D:\Android\Sdk`, platforms 34/36/36.1/37.0 | already present, no `cmdline-tools` |
| Gradle | wrapper pinned **8.14.3** | generated in a temp dir, see below |
| AGP / Kotlin | 8.13.2 / 2.4.10 | |

Two traps worth writing down:

- Git ships `usr/bin/link.exe`, which shadows MSVC's linker and breaks `cargo build` in a way
  whose error message names neither. Fixed by persisting an absolute
  `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` rather than fighting `PATH` order.
- The machine's Gradle is 9.7.1, and **AGP 8.x cannot run on Gradle ≥ 9.6** — it uses
  `InternalProblems`, removed there. So `gradle wrapper` cannot be run inside `android/`: it
  configures the project, applies AGP and dies. Generate the wrapper in an empty directory
  (Gradle 9 needs a `settings.gradle.kts` present, even an empty build) and copy the four
  files in. After that everything goes through `./gradlew` at 8.14.3 and the system Gradle is
  never used again.

## Additions after the first build (your calls)

1. **UI design language references Sefirah.** Android is Compose Material 3 with dynamic
   colour, which is what Sefirah's phone app uses; Windows is Fluent. Design language only —
   none of their XAML or Kotlin is read into our tree, because they are GPL and this stays
   relicensable. The Windows UI remains a **separate, non-resident process**; the daemon has
   no UI framework linked in at all.
2. **A new camera photo also raises a Windows toast.** This is a fourth trigger on the
   notification path, not a fourth feature. `ContentObserver` on
   `MediaStore.Images.Media.EXTERNAL_CONTENT_URI` — event-driven, no polling — filtered to
   `RELATIVE_PATH` under `DCIM/Camera` and `DATE_ADDED` after service start, so downloads,
   screenshots and app caches do not fire it. The photo goes into the toast as a hero image
   and **not** into the Windows clipboard: hijacking the clipboard on every shutter press
   would be wrong.
3. **Noise XX is implemented on BouncyCastle, not taken from a library.** No Noise
   implementation is published to Maven Central for Java or Kotlin — `com.southernstorm`
   and `com.github.rweather` are both 404, and the only hit is a webjar of the JS package.
   The alternatives were JitPack-building a library last touched in 2016, or vendoring ~3000
   lines of it for the one pattern we use. Instead: X25519 + ChaCha20-Poly1305 + BLAKE2s from
   `bcprov-jdk18on`, and the XX state machine written here, since the suite was already
   specified as hardcoded on both sides. `minSdk 29` also predates platform XDH, so the
   platform could not have supplied X25519 anyway. **The check that makes this safe is a
   cross-language interop test against Rust `snow`** — a hand-rolled handshake that agrees
   with a reference implementation on every byte is verified; one that only agrees with
   itself is not.

## Corrections found while implementing

- **`MAX_FRAME` is 65535, not 1 MiB.** A Noise transport message cannot exceed 65535 bytes,
  so the synthesis' 1 MiB frame was never reachable. One frame carries one Envelope, and the
  usable plaintext is 65519 bytes after the ChaChaPoly tag. Consequences: the 64 KiB image
  chunk in `proto/conduit.proto` overflows once protobuf framing is added and must drop to
  32 KiB in M1; and text over ~65 KB needs the same chunking, which M0 logs and skips rather
  than truncating.
- **No ACK/retry in M0.** The synthesis specified ACK within 5 s with 2 retries. On a live
  TCP connection that is nearly redundant, and it costs a timer per in-flight message —
  precisely the growth this project exists to avoid. `Envelope.ack_for` stays in the wire
  format, unused. Liveness is the 60 s `PING` / 10 s `PONG` deadline plus OS keepalive.
- **`message_id` is a per-session counter, not random.** Dedup state is reset when a session
  starts, which is simpler than an LRU keyed by peer and cannot collide across reconnects.
- LAN port is **TCP 41112**, mDNS service type `_conduit._tcp`. Nothing had specified one.
- **The Noise interop pin is `fixtures/noise_xx.txt`.** Generated by `wire::tests::noise_xx_fixture_matches_the_reference` from `snow` with fixed
  statics and fixed ephemerals, replayed in both roles by `NoiseInteropTest` on the JVM.
  It also pins `device_id` and `fingerprint`, because a mismatch there makes pairing
  unverifiable in exactly the same silent way. Regenerate by deleting the file and running
  `cargo test`; never edit it by hand. Both tests fail if either implementation drifts.
- **A camera photo reuses the image path**, with `ClipImageHeader.photo = true` meaning
  "toast this, do not touch the clipboard". Cheaper than a fourth message family, and the
  chunking already exists.
- Gradle stays at **8.14.3**: 8.14.4+ is not fetchable from this network (the
  `services.gradle.org` redirect target times out) and AGP 8.x cannot use ≥ 9.6 anyway. KGP's
  deprecation warning is suppressed in `gradle.properties` so bumping Kotlin to 2.5 fails
  loudly instead of being lost in eight lines of warning on every build.
- **protobuf-gradle-plugin 0.10.0 needs two workarounds under AGP + Kotlin DSL**: its `proto {}`
  accessor is declared on the plain Gradle `SourceSet`, so an Android source set needs
  `(this as ExtensionAware).extensions.getByName("proto")`; and an Android generate task starts
  with no builtin, so it is `maybeCreate("java").option("lite")`, not `getByName("java")`.

