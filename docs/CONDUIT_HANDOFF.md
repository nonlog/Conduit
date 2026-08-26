# Conduit handoff

**Prepared:** 2026-08-26
**Repository:** `D:\Workspace\Conduit`  
**Branch:** `master`  
**Remote:** `https://github.com/nonlog/Conduit.git`

> **Maintenance rule:** keep this handoff current during active development, not only when a
> conversation is ending. Update it after a major implementation milestone, production/runtime
> change, important verification result, root-cause discovery, or change to the recommended next
> step. A new session should be able to resume safely from this file plus the linked docs even if
> the previous conversation ended abruptly.

## Do this first on resumption

1. Read `docs/architecture.md`, `docs/development.md`, `docs/progress.md`, `docs/backlog.md`, and
   `docs/TODO.md`. `TODO.md` is the compact unfinished-work checklist; the other docs carry the
   architecture, rationale, and evidence behind it.
2. Check the live repository state before changing anything:

   ```powershell
   Set-Location D:\Workspace\Conduit
   git status --short
   git log --oneline --decorate -8
   ```

3. The compatible relay migration is **deployed**. TYO runs the compatible server and the installed
   Android/Windows endpoints use explicit roles; keep legacy inference for older clients until M2
   evidence and deliberate retirement.
4. Windows Relay traffic is currently configured through `%LOCALAPPDATA%\Conduit\config.txt` to
   use local Mihomo/Clash Party at `socks5://127.0.0.1:7891`. LAN listener/direct LAN sessions do
   **not** use this proxy. Preserve the hostname `tyo.414222.xyz` through SOCKS so Mihomo can apply
   domain rules. Environment variables remain optional development overrides, not the normal store.
5. The latest Windows relay-park fix enables TCP keepalive **before** the parked socket waits in
   `peek()`. Do not remove this: a phone reboot exposed a zombie Windows responder waiter whose
   remote TYO side was already dead while Windows still showed the socket as `Established`.
6. Screenshot → native Windows toast → Snipping Tool is implemented and device-verified. The next
   P0 remains the actual endurance/flap evidence. `scripts/soak.ps1` is implemented and short-tested.
7. Product-level constraint: Conduit exists because Link to Windows used excessive phone CPU and
   caused lag/heat/battery drain. Do not add periodic Android speed tests, Relay probes, polling, or
   timer-driven scoring. Multi-Relay client selection is now implemented as passive quality learning
   + sticky failover: Windows may park on all configured Relays; Android keeps one session and learns
   only from real connection/session/content-transfer events. Only TYO is deployed publicly today,
   so live cross-node production failover still waits for explicit deployment approval.
8. Windows sign-in autostart is installed for the current user through HKCU Run. The current value
   points to the development binary under `D:\Workspace\Conduit\target\debug`; reinstall the entry
   when a stable packaged path exists. The daemon binds 41112 before starting long-lived workers, so
   duplicate manual/login launches fail fast instead of owning a second clipboard/Relay stack.
9. Explorer **Send to phone with Conduit** is installed for the current user. It points to the
   on-demand `target\debug\conduit-send.exe` helper beside the daemon; the helper is non-resident and
   reuses the daemon's named-pipe send/remote-ACK path. Reinstall the verb after packaging/moving the
   binaries.
10. Windows notification actions and inline reply are implemented. The resident toast thread owns
    foreground activation; there is no COM activator process. Android retains every PendingIntent,
    resolves the current notification only after a real click, and rejects stale action metadata.
    A real fixture E2E passed both reply text and a normal `Mark read` action through the encrypted
    session. Do not add a durable action queue across reconnects.

## Documentation created in this pass

| File | Purpose |
| --- | --- |
| `docs/architecture.md` | Current topology, component ownership, data flows, lifecycle invariants, bounds, security/trust boundaries, and relay migration warning. |
| `docs/development.md` | Scoop-first tooling, build/test/device workflow, debugging, Git attribution, and change discipline. |
| `docs/progress.md` | Dated test/device evidence, observed resource samples, unresolved caveats, and current repository state. |
| `docs/backlog.md` | Prioritised remaining work, with safe relay migration and endurance evidence at P0. |
| `docs/TODO.md` | Compact checklist split into pending implementation, pending verification, and protocol cleanup. |

