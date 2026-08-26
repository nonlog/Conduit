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
- **A screenshot reuses the bounded image path but has explicit semantics.**
  `ClipImageHeader.screenshot = true` identifies it to new peers, while the Android sender also
  sets `photo = true` as a compatibility marker: an older desktop then still treats the unknown
  screenshot as a non-clipboard capture rather than overwriting the user's clipboard. A separate
  `Screenshots` `ContentObserver` accepts only new `Pictures/Screenshots/%` rows named
  `Screenshot_*`, deduped by MediaStore id; there is no poll or new worker. Camera photos and
  screenshots share one bounded Windows capture-toast/staged-file/shared-token slot. On the
  target CPH2573, a real system screenshot produced one `New screenshot` toast, the click opened
  that image in Snipping Tool, the Windows clipboard sequence stayed 979 → 979, and a scanner
  re-notification did not produce a duplicate toast.
- Gradle stays at **8.14.3**: 8.14.4+ is not fetchable from this network (the
  `services.gradle.org` redirect target times out) and AGP 8.x cannot use ≥ 9.6 anyway. KGP's
  deprecation warning is suppressed in `gradle.properties` so bumping Kotlin to 2.5 fails
  loudly instead of being lost in eight lines of warning on every build.
- **protobuf-gradle-plugin 0.10.0 needs two workarounds under AGP + Kotlin DSL**: its `proto {}`
  accessor is declared on the plain Gradle `SourceSet`, so an Android source set needs
  `(this as ExtensionAware).extensions.getByName("proto")`; and an Android generate task starts
  with no builtin, so it is `maybeCreate("java").option("lite")`, not `getByName("java")`.


## M0 transport decisions (Android)

- **Two threads per link, both blocked in a syscall when idle.** A reader thread sits in
  `recvfrom`; a single-thread executor sits on its queue. That split is what removes the
  locks: the reader is the only caller of `recv`, the sender the only caller of `send`, so
  the two Noise counters are never touched concurrently. The PONG a PING earns is *posted*
  to the sender rather than written by the reader, for the same reason. The rejected
  alternative was one thread with `SO_RCVTIMEO` set short enough to drain a queue — which
  is polling with extra steps.
- **`Link` outlives its connections.** The sender thread and the queue are created once per
  service; only the socket, the reader thread and the `WireSession` come and go. A network
  change calls `disconnect()`, not `close()`, so reconnect churn cannot grow the thread
  count. This is the "suspend, not destroy" rule, and it is the direct answer to the
  Phone Link failure this project replaces.
- **`Socket().use { }` is the `SessionGuard` equivalent.** Every exit — return, throw, or
  another thread closing the socket underneath the read — runs the same teardown, and the
  `opened`/`closed` counters are logged on each close so the 48 h criterion is greppable
  from `logcat`.
- **`soTimeout` is 150 s: a deadline, not a poll.** The kernel wakes nobody until it fires,
  and a healthy desktop's 60 s keepalive always beats it.
- **Reconnect is edge-triggered, with a spin guard.** Triggers are service start, a Wi-Fi or
  Ethernet network appearing, the user tapping Link, and *a session that was up being lost*.
  A dial that never completed its handshake deliberately does not re-dial, or a desktop that
  is advertising but refusing would produce a spin loop. Cost of a down desktop is one 8 s
  mDNS burst, not a retry timer.
  - **Known gap:** if the desktop restarts while the phone stays on the same Wi-Fi and the
    phone's session was already down, nothing re-dials until a network event or a tap. The
    fix is a browse that lives only while the screen is on — screen-on is itself an event,
    and the CPU is already up — not continuous discovery, which holds a multicast lock and
    wakes the app for every mDNS packet on the subnet.
- **Discovery bursts, and stops itself twice over**: on the first resolve, or on an 8 s
  deadline. One pending main-looper message is the whole timing cost, and only in flight.
  `resolveService`/`getHost` are deprecated at API 34 but kept because `minSdk` is 29 and one
  code path beats two.
- **Ping-pong prevention is one normalisation invariant, mirrored on both sides.** `lastText`
  always holds the LF form; it is written *before* the clipboard write, because the change
  listener can fire before `setPrimaryClip` returns. Without the LF normalisation the two
  sides trade the same text forever, since Windows hands back CRLF.
