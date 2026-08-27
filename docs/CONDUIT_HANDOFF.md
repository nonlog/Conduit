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

3. The compatible relay migration is **deployed**. US / TYO / WA run the compatible server and the installed
   Android/Windows endpoints use explicit roles; keep legacy inference for older clients until M2
   evidence and deliberate retirement.
4. Windows Relay traffic is currently configured through `%LOCALAPPDATA%\Conduit\config.txt` to
   use local Mihomo/Clash Party at `socks5://127.0.0.1:7891`. LAN listener/direct LAN sessions do
   **not** use this proxy. Preserve Relay hostnames through SOCKS so Mihomo can apply
   domain rules. Environment variables remain optional development overrides, not the normal store.
5. The latest Windows relay-park fix enables TCP keepalive **before** the parked socket waits in
   `peek()`. Do not remove this: a phone reboot exposed a zombie Windows responder waiter whose
   remote TYO side was already dead while Windows still showed the socket as `Established`.
6. Screenshot → native Windows toast → Snipping Tool is implemented and device-verified. The next
   P0 remains the actual endurance/flap evidence. `scripts/soak.ps1` is implemented and short-tested.
7. Product-level constraint: Conduit exists because Link to Windows used excessive phone CPU and
   caused lag/heat/battery drain. Do not add periodic Android speed tests, Relay probes, polling, or
   timer-driven scoring. Multi-Relay client selection is now implemented as passive quality learning
   + sticky failover: Windows parks on US / TYO / WA; Android keeps one session and learns only from
   real connection/session/content-transfer events, including real time-to-session-up EWMA. All three
   production Relay endpoints are deployed and reachable; no periodic probe or speed-test was added.
8. Windows sign-in autostart is installed for the current user through HKCU Run. The installer
   preserves the user choice while rewriting the value to the current installed daemon path. The daemon binds 41112 before starting long-lived workers, so
   duplicate manual/login launches fail fast instead of owning a second clipboard/Relay stack.
9. Explorer **Send with Conduit** is installed for the current user with the Conduit icon. It points
   to the on-demand `conduit-send.exe` helper beside the installed daemon; the helper is non-resident
   and reuses the daemon's named-pipe send/remote-ACK path. The installer refreshes the verb path.
10. Windows notification actions and inline reply are implemented. The resident toast thread owns
    foreground activation; there is no COM activator process. Android retains every PendingIntent,
    resolves the current notification only after a real click, and rejects stale action metadata.
    A real fixture E2E passed both reply text and a normal `Mark read` action through the encrypted
    session. Do not add a durable action queue across reconnects.
11. `conduit-control.exe` has completed its lightweight Fluent pass. It is still raw Win32 and
    on-demand only: no tray, WinUI/WebView runtime, timer, watcher, or transport ownership. It follows
    the Windows app theme/system accent, handles the target 125% DPI correctly, and exits to zero
    processes when closed. Preserve the immutable `OnceLock<Ui>` design; a mutex here caused a
    synchronous Win32 color-callback deadlock during Refresh.
12. Automatic non-root clipboard mirroring is now explicitly platform-blocked/deferred rather than
    an implementation backlog item. Android 10+ background clipboard access requires input focus or
    default-IME status; AccessibilityService alone does not satisfy the contract. Making Conduit the
    default IME solely for clipboard access would replace the user's normal keyboard, and Android has
    no supported thin-IME delegation path. Do not add accessibility/IME privileges unless the user
    explicitly chooses a different input-method product design or Android gains a suitable API.
13. 64 MiB bidirectional Relay stress is green for integrity. PC→phone completed in 20.33 s;
    phone→PC completed in about 4 min 46 s, crossed the 240-second heartbeat boundary, stayed linked,
    and matched SHA-256. The reverse direction is materially slower but did not lose/corrupt data.
14. Restrictive-provider sharing is now real-device verified. A temporary **non-exported** Android
    provider was unreadable to shell without a grant, then successfully shared a 1 MiB private file
    through `ShareActivity` → `SyncService` → Windows with an exact SHA-256 match. The fixture was
    removed. Preserve the current `ClipData` + `FLAG_GRANT_READ_URI_PERMISSION` handoff; replacing it
    with a plain URI extra would regress this case.