This handoff now lives with the project documentation so the next session can resume from the
same authoritative state as the architecture/progress/backlog records.

## Repository state at handoff

```text
latest functional commit: 7dd206d Mirror notification actions to Windows
origin/master:             1c7e18c Send files from the share sheet, and stop toasting what the phone silenced
recent feature commits:    26427af control surface; be2d317 event status; eb31c73 Explorer send
```

Local `master` includes the tested persistence fix, screenshot implementation, compatible relay
migration, M0/M2 sampling, bidirectional file-transfer UX, long-transfer heartbeat fixes, Windows
parked-socket keepalive, Windows Relay SOCKS5 support, and notification actions/inline reply. None of these local commits has been
pushed. The compatible TYO relay and installed endpoints were built from this local line. A future
Git push is still outward-facing: obtain explicit approval unless requested in the same context.

At the current checkpoint, PC→phone CLI success means Android actually published the Downloads row.
A real 1 MiB device test observed last-chunk send first, then `FILE_RESULT`, then CLI success about
9 ms later. Relay/proxy configuration has moved out of the user environment into:

```text
%LOCALAPPDATA%\Conduit\config.txt
relays=tyo.414222.xyz:41113
relay_proxy=socks5://127.0.0.1:7891
```

The old user `CONDUIT_RELAY_PROXY` variable was removed after a no-env restart proved the config-file
path still used Mihomo. Clash Party/Mihomo on port `7891` remains a machine-runtime dependency for
accelerated Relay traffic, not a repository secret or a requirement for LAN use.

Commits created by the coding agent must use:

```text
Codex <codex@openai.com>
```

Do not casually delete old filter-branch backup refs or rewrite-recovery material until remote
history is independently verified.

## Current architecture in brief

- **Android:** Native Kotlin + Compose. `SyncService` owns one reusable `Link`; `MainActivity`
  provides the status/settings home plus a separate searchable clipboard-history page;
  `LinkTileService` exposes connect/disconnect through Quick Settings; `NotificationRelay` borrows
  the active link; `Discovery` is an 8-second mDNS burst; `Photos` and `Screenshots` independently
  observe camera/capture MediaStore changes; and `ShareActivity` forwards explicit file-share URI
  grants safely.
- **Windows:** Rust `conduit-daemon`, single LAN listener, one active session task,
  `SessionGuard::Drop` lifecycle accounting, native clipboard bridge, a dedicated COM/MTA toast
  thread, bidirectional disk-streamed file paths, local named-pipe `send <path>` control seam, and
  mDNS advert. Relay parking optionally dials through the configured Relay proxy; the current
  `%LOCALAPPDATA%\Conduit\config.txt` points this at local Mihomo SOCKS5. Parked Relay sockets enable TCP keepalive before
  blocking for a partner so a dead remote waiter cannot strand the parker forever.
- **Control-surface seam:** `%LOCALAPPDATA%\Conduit\status.txt` is an event-written snapshot, not a
  polled status service. `conduit-daemon status` currently reports daemon/link/phone/path/Relay state
  on demand. Android announces its device name once per encrypted session (`OnePlus 12` on the test
  phone). `conduit-control.exe` now consumes this seam as an on-demand GUI and exits fully when its
  window closes. Do not turn it into a tray app/background watcher. Functional UI is complete;
  lighter Fluent visual refinement remains separate TODO work.