- **UI state is Compose snapshot state in one `object`**, written from the link's threads.
  A flow plus a repository layer for three fields would be scaffolding.
- **mDNS advertising uses `mdns-sd` with `enable_addr_auto()`.** The daemon tracks interface
  addresses itself, so a VPN or dock appearing does not require a re-register and therefore
  cannot disturb a live session. The native alternative, `DnsServiceRegister` from `dnsapi`,
  would avoid the dependency but needs a completion callback in unsafe FFI — the exact class
  of lifetime bug this project exists to avoid. The advert is registered *after* the socket
  binds, so a resolve is never answered by a connection refusal, and the TXT record carries
  `id`/`fp`/`v` so the phone can tell two desktops apart before handshaking.

## Clipboard access from the background

Stock Android 10+ makes the Android→Windows direction of feature 1 impossible, and not by
an app-ops gate: `ClipboardService.clipboardAccessAllowed` refuses a caller that is neither
focused nor the current IME *before* it ever consults app-ops, so `appops set
READ_CLIPBOARD allow` changes nothing. The same method also gates listener delivery, so
without a fix the app is not merely unable to read a clip — it is never told one happened.

Three ways out, in the order they were rejected:

1. `android.permission.READ_CLIPBOARD_IN_BACKGROUND` is AOSP's own escape hatch but is
   `signature|privileged`; using it means shipping the APK into `/system/priv-app` with a
   `privapp-permissions` overlay. Heavier and more fragile than the hook.
2. An `AccessibilityService` can observe clipboard-bearing events, but it is a broad
   capability granted for a narrow purpose. Kept as the M3 fallback for non-rooted phones.
3. **An LSPosed module in the same APK**, hooking `clipboardAccessAllowed` inside
   system_server. Chosen: the phone is rooted with KernelSU + LSPosed, and Play Store policy
   was already sacrificed.

The hook is narrowed twice on purpose. It forces the result only when this package's own
name appears among the arguments, so every other app stays subject to the normal check; and
it matches on the method *name*, never a signature, because that method gained a `userId` in
11, a `shouldNoteOp` in 12 and an `attributionTag` plus a `deviceId` in 13. `xposedscope` is
pinned to `android` alone. The Xposed API jar is `compileOnly` from `api.xposed.info` — the
only place it is published — and the APK was checked to carry nothing from it but type
references.

## What real hardware added to the foreground-service rules

Two platform requirements that no amount of reading the manifest reference produces, both
found by the service refusing to start on an OPPO CPH2573 (Android 16 / API 36):

- **Android 12+ refuses a foreground service started from the background**
  (`ForegroundServiceStartNotAllowedException: mAllowStartForeground false`). The tempting
  fix — `android:exported="true"` so `am start-foreground-service` can reach it — is a
  regression twice over: it opens the service to every app on the device and it does not
  even address the cause. The service is instead always started by `MainActivity`, which is
  a legitimate foreground caller and is also the real user path. The debug host override
  therefore rides in on the *activity's* launch intent and is forwarded to the service.
- **Android 14+ will not start a `connectedDevice` service on
  `FOREGROUND_SERVICE_CONNECTED_DEVICE` alone.** It demands a second permission proving a
  device is genuinely involved, from a list spanning the three `BLUETOOTH_*`,
  `CHANGE_NETWORK_STATE`, `CHANGE_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE`, `NFC`,
  `TRANSMIT_IR`, `UWB_RANGING`, `RANGING` and USB host/accessory. Of these
  `CHANGE_WIFI_MULTICAST_STATE` is the only honest one: mDNS discovery is multicast, so the
  permission describes something the app actually does.

`MainActivity` is `singleTop` with an `onNewIntent`. Without both, a second launch carrying
a different host silently does nothing — the existing instance is reused, `onCreate` never
runs again, and the new extras are dropped on the floor.

## M0 leak invariant, measured

Six connect/drop cycles driven by killing and restarting the desktop daemon, so the phone's
reader wakes on a real FIN rather than a timeout:

```
session 1 closed: opened=1 closed=1   ...   session 5 closed: opened=5 closed=5
threads=24  fds=142  VmRSS≈222 MB     unchanged across all six
```