15. Direct Share desktop-name refresh is verified without touching the real Windows hostname. A
    process-local daemon name override changed Android's long-lived `desktop` sharing shortcut from
    `LOG` → `CONDUIT-RENAME-TEST`; restoring the normal daemon changed it back to `LOG`. Same Noise
    identity, same pairing and same shortcut id throughout. The current APK reinstall path also
    republished the shortcut successfully earlier in this pass.
16. Nagram X on the target phone does **not** populate `Notification.largeIcon` for the inspected
    conversation notification; it does carry `EXTRA_MESSAGES`. `NotificationRelay` now falls back to
    the newest public `MessagingStyle.Message.senderPerson.icon`. A sender-icon-only fixture produced
    an exact Windows face-cache SHA-256 match. Do not replay old private Nagram notifications just to
    close the final genuine-event check; wait for the next naturally posted notification.

17. MessagingStyle conversation history is implemented without new background work. Android reuses
    the public `Notification.EXTRA_MESSAGES` already present on a posted notification, keeps only the
    newest 3 non-empty messages (sender <=80 chars, text <=320 chars), and sends them on both New and
    Update. Windows renders the bounded records in the existing Toast body binding. A real Android
    system MessagingStyle notification carried Alice/Bob/Alice messages; Android logged
    `messages=3`, and the Windows daemon decoded `messages=3` across the live TYO/Mihomo Noise
    session. No query loop, provider read, extra thread, or new resident cache was added.

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
latest functional commit: 25db650 Mirror MessagingStyle conversation history
origin/master:             1c7e18c Send files from the share sheet, and stop toasting what the phone silenced
recent feature commits:    d056b80 Fluent control surface; 7dd206d notification actions; 26427af control surface
```

Local `master` includes the tested persistence fix, screenshot implementation, compatible relay
migration, M0/M2 sampling, bidirectional file-transfer UX, long-transfer heartbeat fixes, Windows
parked-socket keepalive, Windows Relay SOCKS5 support, notification actions/inline reply, and bounded MessagingStyle conversation history. None of these local commits has been
pushed. The compatible TYO relay and installed endpoints were built from this local line. A future
Git push is still outward-facing: obtain explicit approval unless requested in the same context.

At the current checkpoint, PC→phone CLI success means Android actually published the Downloads row.
A real 1 MiB device test observed last-chunk send first, then `FILE_RESULT`, then CLI success about
9 ms later. Relay/proxy configuration has moved out of the user environment into:

```text
%LOCALAPPDATA%\Conduit\config.txt
relays=us.414222.xyz:41113;tyo.414222.xyz:41113;wa.414222.xyz:41113
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
  phone). `conduit-control.exe` consumes this seam as an on-demand GUI and exits fully when its
  window closes. Its Fluent pass is complete using native Win32/DWM/Common Controls only; do not turn
  it into a tray app, background watcher, WinUI host, or second transport owner.
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
- Bounded MessagingStyle conversation history (newest 3 messages) is carried on both new/update events;
  Windows reuses the existing Toast body binding, so updates remain silent/in-place.
- Mirrored Android notification actions, including one free-form inline reply plus ordinary
  buttons. Windows keeps only bounded action descriptors; Android executes the current notification's
  PendingIntent after stale-metadata checks.
- Android-side suppression of Conduit’s own, ongoing, group-summary, media, and silent
  notifications.
- User-owned notification content-hide switch, persisted in app-private storage.
- App-icon/large-icon/Windows avatar cache path implemented, with public MessagingStyle sender-icon
  fallback for conversation apps that omit `Notification.largeIcon`.
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
  and compiled APK resources are verified and installed. Night mode is now visually verified on the
  unlocked Activity; the day/light visual check remains pending.
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

- Android JVM tests: **27 passed, 0 failed**.
- Windows daemon normal test run: **53 passed, 3 ignored, 0 failed**. The added ignored test is an
  interactive native-toast activation check; it was run manually and returned both action arguments
  and Windows `UserInput` on the target machine.