- **Wire/security:** `Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue `conduit/1`; encrypted
  protobuf envelopes; `MAX_FRAME = 65535`, usable plaintext `65519`.  Images/files use 32 KiB
  chunks to fit after protobuf framing.
- **Relay:** both endpoints make outbound TCP connections to a blind byte splice.  It has no
  Noise/protobuf dependency and cannot decrypt payloads.
- **Core invariant:** Android `Socket().use {}` and Windows `SessionGuard` make
  `opened/created == closed` after quiescence, or differ by exactly one active session.

See `docs/architecture.md` for full data flow and trust boundaries.

## Verified capabilities and caveats

### Working/implemented

- Bidirectional text clipboard sync with normalised echo suppression.
- Bidirectional image clipboard sync.
- Android notifications as genuine native Windows toasts, including update/removal.
- Mirrored Android notification actions, including one free-form inline reply plus ordinary
  buttons. Windows keeps only bounded action descriptors; Android executes the current notification's
  PendingIntent after stale-metadata checks.
- Android-side suppression of Conduit’s own, ongoing, group-summary, media, and silent
  notifications.
- User-owned notification content-hide switch, persisted in app-private storage.
- App-icon/large-icon/Windows avatar cache path implemented.
- Phone → PC file share via Android’s share sheet; transfer is chunked to disk and partials are
  deleted on session failure.
- PC → phone file sending via `conduit-daemon send <path>`; Android publishes into Downloads only
  after complete receipt and deletes pending MediaStore rows on failure. The CLI waits for the
  receiver's whole-file publication result before returning success.
- Android file progress is shown both in-app and in a dedicated `File transfers` notification
  channel with direction-specific upload/download small icons; link status remains on `Link`.
- Android clipboard history is a dedicated searchable child page instead of occupying the home
  screen.
- Quick Settings `Conduit` tile toggles the same persisted connect/disconnect state as the app.
- Android day/night theme resources now explicitly choose dark/light system-bar glyphs. The build
  and compiled APK resources are verified and installed; an unlocked visual check is still pending.
- Direct Share target named after the remembered desktop.
- Camera photo → Windows hero-image toast → Snipping Tool activation implementation exists.
- Screenshot → Windows `New screenshot` toast → Snipping Tool is implemented and was verified
  on the target CPH2573 without changing the Windows clipboard.

### Do not overclaim

- M0/M2 endurance gates are not passed: the 48-hour LAN run is still outstanding, and M2 has
  clean short-cycle foreign-Wi-Fi↔cellular evidence but still needs a longer/hotspot campaign.
- Actual Nagram XF contact-avatar rendering has not yet been proven with a genuine notification.
- The former 259,737-byte “missing received file” caveat is resolved. The exact historical
  screenshot was replayed over the production relay: it was absent through six seconds,
  appeared by eight seconds, and matched the phone SHA-256 exactly. Earlier evidence showed the
  same mid-transfer observation pattern. Commit `d5554ec` also fixes cleanup after finalisation
  errors.
- Device/daemon logs can be stale/buffered; do not diagnose current connectivity from old temp
  logs alone.

## Latest evidence

- Android JVM tests: **25 passed, 0 failed**.
- Windows daemon normal test run: **49 passed, 3 ignored, 0 failed**. The added ignored test is an
  interactive native-toast activation check; it was run manually and returned both action arguments
  and Windows `UserInput` on the target machine.
- Compatible relay migration: **9 passed, 0 failed**, including legacy↔legacy, both mixed
  upgrade orders, explicit stale-role replacement, and legacy stale-phone replacement.
- Production rollout: old↔old and old-phone↔new-desktop connected through the compatible relay;
  installed new Android + new Windows now connect with `legacy=false` on both sides. Three forced
  phone restart cycles left Windows at `created=4 closed=4` before the fifth session became active.
- M0/M2 sampler: `scripts/soak.ps1` records resource samples and lifecycle logs; creation-side
  lifecycle counters are now emitted by both Android and Windows. A controlled short quiescent
  self-test ended at Windows `created=5 closed=5` and Android `opened=4 closed=4` with thread/FD
  counts back at baseline. It can also follow ADB transport changes by `ro.serialno`; a live
  `15557 → 15556` failover retained 100% Android sample coverage and a quiescent follow-up still
  ended with both lifecycle gaps at zero. This proves the collector works, not that M0 is complete.
- Bettbox fake-IP handover fix: the relay hostname resolved to `198.18.0.137` and produced
  `Broken pipe` after underlying-network changes without reaching TYO. Conduit now replaces only
  a `198.18.0.0/15` relay answer with TYO's public fallback `138.3.214.175`; actual traffic still
  follows Android/VPN routing. The device reconnects successfully with `legacy=false` after the
  substitution, and the selection logic is JVM-tested.
- M2 short-cycle evidence: six foreign-Wi-Fi↔cellular transitions kept lifecycle counters
  balanced. FD-class analysis proved apparent total-FD changes were APK/ashmem resource caching,
  not socket growth. A classified follow-up ended Windows threads 11→10, handles 264→261,
  Android sockets 7→7, anon-inodes unchanged, and both lifecycle gaps zero at 100% sample coverage.
- Last sampled daemon: about **9 threads**, **247 handles**, **24.1 MB working set**, about
  **276 minutes** uptime.
- Earlier lifecycle observation: 14 completed sessions with `created == closed`; an active
  relay link also survived approximately 96 minutes.  These are samples, not milestone proof.
- A post-reboot reconnect failure was traced to the Windows *parked* relay socket lacking client
  keepalive before `peek()`. TYO had already reaped the responder while Windows still reported a
  zombie `ESTABLISHED`; every phone retry then waited alone. The repaired daemon enables keepalive
  before parking, reconnects successfully, and leaves a fresh responder waiter at TYO.
- Windows Clash Party has TUN disabled, so native Conduit relay sockets were bypassing the local
  proxy. Relay-only SOCKS5 support is now persisted in `config.txt` as
  `relay_proxy=socks5://127.0.0.1:7891`; `CONDUIT_RELAY_PROXY` remains only an optional override.
  An isolated 4 MiB relay receive improved from 10.6 KiB/s DIRECT to 362.8 KiB/s through SOCKS5;
  a real 4 MiB PC→phone Conduit send completed in about 1.35 s and landed in Android Downloads.

