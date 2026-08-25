# Conduit development progress

> **Snapshot date:** 2026-08-25  
> **Meaning of “verified”:** an observed test/device result, not an assumption inferred from
> source.  This record is intentionally more conservative than a feature checklist.

## Current position

Conduit has functioning implementation paths beyond the original pre-M0 description, but it
has **not** earned M0/M2 completion.  The central endurance requirement remains open: a
48-hour run must show no net thread, handle/FD, or session-lifecycle growth.

The latest protocol implementation commit on local `master` is `86a2b86`:

```text
Make relay role migration backward compatible
```

At this snapshot it is ahead of `origin/master`; do not treat it as published. The compatible
relay migration is committed locally but has not been deployed. The installed phone/daemon still
speak the deployed 47-byte form; do not install or restart the new role-aware clients until the
compatible relay is deployed first.

## Test evidence

| Area | Last recorded result | What it establishes | Limitation |
| --- | --- | --- | --- |
| Android JVM suite | **16 passed, 0 failed** | Noise transcript, frame limits, bounded history, file/image validation including capture flags, notification payload budget, wire behaviours, and explicit initiator relay preamble. | No actual system-server hook, notification listener, or device radio lifecycle. |
| Windows daemon | **39 passed, 2 ignored, 0 failed** on the last full normal run | Rust transport, clipboard/image/file/toast helpers, file-finalisation cleanup failures, screenshot semantics, resource-bound assertions, and explicit responder relay preamble. | Ignored tests show real toasts and require interactive validation. |
| Compatible relay migration | **9 passed, 0 failed** | Explicit-role splice, legacy 47-byte role inference without consuming Noise, both mixed upgrade orders, stale same-role replacement for new and legacy phones, dead-waiter recovery, and rendezvous isolation. | Production still runs the old relay; server-first rollout and live stale/flap proof remain. |
| Noise interoperability | JVM transcript test + Rust `snow` fixture | The hand-written Android Noise XX agrees byte-for-byte with a reference implementation. | Does not replace live-network testing. |

The misleading earlier `the_two_roles_of_one_id_are_separate_slots` test was replaced with
`opposite_roles_of_one_id_splice_immediately`. The load-bearing stale-waiter regressions now
cover both explicit-role peers and a deployed-format legacy phone reconnect.

## Device and feature evidence

### Android installation and persistence

- A debug APK containing the `filesDir` persistence change was installed on the test phone.
- The sensitive-notification AppOp was re-granted after install:

  ```text
  cmd appops set com.conduit.sync RECEIVE_SENSITIVE_NOTIFICATIONS allow
  ```

- Device inspection using `run-as` established that `SharedPreferences` did not create its
  expected directory or preserve writes on this phone, without a visible exception or SELinux
  denial.
- `filesDir` is reliable.  `identity.bin` and `peer-name.txt` exist there; planted and then
  service-written `settings.txt` values were read correctly.
- The current deliberate defaults after testing are:

  ```text
  hide_notification_content=false
  link_wanted=true
  ```

- `History` and `Settings` now use bounded app-private files.  A prior JVM regression caused
  by eagerly invoking stubbed `org.json` was fixed; `History.save()` returns before encoding
  when no file destination has been loaded.

### Functionality observed or previously exercised