- Fluent control-surface verification: the target Windows dark theme at 125% scaling rendered a full
  818×729 physical window with rounded cards and no black DPI gutter. Manual Refresh stayed
  responsive; a normal close left 0 UI processes and no `%TEMP%\conduit-control-v6.manifest`.
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

## 2026-08-26 reconnect recovery checkpoint

- Frequent-disconnect diagnosis no longer points at the old Relay role/stale-waiter bug. In the observed failure, Android made repeated Relay dials that never reached TYO, consistent with a transient cellular/Bettbox path blackhole.
- Windows heartbeat now keeps an absolute 10 s PONG deadline after its 240 s Relay PING. Ordinary inbound notification/file/clipboard frames no longer satisfy that challenge, so a one-way PC -> phone failure cannot be hidden by phone -> PC traffic.
- Android recovery after a Relay session that was healthy for at least 60 s now uses the existing Handler/uptime retry mechanism with a 60 s ceiling for a bounded 10-minute awake-time recovery episode. Long outages still age back to the 300 s ceiling. No AlarmManager, wake lock, periodic probe, or extra radio wake was added.
- Automatic verification on this change: Windows daemon 50 passed / 3 ignored / 0 failed; Android 26 passed / 0 failed and assembleDebug succeeded.
- Real recovery check: a session that had been linked for >60 s was killed by stopping the Windows daemon at 23:27:51. The daemon was restarted 8 s later and the new Noise Relay session was up at 23:28:10, about 18.7 s after the forced loss. Current path remained TYO through Mihomo SOCKS5.
- Keep long-duration Relay + Mihomo stability in TODO: this proves prompt recovery from one controlled loss, not the full M2/soak gate.
- Post-recovery healthy-session check remained linked from 23:28:10 through 23:33:53 (>343 s), crossing the 240 s Relay PING boundary without a false disconnect; notification traffic still arrived at 23:33:30.
## 2026-08-27 sleep-aware reconnect observation

- The bounded 60-second Android recovery ceiling is **awake-time scheduling**, not an alarm. `Handler.postDelayed` intentionally does not wake a sleeping phone, preserving Conduit's low-radio/low-CPU design.
- In the final runtime normalization test, a 60-second retry became overdue while the phone slept and therefore did not execute. Waking only to the lockscreen (no unlock/screenshot) let the overdue retry run immediately: TYO spliced at 00:36:33 and Noise was up at 00:36:33.819; status returned to `linked`.
- Do not "fix" this by adding AlarmManager/WakeLock/background polling. If product requirements ever demand reconnect while the phone is fully asleep, treat the battery cost as an explicit design decision.
### 2026-08-27 Android + Windows UI redesign checkpoint

- **Design System Artifact**: Persisted `design-system/conduit/MASTER.md` under the repository using `search.py` (`--design-system --variance 2 --motion 2 --density 8 -p Conduit --stack jetpack-compose`), establishing a Swiss/Minimalism low-variance, low-motion, high-density direction tailored for a native utility app.
- **Android UI Redesign** (`MainActivity.kt`): Reorganized Jetpack Compose + Material 3 interface with a strong Hero Status surface (`StatusHero`) featuring dynamic container colors (`primaryContainer` / `tertiaryContainer` / `surfaceVariant`), status badges, route pills, peer fingerprint, and 48dp minimum touch targets. Active transfers (`TransferCard`) remain conditional on real file activity. Section rhythm is structured with distinct surface containers: `SyncPrivacyGroup` (grouped Clipboard History and Privacy Switch with dividers) and `IdentityGroup` (Outlined Card with monospace identity code block and dedicated Copy action button).
- **Windows UI Redesign** (`conduit-control.rs`): Native Win32/DWM/Common Controls surface reorganized into distinct Windows 11-style section cards with custom 1px border pens (`theme.border`), dynamic left accent indicator, structured status detail rows, grouped Relay Routing and Windows Integration cards, and native keyboard access keys (`&Refresh`, `&Save settings`, `Open &diagnostics`). Lifecycle remains strictly on-demand: 0 resident background processes, 0 timers, 0 watchers, 0 WebViews, 0 WinUI dependencies.
- **Source Formatting**: Formatted modified file individually using `rustfmt --edition 2021 windows/conduit-daemon/src/bin/conduit-control.rs` without disturbing unrelated files.
- **Verification**: `git diff --check` passed cleanly with 0 errors. Android `.\gradlew.bat --no-daemon assembleDebug testDebugUnitTest` completed with **BUILD SUCCESSFUL** (50 actionable tasks). Windows `cargo test -p conduit-daemon` completed with **51 passed, 0 failed, 3 ignored**; `cargo check -p conduit-daemon` completed successfully.
- **Audit**: Zero timers, zero polling loops, zero scheduled workers, zero background refresh threads, zero extra wake locks, zero AlarmManager loops added.
- **Runtime deployment**: The redesigned debug APK was installed in-place on the connected `CPH2573` test phone with `adb install -r`, and `com.conduit.sync/.MainActivity` was started so the user can inspect the new UI directly. No phone screenshot or screen capture was taken. The redesigned Windows `target/debug/conduit-control.exe` was also rebuilt and launched as a responsive top-level `Conduit Control` window; independent visual approval is still pending because the desktop-observation connector is blocked by caller-identity validation.
- **Privacy & Safety Confirmation**: Zero phone screenshots were captured, requested, or opened. Zero git commits or pushes were made.