Recent file/UI device evidence: the foreground notification reads `Linked to LOG`; a real Quick
Settings tile off/on cycle removed and restored the session/notification; separate transfer
notifications were observed as ID 2 upload / ID 3 download on `channel=transfers` while ID 1 stayed
on `channel=link`. PC→phone 131,071-byte and 1 MiB transfers matched SHA-256, a 64 MiB interrupted
receive removed its pending Android row at 7,471,104 bytes, and a 4 MiB phone→PC transfer completed
on the current progress build. Long-send heartbeat handling now keeps receive/send ciphertext in
separate Windows scratch buffers and lets Android answer PING between transfer chunks without
creating a second Noise writer.

Notification-action device evidence used a temporary standalone Android fixture so the verification
could prove real `PendingIntent`/`RemoteInput` execution rather than merely protobuf transport. A
Windows reply containing `Conduit reply E2E` produced `REPLY=Conduit reply E2E` in the fixture, and
the separate `Mark read` button produced `MARK`. The fixture APK was uninstalled afterward. The
final Conduit APK was rebuilt/reinstalled and the sensitive-notification AppOp re-granted; the phone
was locked at the end of the pass, so the service was not manually reconnected from the Activity.

## Android device facts

- Always run `adb devices -l` first and use an explicit serial after more than one transport
  appears.  The prior wireless transport became unreliable/closed during testing.
- After every APK reinstall, re-grant sensitive notification visibility on the test device:

  ```powershell
  adb -s <serial> shell cmd appops set com.conduit.sync RECEIVE_SENSITIVE_NOTIFICATIONS allow
  ```

- `getSharedPreferences()` silently did not persist on the target phone.  `Settings` and
  `History` now use `filesDir/settings.txt` and `filesDir/history.json`; preserve that design
  unless a true root cause/fix is established.
- Device defaults were restored after persistence testing:

  ```text
  hide_notification_content=false
  link_wanted=true
  ```