| Capability | Status / evidence | Remaining qualification |
| --- | --- | --- |
| Bidirectional text clipboard | Implemented and exercised. | Needs long-duration lifecycle proof. |
| Bidirectional image clipboard | Implemented and previously verified. | Continue testing diverse `content://` providers and large-but-valid images. |
| Android notification → Windows native toast | Working; new, update, and removal paths are implemented. | Real platform checks remain useful after toast code changes. |
| Notification filtering | Device-shade inspection confirmed normal Play Store notification mirroring while media playback and Pano Scrobbler silent notifications were dropped. | Test other OEM/ranking edge cases when encountered. |
| Notification privacy setting | User-owned hide switch persists and defaults off. | Android listener redaction still needs the post-install AppOp. |
| Notification app icons / avatars | App icon and large-icon cache paths are implemented. | A genuine Nagram XF contact-avatar notification still needs end-to-end proof. |
| Phone → PC file share | Implemented and re-verified byte-for-byte over the production relay, including the exact historical 259,737-byte screenshot source. | A transfer is not visible at its final filename until all chunks arrive; UI/progress feedback is still intentionally minimal. |
| Direct Share target | The remembered desktop name is published to Android’s share sheet. | Verify after desktop rename/reinstall scenarios as needed. |
| Camera photo toast → Snipping Tool | Implementation exists: event-driven MediaStore watcher, staged image, shared-storage token, protocol activation. | Continue interactive checks after changes to the shared capture path. |
| Screenshot → Windows toast → Snipping Tool | **Implemented and verified on CPH2573.** A real `Pictures/Screenshots/Screenshot_...png` produced exactly one native `New screenshot` toast; clicking it opened that image in Snipping Tool. | Keep the target-device path/name filter current after OEM/platform changes. |

### Phone → PC file incident resolved — 2026-08-25

The earlier backlog entry described a roughly 259,737-byte receive as “logged completed but
missing”. Rechecking the preserved evidence and replaying the exact source shows that description
was too strong: the observed gap was a **mid-transfer filesystem check**, not a confirmed file
that disappeared after `file received`.

- Windows' real Downloads known folder on this machine is `D:\Downloads`.
- An independent 259,737-byte PNG probe was sent as eight 32 KiB chunks. At a six-second check
  the final filename was still absent; the daemon logged `file received` about nine seconds after
  the offer, then the file appeared at exactly 259,737 bytes. Source and destination SHA-256
  matched.
- The phone still contains the exact-size historical screenshot
  `Screenshot_2026-08-24-23-17-29-22_com.tencent.mm.png` (MediaStore id `1000004651`, 259,737
  bytes). Replaying that real source reproduced the timing: no final file at 2/4/6 seconds,
  present at 8 seconds, `file received` after about 7.35 seconds, and desktop SHA-256 exactly
  matched the phone (`318a0ab0...07edb2`).
- An earlier 362,534-byte screenshot test preserved the same pattern: a four-second check saw
  only the zero-byte scratch file, then the transfer completed and the scratch was replaced by
  the full destination. There is no preserved evidence of a completed destination subsequently
  being deleted.
- Source review did expose a separate finalisation-error cleanup hole. Commit `d5554ec` tracks
  publication independently from the open file handle, deletes `.part` after reserve/rename
  errors, and removes a zero-byte reserved destination if rename fails. Two regression tests
  cover those windows.

Phone → PC file transfer can therefore be treated as reliable at the tested sizes; future
failures should be diagnosed from timestamped `file in, receiving` versus `file received` lines,
not from an early directory snapshot.

### Screenshot verification — 2026-08-25

- The target OnePlus/ColorOS device stores captures under `Pictures/Screenshots/` and names
  ordinary system captures `Screenshot_...png`.
- A real system screenshot was observed once by `conduit.screenshot`; Windows received a
  71,105-byte PNG as three chunks with `photo=true,screenshot=true` and showed the native
  `New screenshot` toast.
- Clicking that Action Center entry opened the phone capture in Snipping Tool through the
  shared-storage-token `ms-screensketch://` activation path.
- Windows `GetClipboardSequenceNumber()` was **979 before and 979 after** the capture, proving
  this path did not overwrite the desktop clipboard during the test.
- Re-scanning the newest screenshot with `MEDIA_SCANNER_SCAN_FILE` produced no second capture:
  the daemon's capture-toast count remained **2 → 2** and Android emitted no new screenshot log.

## Lifecycle and resource observations

These values are encouraging samples, not exit criteria:

- An earlier controlled series observed **14 completed sessions** with `created == closed`.
- One active relay session remained alive for roughly **96 minutes** before later reinstall and
  testing activity changed the environment.
- Last sampled Windows daemon process:

  ```text
  pid=17556
  threads=9
  handles=247
  working set=24.1 MB
  uptime≈276 min
  ```

- Earlier M0 work also observed unchanged Android thread/FD/RSS values across six real
  desktop-restart cycles, with the reader-thread ID changing per connection.  This demonstrates
  teardown on those cycles; it does not establish a 48-hour zero-delta result.

The mandatory interpretation is:

```text
after quiescence: created == closed
while a link is up: created == closed + 1
```

A count that merely remains stable while an established socket never closes is not a lifecycle
test.  For example, removing an `adb reverse` rule does not close an existing socket and
therefore proves nothing about teardown.

## Relay investigation status

### Reproduced failure

A stale phone initiator socket parked under the same rendezvous ID could be paired with a new
phone initiator because the old preamble carried no role.  Each initiator then received a
32-byte first Noise message where it expected the 80-byte responder message, producing the
observed Android slicing failure and leaving recovery subject to retry timing.

Android Noise input now reports a protocol-sized short handshake error rather than a generic
internal `IndexOutOfBoundsException`.

### Compatible migration implemented locally, not released

New endpoint builds use:

```text
CDT1 + role byte + rendezvous ID
```

and the relay replaces a stale same-role waiter instead of self-splicing. For the rollout
window, the relay also accepts the currently deployed form:

```text
CDT1 + rendezvous ID
```

Legacy-role inference is deliberately narrow: after reading the 47-byte preamble, the relay
peeks for up to one second. Immediate post-preamble Noise bytes identify the phone/initiator;
a quiet legacy connection is the desktop/responder. The peek leaves Noise message 1 untouched.
Tests prove old↔old and both mixed upgrade orders, plus stale legacy-phone displacement.

The migration now has a safe order but is **not deployed**: install the compatible relay first,
then upgrade endpoints. New endpoint builds cannot be used against the currently deployed old
relay because it does not understand the extra role byte. Live stale-reconnect and network-flap
evidence remain part of the release gate.

### Live-state caveats

- Restarting the legacy daemon reproduced one old-relay fresh-park refusal at
  `13:29:59.932Z`, followed by a successful session on the normal retry at `13:30:15.441Z`.
  This is consistent with the already-reproduced same-role stale-waiter bug in the id-only
  47-byte relay: a new desktop can momentarily meet a stale desktop park. The compatible
  role-aware relay replaces same-role waiters instead; live confirmation remains part of its
  server-first rollout rather than a separate unknown-cause investigation.
- Phone logs showed a parked relay attempt; a final post-reinstall session conclusion was not
  captured because ADB transport and live logging became unreliable.
- Temporary daemon logs observed during testing were stale/buffered.  Timestamp and active
  process logging must be checked before diagnosing a current session from them.

## Known gaps and evidence still required

1. **M0 endurance:** 48 hours with zero net Android/Windows resource delta and lifecycle
   counters matching the invariant.
2. **M2 flap resilience:** repeated cellular ↔ Wi-Fi/hotspot changes with no growing session
   count, thread count, or relay-pair leak.
3. **Relay rollout:** deploy the compatible server first with explicit approval, then upgrade
   endpoints and validate real stale reconnect / opposite-role splice before retiring legacy
   inference.
4. **Avatar proof:** capture a real incoming Nagram XF notification carrying a contact icon.
5. **UI polish:** fix light-surface status-bar icon appearance in the Android app itself.  This
   is distinct from the already corrected monochrome foreground-service notification icon.
6. **Windows operability:** add daemon autostart at login and later a non-resident Fluent UI.

## Documentation maintenance

This progress record is not a release note.  Update its dated evidence when a test is run,
when an unresolved item is actually resolved, or when deployment status changes.  Do not mark
a milestone complete based solely on source review or a single happy-path run.