### 2026-08-27 UI redesign v2 — rejected design superseded

- The preceding Android/Windows UI checkpoint was explicitly rejected after real-device review and is **superseded by this section**. Its large colored Android status hero, workflow/tagline copy, fingerprint/identity surfaces, redundant section explanations, and vertically stacked Windows dashboard-card layout are no longer the target design.
- `design-system/conduit/MASTER.md` was rewritten around a native system-utility language: Material 3/dynamic color on Android, Windows 11/Fluent principles on desktop, concise labels, progressive disclosure, standard density, and no marketing/value-proposition copy.
- Android `MainActivity.kt` now uses a compact neutral `ConnectionPanel`: peer name, connection state/route, and one connect/disconnect action. The home screen no longer renders the desktop fingerprint, the phone fingerprint/identity card, pairing/MAC-like identifiers, `Phone companion · quiet idle`, or the photo/screenshot workflow explanation.
- Android persistent settings are reduced to two compact rows: `Clipboard history` with its count and `Hide notification content` with its switch. Active transfers remain conditional on actual transfer state. Clipboard History no longer shows instructional filler such as `tap to copy`.
- Clipboard History navigation now installs `BackHandler(enabled = page == "history") { page = "home" }`, so the Android system Back/edge-back path is consumed by the child page before the Activity can exit. The code path is built and installed; direct gesture proof is pending because the test phone was locked/dozing during final automation and it was not unlocked solely for UI testing.
- Windows `conduit-control.exe` was restructured from the rejected full-width vertical card stack into a fixed 760×560 two-pane utility: connection/status and Diagnostics/Refresh on the left; Relay and Windows integration settings on the right; Save at bottom-right. Labels were shortened to `Connection`, `Relay`, `Windows`, `Endpoints`, `SOCKS5 proxy`, `Start at sign-in`, and `Send to phone in Explorer`.
- Windows icon identity now matches Android instead of using the rejected ad-hoc chain mark. This was later hardened into the static multi-resolution asset pipeline documented below; do not restore the earlier runtime GDI rasteriser. Relay keeps a connected-node mark and Windows keeps a four-pane mark.
- The peer-name control was also corrected after real review: it now uses a dedicated 18pt semibold font and a 126×58 DIP text area instead of the previous 24pt/122×38 DIP box. Runtime probe confirms the current `OnePlus 12` peer text is present in full in the control, with enough height for a second line when needed.
- Windows remains raw on-demand Win32/DWM/Common Controls with Segoe UI Variable, system light/dark theme, system accent, native keyboard access keys, and no WinUI/WebView/tray/timer/watcher/transport ownership.
- Final Android verification: `.\gradlew.bat --no-daemon assembleDebug testDebugUnitTest` -> **BUILD SUCCESSFUL** (50 actionable tasks; 9 executed, 41 up-to-date). Final APK was installed with `adb install -r` successfully and `com.conduit.sync/.MainActivity` was started without taking a phone screenshot.
- Final Windows verification: `rustfmt --check --edition 2021` passed; `cargo test -p conduit-daemon` -> **51 passed, 0 failed, 3 ignored**; `cargo check -p conduit-daemon` passed. `git diff --check` passed.
- Static audit of the UI diffs found no Android timer/poll/thread/scheduled-work/wake mechanism and no Windows timer/poll/thread/watcher implementation. The sole text match for `watcher` was a comment explicitly stating that no watcher or resize loop is introduced.
- The desktop-observation connector is still blocked by caller-identity validation, so assistant-side visual approval of the Windows window is not claimed. The rebuilt window can be launched directly for user review. No commit or push was made.

