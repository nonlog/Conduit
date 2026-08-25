# Conduit development progress

> **Snapshot date:** 2026-08-25  
> **Meaning of “verified”:** an observed test/device result, not an assumption inferred from
> source.  This record is intentionally more conservative than a feature checklist.

## Current position

Conduit has functioning implementation paths beyond the original pre-M0 description, but it
has **not** earned M0/M2 completion.  The central endurance requirement remains open: a
48-hour run must show no net thread, handle/FD, or session-lifecycle growth.

The repository is currently on local `master` commit `02f0afe`:

```text
Mirror phone screenshots into Snipping Tool
```

At this snapshot it is ahead of `origin/master`; do not treat it as published.  The working
tree also has uncommitted role-aware relay changes in `relay/src/main.rs`.  That draft is
incompatible with the current client preamble and must not be deployed alone.

## Test evidence

| Area | Last recorded result | What it establishes | Limitation |
| --- | --- | --- | --- |
| Android JVM suite | **15 passed, 0 failed** | Noise transcript, frame limits, bounded history, file/image validation including capture flags, notification payload budget, and wire behaviours covered by unit tests. | No actual system-server hook, notification listener, or device radio lifecycle. |
| Windows daemon | **36 passed, 2 ignored, 0 failed** on the last full normal run | Rust transport, clipboard/image/file/toast helpers, screenshot semantics, and resource-bound assertions. | Ignored tests show real toasts and require interactive validation. |
| Local role-aware relay draft | **7 passed, 0 failed** | Blind byte splice, invalid preamble rejection, stale same-role replacement, dead-waiter recovery, and rendezvous isolation. | The draft changes the protocol from 47 to 48 bytes; endpoints still send 47 bytes. |
| Noise interoperability | JVM transcript test + Rust `snow` fixture | The hand-written Android Noise XX agrees byte-for-byte with a reference implementation. | Does not replace live-network testing. |

The relay draft test named `the_two_roles_of_one_id_are_separate_slots` should be reviewed if
it is retained: normal opposite roles with one ID splice immediately, so that test currently
uses distinct IDs and does not demonstrate both roles parked under one rendezvous.  The
load-bearing regression coverage is `a_peer_is_never_spliced_to_a_stale_copy_of_itself`.

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
| Phone → PC file share | Implemented; at least one transfer was verified byte-for-byte. | One logged approximately 259,737-byte completed transfer was later absent from disk; cause unresolved. |
| Direct Share target | The remembered desktop name is published to Android’s share sheet. | Verify after desktop rename/reinstall scenarios as needed. |
| Camera photo toast → Snipping Tool | Implementation exists: event-driven MediaStore watcher, staged image, shared-storage token, protocol activation. | Continue interactive checks after changes to the shared capture path. |
| Screenshot → Windows toast → Snipping Tool | **Implemented and verified on CPH2573.** A real `Pictures/Screenshots/Screenshot_...png` produced exactly one native `New screenshot` toast; clicking it opened that image in Snipping Tool. | Keep the target-device path/name filter current after OEM/platform changes. |

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

### Local repair, not released

The local relay draft uses:

```text
CDT1 + role byte + rendezvous ID
```

and replaces a stale same-role waiter instead of self-splicing.  Its unit suite passes, but it
is deliberately unfinished because Android and Windows endpoint writers still emit:

```text
CDT1 + rendezvous ID
```

Do not deploy, publish, or casually merge the relay draft without the coordinated endpoint and
server migration work in [backlog.md](backlog.md#p0--correctness-and-release-safety).

### Live-state caveats

- A relay previously appeared to refuse fresh parks for a period, then self-healed.  Its cause
  has not been established.
- Phone logs showed a parked relay attempt; a final post-reinstall session conclusion was not
  captured because ADB transport and live logging became unreliable.
- Temporary daemon logs observed during testing were stale/buffered.  Timestamp and active
  process logging must be checked before diagnosing a current session from them.

## Known gaps and evidence still required

1. **M0 endurance:** 48 hours with zero net Android/Windows resource delta and lifecycle
   counters matching the invariant.
2. **M2 flap resilience:** repeated cellular ↔ Wi-Fi/hotspot changes with no growing session
   count, thread count, or relay-pair leak.
3. **Relay migration:** a compatible, coordinated protocol rollout for the role byte.
4. **File incident:** explain the logged-but-missing received file before calling phone → PC
   files reliable at all sizes.
5. **Avatar proof:** capture a real incoming Nagram XF notification carrying a contact icon.
6. **UI polish:** fix light-surface status-bar icon appearance in the Android app itself.  This
   is distinct from the already corrected monochrome foreground-service notification icon.
7. **Windows operability:** add daemon autostart at login and later a non-resident Fluent UI.

## Documentation maintenance

This progress record is not a release note.  Update its dated evidence when a test is run,
when an unresolved item is actually resolved, or when deployment status changes.  Do not mark
a milestone complete based solely on source review or a single happy-path run.