The reader thread's tid differs every session (18519 → 19345) while the thread count stays
put, which is the point: each session's thread genuinely exits instead of accumulating. This
is the failure mode the project was started over — connect, disconnect, Java believes it
closed, the native session lives on — and `opened == closed` is the assertion that catches
it. Two measurements that looked like passes but were not, kept here because both are easy
to repeat:

- `adb reverse --remove` does **not** close an established socket. It stops new connections
  only, so the phone's reader sits on its 150 s deadline and the thread count naturally
  holds steady. Nothing was being torn down, so nothing was being tested.
- Re-launching the activity with a host extra did nothing before the `onNewIntent` fix
  above, so the "cycles" never re-dialled either.

Still outstanding for M0: the same counters over a 48 h run, and mDNS discovery exercised
for real, which needs the phone and the desktop on one subnet.

## Notification mirroring

The phone's shade becomes genuine `ToastNotification`s — Action Center entries that update
in place and withdraw when the phone's notification is dismissed — not balloons and not a
custom window.

**Android.** A `NotificationListenerService` cannot own the transport: the system binds and
rebinds it on its own schedule, so it borrows `SyncService`'s `Link` through one volatile
field and drops the notification when nothing is connected. Queueing instead would be
wrong — a notification the desktop missed is not worth showing minutes later. Ongoing
notifications (media transports, other apps' foreground services, progress bars) and group
summaries are filtered out: they are states rather than events, so mirroring them would
leave permanent toasts and duplicates. A repost of a key already seen becomes `NOTIF_UPDATE`
rather than a second `NOTIF_NEW`, which is what stops a chat thread popping once per
message. The key set is bounded at 256 because a removal that never arrives — listener
rebound, posting app killed — would otherwise leak it.

The send queue went from 16 to 64 now that notifications share it with clips. Discard-oldest
is still right, but a chat app catching up after a reconnect posts a burst no human
clipboard ever produces.

**Windows.** Two things make this less obvious than it looks, and both were settled by a
test against the real platform rather than by reading:

- `ToastNotifier` is COM apartment-bound, so it cannot be built on one tokio worker and
  used from another. One dedicated MTA thread owns it for the life of the process and takes
  commands over a channel — the same shape as the clipboard bridge, and for the same reason:
  notification traffic must not add a thread per message.
- An unpackaged Win32 process has no package identity, so `CreateToastNotifierWithId` needs
  an AppUserModelID Windows can resolve. A key under
  `HKCU\Software\Classes\AppUserModelId\Conduit.Desktop` with a `DisplayName` is enough;
  the documented alternative is a Start Menu shortcut carrying the ID as a shell property,
  which means `IShellLink` plumbing for the same result.

Toast tags are a digest of the Android key, not the key: Windows caps a tag at 64 characters
and keys like `0|com.tencent.mm|1234567|null|10123` are already close. Because the tag is
*derived* rather than remembered, update and removal need no per-notification state on the
desktop at all — the same key always hashes to the same tag.

Update uses `{title}`/`{body}` data binding with sequence number 0, not a re-`Show` with the
same tag. Re-showing would work, but it re-alerts: a typing indicator or a download
percentage would pop the toast again every time. Sequence 0 means "apply unconditionally",
which removes the per-toast counter a real sequence would need — the single sender thread
already guarantees order. Only the app name is inlined into the XML, so it is the only value
that needs escaping; title and body travel as bound data and never reach the parser.

`cargo test -p conduit-daemon -- --ignored` pops a real toast and asserts the whole cycle:
the tag reaches Action Center, survives an update, and is gone after a hide. The
load-bearing assertion is reading `title` back off the live toast — without it the test
would pass just as happily on a toast rendering the literal text `{title}`.

Deliberately not in this first cut: app and large icons, notification actions and inline
reply, and `MessagingStyle` history. The proto already carries all of them, so adding them
later does not change the wire.

## Relay: off-LAN reach without transport machinery

The relay is a byte splicer and nothing else. Both peers dial out to it, it pairs them by
rendezvous id, and from there it is `copy_bidirectional` over opaque ciphertext. It has no
protobuf dependency, no Noise dependency, and no key — if anything in its dependency tree
could decrypt a frame, the design would be wrong.

No ICE, STUN or TURN, ever. That machinery is precisely the transport lifecycle this project
exists to avoid: candidate gathering, per-candidate sockets and a session object whose
destruction is someone else's problem is the exact shape of the Phone Link leak. A splice
needs none of it, because the phone always dials out and NAT only has to be traversed in the
direction it already permits.

The original deployed preamble was the fixed 47 bytes — `CDT1` plus a 43-character base64url
id. That omitted the role because the phone is always the Noise initiator and the desktop is
always the responder. Real reconnect behaviour proved the omission wrong: a stale parked phone
could survive long enough for the same phone's next attempt to arrive, and the id-only relay
would splice the two initiators together. Each then saw the other's 32-byte Noise message 1
where an 80-byte responder message 2 was expected.

The migration format is therefore 48 bytes: `CDT1`, one role byte (`>` initiator / `<`
responder), then the same 43-byte rendezvous id. The role bytes are deliberately outside the
base64url alphabet. That gives the compatible relay a zero-ambiguity discriminator at byte five:
an explicit role means the new form; a base64url byte means the first character of a legacy id.
The waiting map is keyed by `(id, role)`, and a same-role arrival replaces the previous waiter
instead of ever being paired with it.

Compatibility is server-first. For a legacy 47-byte connection only, after reading the id the
relay performs a non-consuming `peek` for at most one second. The deployed phone immediately
writes Noise message 1 and is classified as initiator; the deployed desktop writes nothing until
a partner speaks and is classified as responder. This temporary timer is negotiation machinery,
not steady-state liveness. Explicit-role peers have no such wait. The test suite covers old↔old,
both mixed upgrade orders, explicit stale reconnect and legacy stale reconnect, and proves the
peek leaves the first Noise bytes untouched.

Steady-state staleness still needs no userspace probe. A waiter whose TCP genuinely died is
spliced to the next opposite-role arrival, the copy ends immediately, and the live peer sees EOF
and redials — one wasted round trip in place of a liveness protocol. Kernel `SO_KEEPALIVE` reaps
dead waiters and refreshes live NAT mappings. After old clients are retired, the one-second
legacy inference path can be deleted and the relay returns to having no userspace timer at all.

On the desktop, parking is one `peek`. Nothing arrives on a parked connection until it is
spliced, so a single blocked `peek` replaces a poll, and the bytes stay in the socket for the
handshake that follows. `Ok(0)` means the relay hung up, which is also what being spliced
onto a dead peer looks like from that side. The desktop re-parks the instant it hands a
stream over, so a reconnecting phone finds a partner already waiting instead of racing it.

Keepalive is per-path, because a ping that is free on Wi-Fi is a radio wake on cellular:
60 s on the LAN, 240 s over the relay, with the phone's read deadline following at 2.5x
(150 s and 600 s). Four radio wakes an hour instead of sixty. The cost is that a tunnel
dying without a FIN goes unnoticed for up to ten minutes, which is the right trade for a
clipboard.

Routing is one decision on the phone. Wi-Fi or Ethernet gets an mDNS burst, and the burst's
empty callback falls through to the relay — that is the foreign-Wi-Fi case. Cellular skips
mDNS entirely rather than running and waiting it out, because eight seconds of multicast on a
mobile network is eight seconds of radio for a guaranteed miss. The relay hostname is
resolved on the reader thread, the one thread allowed to block, never on the connectivity
callback that asked for the dial.

`registerDefaultNetworkCallback`, not a transport-filtered request. Filtering would have made
the single `networkUp` flag wrong the moment cellular was included: a Wi-Fi `onLost` while
cellular was up would have cleared it. On the default network a handover is exactly one
`onAvailable` for the network that replaced the old one.

Pairing must happen once on a LAN, because the rendezvous is the desktop's device id and
nothing knows it until one direct handshake has said so. It is then persisted to `peer.txt`
and length-checked on read, so a truncated file reads as "never paired" rather than producing
a rendezvous the relay refuses.

### Deployed

`tyo.414222.xyz:41113`, a 1 MB static musl binary under systemd with `DynamicUser=yes`,
`ProtectSystem=strict` and `MemoryMax=64M`. It holds no state on disk, so it needs no user,
no home and no writable path. Cross-compiled from Windows with the toolchain's own
`rust-lld` and `-C link-self-contained=yes` — the dependency tree is pure Rust, so no C
cross-toolchain is involved, and the box has only ~390 MB of RAM free, which is not enough
to compile tokio on.

On 2026-08-26 this instance was upgraded to the compatible implementation (static-musl binary
sha256 `b54a352b...0320b391`), with the prior binary retained as a rollback copy. The rollout was
performed in the designed order and observed live: old↔old succeeded, old-phone↔new-desktop
succeeded, then new↔new succeeded after the Android upgrade. Three subsequent phone process
restart cycles re-spliced cleanly, and an isolated same-role live-server probe logged stale waiter
replacement. Legacy 47-byte inference remains enabled only for older-client compatibility and
should be removed after M2 evidence and the compatibility window are complete.

Chosen over the other three hosts on latency (77 ms) and, decisively, on reachability: the
other candidate's outbound clients need a local HTTP proxy, and a relay the phone must reach
through a proxy on cellular is a relay that does not work.

frp was already installed on that box and was still rejected: `stcp` needs an `frpc` daemon
at each end, which is more lifecycle to audit than the whole relay is.

Verified from off-network: a pair spliced, 12 bytes crossed each way byte-for-byte, bad magic
was refused with a clean EOF, and after the pair closed the process held 0 sockets, 1.5 MB
RSS and 3 tasks.

## Image clipboard sync

PNG on the wire in both directions, so nothing has to negotiate a format. 32 KiB chunks,
because 64 KiB plus protobuf framing overflows the 65519-byte Noise plaintext ceiling and
`send` refuses an oversized frame — which would tear the session down over a pasted
screenshot. 10 MiB ceiling on one image: a clipboard is not a file transfer, and it is a
cheap thing to refuse before allocating anything for it.

No new dependency. `Windows.Graphics.Imaging` already ships PNG and BMP codecs and the
`windows` crate was already here for toasts, so image support cost two cargo features
instead of an image crate and its colour-management CVE stream. The fiddly part was not the
codec but the 14 bytes either side of it: `CF_DIB` is a .bmp with its `BITMAPFILEHEADER`
removed, and `bfOffBits` has to account for the colour table and, for a `BI_BITFIELDS` DIB,
the channel masks that follow the header.

Both clipboard formats are written inside one `with_clipboard` closure. Two `set_clipboard`
calls would each empty the clipboard first, so the second would delete the first and leave
either Paint or Chrome with nothing to paste.

windows-future 0.3.2 does not re-export the `Async` trait that carries the blocking `join`
— it is imported into the crate with `use r#async::*`, not `pub use` — so only `IntoFuture`
is reachable. Waiting on a WinRT operation is therefore a hand-rolled park/unpark
`block_on`, which has the side benefit of keeping the module synchronous so the clipboard
thread and `spawn_blocking` workers can both call it. `CoIncrementMTAUsage` gives the
process an implicit MTA so neither of those threads needs apartment bookkeeping.

The phone does not re-encode. Turning a 4 MB camera JPEG into PNG on a battery costs a full
decode and encode and can produce 20 MB — past the ceiling, so the transfer would be
refused *after* the work. It sends the source bytes with the provider's MIME type and the
desktop normalises, detecting PNG by signature rather than trusting the declared MIME.

Echo suppression differs per side, deliberately. Windows compares a `Seen` enum, text by
value and image by length. Android compares the clipboard URI's authority against its own
provider, which is exact and costs a string comparison instead of reading the file back on
the main thread to discover we wrote it.

## Phone → PC file publication and the false missing-file alarm

The receiver streams directly into one `conduit-<transfer-id>.part` file under the actual
Windows Downloads known folder, then publishes it only after the declared byte/chunk counts are
complete. The final name is sanitised and collision-reserved with `create_new`; a completed-file
toast opens the containing folder rather than executing peer-selected content.

A “missing file” investigation turned out to be timing, not post-completion deletion. On the
production relay, a 259,737-byte/eight-chunk transfer can take roughly 7–9 seconds from offer to
`file received`. Both an artificial exact-size probe and the original 259,737-byte phone
screenshot were invisible at their final filename through six seconds and present by eight;
both matched source SHA-256 exactly. Preserved evidence from an earlier 362,534-byte case showed
the same shape: an early check saw the zero-byte scratch, followed by normal completion and
scratch removal. Treat `file in, receiving` as initialization only; `file received path=...` is
the publication boundary.

The audit did uncover one real error-path issue. The receiver used `file == None` as a proxy for
successful publication, but the handle is intentionally taken and closed *before* final
reserve/rename. A reserve or rename failure in that window could therefore bypass `Drop` cleanup,
and a failed rename could leave the zero-byte reserved destination. Commit `d5554ec` adds an
explicit `published` state, cleans unpublished scratch files regardless of handle state, and
removes the owned placeholder on rename failure. Regression tests cover both failure windows.

## A phone photo becomes a Snipping Tool snip

The ask was Phone Link's behaviour: take a photo, and Windows offers it as though you had
just pressed Win+Shift+S. So the toast is not the feature — the click is.

Snipping Tool's protocol was read out of its own binaries rather than guessed:
`ms-screensketch://edit/?source=<who>&isTemporary=<bool>&sharedAccessToken=<token>`, and
`source=Toast` is one of the values it ships with. A file path in that URI is useless: it
is a packaged app in a container, and the handoff it expects is a token minted by
`SharedStorageAccessManager` that it redeems for the file. Whether an unpackaged daemon
could mint one at all was the open question, and it can — the token comes back as a plain
GUID, no package identity required.

`activationType="protocol"` is what keeps this cheap. Windows resolves the URI itself, so
there is no COM activator to register, no CLSID, and no callback the daemon has to stay
alive to serve.

One phone-capture toast at a time: camera photos and screenshots share a fixed tag, one staged
file and one outstanding token, each replaced by the next capture. That bounds all three by
construction rather than by cleanup, which is the only kind of bound this project trusts. The
cost is that a burst of captures leaves only the last one on screen.

No transcode on the desktop either. Both the toast image loader and Snipping Tool read
JPEG, the phone already downscaled, and re-encoding a photograph as PNG would multiply its
size — against a 3 MB cap on local toast images.

The photo never touches the clipboard. It is not something the user copied, and overwriting
their clipboard with it would be a mistake they cannot undo; on the phone side the same
frame arriving inbound is dropped for the same reason.

On the phone it is a `ContentObserver` on MediaStore and nothing else — no thread, no timer,
no poll. The callback does no work whatsoever: it hands the query, the decode and the JPEG
encode to the sender thread that already exists. `DCIM/%` and `DATE_ADDED` after service
start, deduped by MediaStore id, because the scanner writes a row several times per file and
the existing library is not news. `ImageDecoder` rather than `BitmapFactory`, because it
subsamples during the decode instead of allocating the full frame first, and it applies the
EXIF rotation a phone camera always writes — without which half the photos arrive sideways.

1280 px longest edge. A hero image renders at 364x180 dip so most of a 12 MP frame is bytes
nobody sees, and over the relay they are cellular data; it is not smaller because the toast
is only the doorway, and that is the resolution you then have to mark up.

Known ceiling: Android 14+ can grant "Selected photos only", which satisfies the permission
check while making new camera shots invisible to the query. The phone this targets is
rooted and granted through adb, so it is noted rather than handled.

## The heartbeat was measured from the wrong end

Found on real hardware, not in a test: sessions ended after exactly 150 000 ms, with
`frames_in=16` and `frames_out=0`.

The desktop's keepalive was a read timeout — it pinged after hearing nothing for 60 s. A
phone forwarding notifications is never silent that long, so the timeout kept being reset
and the desktop never sent anything at all. The phone's read deadline can only be satisfied
by hearing something, so it hung up on the dot and re-dialled. Two mechanisms that each
looked correct alone, producing session churn every two and a half minutes — precisely the
failure this project exists to avoid.

Silence has to mean *our* silence. The timestamp therefore lives in `Session` and is
updated by `send`, not in the session loop: every send in the process already funnels
through that one function, and a heartbeat that depends on each caller remembering to poke
a variable is a heartbeat that dies the first time someone adds a message kind.