### 2026-08-27 Windows clipboard + notification identity repair

- The temporary `AGENTS.md` foreground-UI rule added during UI debugging was removed completely; it was not user-requested and `git diff -- AGENTS.md` is clean.
- **Windows image clipboard root cause:** `clipboard-win` image `Getter::read_clipboard` APIs require an already-open clipboard. Conduit called the image getters directly while the text helper opened the clipboard internally, so text sync worked while image sync silently returned nothing. The fallback also confused `CF_BITMAP` with `CF_DIB`: `clipboard-win` serialises `CF_BITMAP` as a complete BMP file (with BITMAPFILEHEADER), but Conduit fed those bytes to the DIB-only decoder. Modern Snipping Tool `CF_DIBV5` was not handled explicitly.
- `clip.rs` now reacts to the existing clipboard-change event with bounded `with_clipboard_attempts`, then reads in order: registered PNG, `CF_DIBV5`, `CF_DIB`, legacy `CF_BITMAP`. DIB/DIBV5 go through `dib_to_png`; CF_BITMAP's complete BMP goes through `to_png`. Remote image writes now advertise a real `CF_DIB` instead of misusing the CF_BITMAP setter. No idle polling/timer was introduced.
- **Physical-device E2E:** a Windows image clipboard update produced `DIBV5` **518,536 B**, Conduit logged `clip image out` **3,367 B**, OnePlus 12 logged `image in: 3367 B, photo=false screenshot=false` and `clip image in: 3367 B`, and `run-as com.conduit.sync ls -l cache/clip.png` reported a **3,367 B** file. No `could not put the image on the clipboard` failure appeared.
- The rejected low-resolution runtime GDI Conduit mark is superseded. `windows/conduit-daemon/tools/generate_icon.py` renders Windows assets from the Android launcher's exact source geometry/colors (`#6E5BD6` -> `#2F6FE0`, opposing white sync arrows) with supersampling. It produces a 512×512 RGBA PNG and an ICO with 16/20/24/32/40/48/64/128/256 px entries. `conduit-control.exe` loads those static ICO entries for titlebar/taskbar/in-window branding rather than rasterising a logo at runtime.
- `toast.rs` persists the same PNG as `%LOCALAPPDATA%\Conduit\conduit-icon.png` and registers `IconUri` plus `IconBackgroundColor=FF2F6FE0` under `HKCU\Software\Classes\AppUserModelId\Conduit.Desktop`; live registry inspection confirmed the values. This replaces the generic Windows glyph used when an unpackaged AUMID has no icon identity.
- Mirrored source app names (for example `ChatGPT`) no longer use Windows' tiny `placement="attribution"` line. They are now a normal ToastGeneric `hint-style="body"` text row. A dedicated unit test asserts both the body style and absence of the attribution placement.
- A real Android shell notification was mirrored through the live OnePlus -> TYO/Mihomo -> Windows session after the AUMID update; Android logged `notif out com.android.shell` and the Windows daemon logged `notif in app=Shell`, with no toast failure. The temporary probe notification was then snoozed out of the user's shade.
- Current Windows verification after these changes: **52 passed, 0 failed, 3 ignored**, `cargo check -p conduit-daemon` passed, all Windows binaries built, and `git diff --check` passed. Final daemon runtime was relaunched via `Win32_Process.Create` so it is not owned by an AgentDock command job; PID 35832, Session 2, parent `WmiPrvSE`, responding, and status returned to `linked` through TYO. No temporary scheduled task remains. No commit or push was made.

