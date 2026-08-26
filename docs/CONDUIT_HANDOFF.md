# Conduit handoff

**Prepared:** 2026-08-26
**Repository:** `D:\Workspace\Conduit`  
**Branch:** `master`  
**Remote:** `https://github.com/nonlog/Conduit.git`

## Do this first on resumption

1. Read `docs/architecture.md`, `docs/development.md`, `docs/progress.md`, and
   `docs/backlog.md`.  They were created in this documentation pass.
2. Check the live repository state before changing anything:

   ```powershell
   Set-Location D:\Workspace\Conduit
   git status --short
   git log --oneline --decorate -8
   ```

3. The compatible relay migration is **deployed**. TYO runs the compatible server and the installed
   Android/Windows endpoints use explicit roles; keep legacy inference for older clients until M2
   evidence and deliberate retirement.
4. Screenshot → native Windows toast → Snipping Tool is implemented and device-verified. The next
   P0 is endurance/flap instrumentation and evidence, not more relay protocol work.

## Documentation created in this pass

| File | Purpose |
| --- | --- |
| `docs/architecture.md` | Current topology, component ownership, data flows, lifecycle invariants, bounds, security/trust boundaries, and relay migration warning. |
| `docs/development.md` | Scoop-first tooling, build/test/device workflow, debugging, Git attribution, and change discipline. |
| `docs/progress.md` | Dated test/device evidence, observed resource samples, unresolved caveats, and current repository state. |
| `docs/backlog.md` | Prioritised remaining work, with safe relay migration and endurance evidence at P0. |

This handoff now lives with the project documentation so the next session can resume from the
same authoritative state as the architecture/progress/backlog records.

## Repository state at handoff

```text
protocol implementation: 86a2b86 Make relay role migration backward compatible
screenshot implementation: 02f0afe Mirror phone screenshots into Snipping Tool
origin/master:           1c7e18c Send files from the share sheet, and stop toasting what the phone silenced
```

Local `master` includes the tested persistence fix, screenshot implementation, and compatible
relay migration. None has been pushed. The deployed relay/installed clients were built from this
local line. A future Git push is still outward-facing: obtain explicit approval unless requested
in the same context.

Commits created by the coding agent must use:

```text
Codex <codex@openai.com>
```

Do not casually delete old filter-branch backup refs or rewrite-recovery material until remote
history is independently verified.

## Current architecture in brief

- **Android:** Native Kotlin + Compose. `SyncService` owns one reusable `Link`; `MainActivity`
  provides UI and legitimate foreground-service startup; `NotificationRelay` borrows the active
  link; `Discovery` is an 8-second mDNS burst; `Photos` and `Screenshots` independently observe
  camera/capture MediaStore changes; and `ShareActivity` forwards explicit file-share URI grants
  safely.
- **Windows:** Rust `conduit-daemon`, single LAN listener, one active session task,
  `SessionGuard::Drop` lifecycle accounting, native clipboard bridge, a dedicated COM/MTA toast
  thread, disk-streaming phone-file receive path, and mDNS advert.
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
- Android-side suppression of Conduit’s own, ongoing, group-summary, media, and silent
  notifications.
- User-owned notification content-hide switch, persisted in app-private storage.
- App-icon/large-icon/Windows avatar cache path implemented.
- Phone → PC file share via Android’s share sheet; transfer is chunked to disk and partials are
  deleted on session failure.
- Direct Share target named after the remembered desktop.
- Camera photo → Windows hero-image toast → Snipping Tool activation implementation exists.
- Screenshot → Windows `New screenshot` toast → Snipping Tool is implemented and was verified
  on the target CPH2573 without changing the Windows clipboard.

### Do not overclaim

- M0/M2 endurance gates are not passed: a 48-hour zero-delta lifecycle/resource run and
  network-flap proof remain outstanding.
- Actual Nagram XF contact-avatar rendering has not yet been proven with a genuine notification.
- The former 259,737-byte “missing received file” caveat is resolved. The exact historical
  screenshot was replayed over the production relay: it was absent through six seconds,
  appeared by eight seconds, and matched the phone SHA-256 exactly. Earlier evidence showed the
  same mid-transfer observation pattern. Commit `d5554ec` also fixes cleanup after finalisation
  errors.
- Device/daemon logs can be stale/buffered; do not diagnose current connectivity from old temp
  logs alone.

## Latest evidence

- Android JVM tests: **16 passed, 0 failed**.
- Windows daemon normal test run: **39 passed, 2 ignored, 0 failed**.
- Compatible relay migration: **9 passed, 0 failed**, including legacy↔legacy, both mixed
  upgrade orders, explicit stale-role replacement, and legacy stale-phone replacement.
- Production rollout: old↔old and old-phone↔new-desktop connected through the compatible relay;
  installed new Android + new Windows now connect with `legacy=false` on both sides. Three forced
  phone restart cycles left Windows at `created=4 closed=4` before the fifth session became active.
- Last sampled daemon: about **9 threads**, **247 handles**, **24.1 MB working set**, about
  **276 minutes** uptime.
- Earlier lifecycle observation: 14 completed sessions with `created == closed`; an active
  relay link also survived approximately 96 minutes.  These are samples, not milestone proof.

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

The highest-value remaining P0 is endurance/flap instrumentation and evidence: automate resource
sampling, then run the 48-hour M0 window and controlled cellular ↔ Wi-Fi/hotspot M2 transitions.
Do not remove legacy relay inference merely because current clients are upgraded; retire it only
after the compatibility window and M2 evidence are sufficient.

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