- Android logcat rotates rapidly; perform capture/action/dump in one test round or log to a
  device file.  Do not perform a screenshot and then wait through the short keyguard/bouncer
  window before operating it.

## Relay failure and deployed compatible migration

### Failure

The old fixed 47-byte preamble is:

```text
CDT1 + 43-character base64url desktop rendezvous ID
```

It does not encode whether the peer is the phone (Noise initiator) or desktop (Noise responder).
If a stale phone park remains and the same phone reconnects, the relay can splice two phone
initiators together.  A 32-byte Noise message 1 then arrives where the initiator expects the
80-byte message 2.  Android’s bounds hardening now reports a peer-protocol error rather than
an internal slice exception.

### Migration implementation

New client builds send:

```text
CDT1 + role byte + 43-character rendezvous ID
       > phone / initiator
       < desktop / responder
```

The waiting-map key is `(rendezvous ID, role)` and a same-role reconnect displaces the old
waiter. The compatible relay also accepts the deployed 47-byte form. Byte five is either an
explicit role or the first base64url id byte; for a legacy connection it peeks for up to one
second after the id. Immediate Noise bytes classify a phone/initiator, while a quiet connection
is the desktop/responder. `peek` leaves the Noise bytes untouched.

### Deployment state

Android `Link.kt` builds an explicit `>` initiator preamble and Windows `wire.rs::park` builds an
explicit `<` responder preamble. On 2026-08-26 the compatible relay was deployed first, old clients
were verified, Windows was upgraded and verified in a mixed session, then Android was upgraded.
The installed pair now uses explicit roles on both ends. The old relay binary is retained on TYO
as `/usr/local/bin/conduit-relay.pre-compat-20260826-100046` for rollback.

The old misleading role-slot test was replaced by `opposite_roles_of_one_id_splice_immediately`.
The two stale-waiter regressions are `a_peer_is_never_spliced_to_a_stale_copy_of_itself` and
`a_legacy_phone_reconnect_displaces_its_stale_copy`.

## Recommended next work

The highest-value remaining P0 is now the evidence run itself. Before a long run, retain the current
Windows Relay SOCKS configuration and include at least one real phone reboot / network-flap sequence
so the parked-socket keepalive fix is exercised rather than only a steady active session. M2 should
extend the successful
foreign-Wi-Fi↔cellular short cycles into a longer campaign including hotspot/default-network
variants. M0 still needs a true same-LAN phone/desktop setup before starting its 48-hour window;
the currently saved `www` Wi-Fi is a different subnet and cannot count as an M0 LAN run.
Do not remove legacy relay inference merely because current clients are upgraded; retire it only
after the compatibility window and M2 evidence are sufficient.

Also throttle Android transfer progress/notification refreshes before treating the UX as finished:
this is now implemented. Intermediate updates are capped at 4 Hz while the initial and final
progress edges remain immediate, so the 32 KiB wire cadence no longer becomes hundreds or
thousands of main-thread/SystemUI updates during a large transfer.

`docs/TODO.md` is now the canonical short checklist for remaining implementation and verification
work. Multi-relay selection/failover is implemented battery-first: no periodic phone benchmarks;
Windows parks all configured responders, Android selects one sticky Relay from persisted real-event
history and advances sequentially only inside a natural reconnect. A controlled Android test proved
failed-candidate→TYO fallback in one reconnect, and a local Windows test proved simultaneous parking
on two Relay processes. The remaining work is public US/WA/JP Relay deployment/live cross-node
evidence, which is outward-facing and still requires explicit approval.

## Useful commands

```powershell
# Android build and JVM tests
Set-Location D:\Workspace\Conduit\android
.\gradlew.bat assembleDebug testDebugUnitTest

# Rust tests
Set-Location D:\Workspace\Conduit
cargo test -p conduit-daemon
cargo test -p conduit-relay

# Check the test phone explicitly
adb devices -l

# Current diff
Set-Location D:\Workspace\Conduit
git status --short
git diff --check
```

Follow `docs/development.md` for Scoop-first tool installation and safe Android device workflow.