### 2026-08-27 Windows application identity + installation hardening

- User review exposed three packaging defects: Action Center still used the generic Win32 glyph, the in-window mark was soft at scaled DPI, and manually running the daemon exposed a console window.
- `tools/install-windows.ps1` now creates `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Conduit.lnk` with `System.AppUserModel.ID=Conduit.Desktop`, points it at the installed GUI, and assigns the shared Conduit ICO. This follows Microsoft's unpackaged desktop-toast identity model instead of relying on registry `IconUri` alone.
- The user-facing installed entry is `%LOCALAPPDATA%\Programs\Conduit\Conduit.exe`; `conduit-daemon.exe` and `conduit-send.exe` are internal siblings. The old installed `conduit-control.exe` name is removed. The installer uses Windows Known Folder APIs instead of inherited `APPDATA/LOCALAPPDATA`.
- Both daemon and GUI set explicit process AUMID `Conduit.Desktop`. `conduit-daemon.exe` is now Windows GUI subsystem (`PE Subsystem=2`), and HKCU Run launches it directly without PowerShell/cmd. Opening `Conduit.exe` performs one on-demand 41112 single-instance probe and starts the hidden sibling only when needed; closing the GUI leaves the daemon running.
- Icon generation now uses 8x supersampling and includes dedicated ICO frames for native sizes plus common physical sizes for the 34-DIP title and 44-DIP connection marks. The GUI loads separate HICONs at the current monitor DPI instead of stretching one 48px icon.
- The stale `Conduit.Desktop` notification-settings cache contained only counters/timestamps and no user preference values; that one Conduit-specific entry was backed up, removed, and regenerated after the Start-menu shortcut existed. A controlled notification-center check then found a 20x20 Conduit violet/blue cluster at the notification header position; the closed-panel control image contained zero brand-color pixels. Temporary test artifacts were removed.
- Final installed runtime: `%LOCALAPPDATA%\Programs\Conduit\Conduit.exe` PID 2600 responding with title `Conduit`; one installed hidden daemon PID 7332 responding. Both subsystem probes returned 2. Windows tests remain **52 passed, 0 failed, 3 ignored**; release all-bin build and `git diff --check` passed. No commit or push.

## 2026-08-27 release-candidate shell / relay checkpoint

- Windows device naming no longer depends on the `COMPUTERNAME` process environment. The daemon
  reads `ActiveComputerName`; a launch with `COMPUTERNAME` deliberately removed advertised `LOG`,
  and the installed phone persisted `peer-name.txt = LOG` after the encrypted session handshake.
- Production Relay inventory is `US / TYO / WA`. Windows parks one responder at each endpoint while
  Android holds one Relay session. `RelayQualityStore` v2 passively persists real success/failure,
  unstable-session evidence, completed image/file goodput and session-up EWMA per coarse network
  class. A forced reconnect produced independent real US/TYO/WA records; no ping, periodic speed
  test, probe worker or timer-driven scoring was introduced.
- Explorer integration is `Send with Conduit`; the installed verb points at the installed
  `conduit-icon.ico` and `conduit-send.exe`. A real Windows 11 context menu showed the product icon.
- The optional daemon-owned tray menu is deliberately text-only: `Open Conduit` and `Exit Conduit`.
  `Exit Conduit` was exercised against the live tray window and terminated the daemon; the daemon
  was then restarted detached and linked normally.
- Product artwork is based on Microsoft Fluent UI System Icons `Phone Desktop`, with the same
  phone/desktop geometry on Android and Windows. The tray uses dedicated monochrome 16/20/24 px
  regular glyphs rather than a shrunken coloured application tile.
- Final pre-release verification: Android `assembleDebug + testDebugUnitTest` succeeded with
  **27 passed / 0 failed**; Windows `cargo test` is **53 passed / 0 failed / 3 ignored**, and
  `cargo check` plus release all-bin build succeeded.
- No unlocked-phone foreground screenshot was captured during this checkpoint.