# Conduit development progress

> **Snapshot date:** 2026-08-26
> **Meaning of “verified”:** an observed test/device result, not an assumption inferred from
> source.  This record is intentionally more conservative than a feature checklist.

## Current position

Conduit has functioning implementation paths beyond the original pre-M0 description, but it
has **not** earned M0/M2 completion.  The central endurance requirement remains open: a
48-hour run must show no net thread, handle/FD, or session-lifecycle growth.

The latest functional implementation commit on local `master` is `d056b80`:

```text
Polish the Windows control surface
```

Local `master` remains ahead of `origin/master`; do not treat the source commits as published.
The protocol rollout itself **is deployed on the test/production path**: TYO now runs the
compatible relay, and the installed Android/Windows endpoints now send explicit roles. Legacy
47-byte inference remains enabled only as an upgrade bridge for older clients.

## Test evidence

| Area | Last recorded result | What it establishes | Limitation |
| --- | --- | --- | --- |
| Android JVM suite | **25 passed, 0 failed** | Noise transcript, frame limits, bounded/searchable history model, file/image validation including capture flags, throttled transfer-progress model, notification payload budget, wire behaviours, explicit initiator relay preamble, fake-IP relay fallback, passive multi-relay candidate/cooldown persistence, and file-publication result encoding. | No actual system-server hook, notification listener, Quick Settings host, or device radio lifecycle. |
| Windows daemon | **49 passed, 3 ignored, 0 failed** on the last full normal run | Rust transport, clipboard/image/file/toast helpers, desktop→phone outbound-file validation, file-finalisation cleanup failures, screenshot semantics, resource-bound assertions, explicit responder relay preamble, multi-relay config parsing/parking, cancel/resume receive safety across an intervening Noise send, SOCKS5 relay domain routing, autostart/Explorer helpers, event-driven control-surface status snapshots, and notification-action XML/activation parsing. | The three ignored tests exercise real Windows toasts and require interactive validation; the new action callback test was also run interactively on the target Windows machine. |
| Compatible relay migration | **9 passed, 0 failed** | Explicit-role splice, legacy 47-byte role inference without consuming Noise, both mixed upgrade orders, stale same-role replacement for new and legacy phones, dead-waiter recovery, and rendezvous isolation. | Production rollout is complete; long-duration M2 flap evidence and eventual legacy-path retirement remain. |
| Noise interoperability | JVM transcript test + Rust `snow` fixture | The hand-written Android Noise XX agrees byte-for-byte with a reference implementation. | Does not replace live-network testing. |

The misleading earlier `the_two_roles_of_one_id_are_separate_slots` test was replaced with
`opposite_roles_of_one_id_splice_immediately`. The load-bearing stale-waiter regressions now
cover both explicit-role peers and a deployed-format legacy phone reconnect.

### Relay production rollout — 2026-08-26

- TYO `tyo.414222.xyz:41113` was upgraded server-first at **10:00:46 +08** to the compatible
  static-musl relay (`sha256 b54a352b...0320b391`). The previous binary was preserved as
  `/usr/local/bin/conduit-relay.pre-compat-20260826-100046` (`sha256 9ff6b8af...9baff6a`).
- Before either endpoint was upgraded, the existing clients established a real session through
  the new relay. The relay classified the old phone as `role=> legacy=true` and the old desktop
  as `role=< legacy=true`.
- Windows was upgraded next. A real mixed session succeeded with old Android
  `role=> legacy=true` and new Windows `role=< legacy=false`.
- Android was then rebuilt (`16 passed, 0 failed`), reinstalled, and its sensitive-notification
  AppOp re-granted. The final real session is explicit on both ends:
  `role=> legacy=false` phone and `role=< legacy=false` desktop.
- Three forced Android process stop/restart cycles then produced three clean re-splices. Windows
  reached `created=4 closed=4` before the fifth session became active; no 36/36 initiator
  self-splice reappeared.
- A separate production-relay probe under an isolated rendezvous id presented two explicit `>`
  waiters. The second logged `displaced a stale waiter`; an opposite-role probe then consumed the
  remaining waiter and closed the test pair, proving the replacement branch on the live server
  without disturbing the real device id.

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
| Windows notification action / inline reply | **Implemented and real-device verified.** The Windows resident toast thread receives foreground action activation and free-form `UserInput`, sends one `NOTIF_ACTION` through the live Noise session, and Android resolves/executes the current notification's `PendingIntent`/`RemoteInput`. A temporary fixture notification returned `REPLY=Conduit reply E2E`; its ordinary `Mark read` action returned `MARK`. | No durable action queue exists by design; clicks while disconnected are dropped. Multiple free-form reply actions on one Android notification are reduced to one Windows reply box; ordinary buttons remain available. |
| Notification filtering | Device-shade inspection confirmed normal Play Store notification mirroring while media playback and Pano Scrobbler silent notifications were dropped. | Test other OEM/ranking edge cases when encountered. |
| Notification privacy setting | User-owned hide switch persists and defaults off. | Android listener redaction still needs the post-install AppOp. |
| Notification app icons / avatars | App icon and large-icon cache paths are implemented. | A genuine Nagram XF contact-avatar notification still needs end-to-end proof. |
| Phone → PC file share | Implemented and re-verified byte-for-byte over the production relay, including the exact historical 259,737-byte screenshot source and a 4 MiB current build transfer. Android shows byte/percent progress in-app and in a separate upload notification. | Continue endurance/very-large-file testing; the final Windows filename remains atomic and therefore appears only after all chunks arrive. |
| PC → phone file send | **Implemented and device-verified.** `conduit-daemon send <path>` hands a validated path to the resident daemon over a local named pipe; Android streams it into a pending Downloads MediaStore row. 131,071-byte and 1 MiB transfers matched SHA-256; a 64 MiB interrupted transfer deleted its pending row at 7,471,104 bytes. The CLI now waits for Android's post-publication `FILE_RESULT` before returning success. | A future Windows UI/right-click surface can reuse the same local control pipe and remote-result semantics. |
| Direct Share target | The remembered desktop name is published to Android’s share sheet. | Verify after desktop rename/reinstall scenarios as needed. |
| Camera photo toast → Snipping Tool | Implementation exists: event-driven MediaStore watcher, staged image, shared-storage token, protocol activation. | Continue interactive checks after changes to the shared capture path. |
| Screenshot → Windows toast → Snipping Tool | **Implemented and verified on CPH2573.** A real `Pictures/Screenshots/Screenshot_...png` produced exactly one native `New screenshot` toast; clicking it opened that image in Snipping Tool. | Keep the target-device path/name filter current after OEM/platform changes. |

### Android UI / notification controls — 2026-08-26

- Clipboard history no longer occupies the home list. `History` opens a dedicated Compose page
  with a search field; filtering is case-insensitive across preview text and direction, and the
  bounded 100×200-character persistence model is unchanged.
- The foreground link notification now says **`Linked to LOG`** on the test pair rather than
  `Clipboard linked to the desktop`. Killing/disconnecting the desktop removes the link
  notification; reconnecting recreates it only after Noise is up.
- Link state and file-transfer progress no longer share a notification channel. `Link` owns only
  the connection notification. `File transfers` owns independent upload/download progress
  notifications (IDs 2/3) with distinct upload/download monochrome status-bar icons. During a
  live download, dumpsys showed ID 1 on `channel=link` still saying `Linked to LOG`, while ID 3
  was on `channel=transfers` at 23%; a live upload similarly used ID 2 and the upload icon.
- A Quick Settings `Conduit` tile was added on the target ColorOS device. A real tile click while
  linked persisted `link_wanted=false`, closed the session and removed the notification; a second
  click restored `link_wanted=true`, established a new relay session, and restored the active tile.
- The home page shows one transfer card per active direction with filename, bytes and percentage.
  Both receiving and sending cards were visually checked on the device.
- The app theme now sets `windowLightStatusBar/windowLightNavigationBar=true` in the day resource
  and `false` in `values-night`. `aapt2 dump resources` verified both compiled variants in the APK,
  and the APK was installed. The device later became unlocked naturally; a real Conduit Activity
  screenshot then confirmed the **night** half visually: dark app surface with light status icons and
  a light navigation gesture bar. The day/light half still needs a visual check; lockscreen SystemUI
  is not evidence for Activity bar colours.

### Large bidirectional Relay transfer stress — 2026-08-26

- A generated **64 MiB** file (`67,108,864 B`) with SHA-256
  `3b6a07d0d404fab4e23b6d34bc6696a6a312dd92821332385e5af7c01c421351` was sent from Windows to
  Android through the current TYO + Mihomo path. `conduit-daemon send` waited for Android's remote
  publication ACK and returned success after **20.33 s**; the Downloads file had the exact size and
  hash.
- The same Android Downloads object was then shared back to Windows through the real exported
  `ShareActivity` URI-grant path. Android logged all **2048 × 32 KiB** chunks written. The Windows
  receiver created its scratch at 20:31:54 and atomically published the final file at 20:36:40,
  about **4 min 46 s** end to end. That crosses the Relay's 240-second outbound-idle heartbeat
  boundary while Android's single sender is occupied, exercising the `pongPending` between-chunks
  repair under a real large transfer. The session remained linked and the final Windows SHA-256
  matched exactly.
- A 90-second mid-transfer observation initially saw only the expected zero-length scratch and no
  final filename. That is the same Windows metadata/publication timing pattern documented in the
  earlier false “missing file” incident, not data loss: the final file appeared only after the full
  transfer and rename completed. All three stress copies were removed afterward.

### Non-root clipboard fallback constraint — 2026-08-26

- The former M3 note proposed an `AccessibilityService` fallback. Current Android platform behaviour
  makes that insufficient: on Android 10+ `ClipboardManager.getPrimaryClip()` returns no clipboard
  data to an ordinary background app unless it has input focus or is the default IME. Accessibility
  callbacks do not by themselves grant that clipboard role.
- No accessibility service was added. Requiring a high-privilege accessibility toggle for a path
  that still cannot read the clipboard would be misleading and would expand Conduit's permission
  surface for no benefit. M3 is now explicitly a product/design decision around a truly authorised
  input path or future platform API.

### Restrictive `content://` share-grant verification — 2026-08-26

- A temporary Android fixture was built outside the repository with a provider declared
  `exported=false` and `grantUriPermissions=true`. Its private file was therefore genuinely
  inaccessible without an explicit grant: `adb shell content query` was rejected by Android with
  `SecurityException` because the provider was not exported.
- The fixture created a deterministic **1 MiB** private file and explicitly targeted
  `com.conduit.sync/.ShareActivity` with `ACTION_SEND`, `EXTRA_STREAM`, `ClipData`, and
  `FLAG_GRANT_READ_URI_PERMISSION`. This is the exact constrained-provider case the share path was
  designed for: Conduit has no blanket provider access and can read the URI only because of that
  share grant.
- `ShareActivity` passed the URI/grant into the existing `SyncService` path; Android logged
  `sent restricted-provider.bin, 1048576 B as 32 chunks`. Windows published the file in Downloads
  immediately afterward. Source and destination SHA-256 were both
  `631b84027d6b9e52b539c4e8373622d23032dfadc64d60af87339c9037e4f769`.
- The Windows copy, temporary APK/app and complete temporary Gradle project were removed after the
  check. No repository source change was required.

### Bidirectional file transfer / long-send heartbeat findings — 2026-08-26

- Desktop→phone reuses the existing `FILE_OFFER` / `FILE_CHUNK` protocol rather than adding a
  second file dialect. Android publishes a MediaStore Downloads row only after exact declared
  byte/chunk completion; an interrupted 64 MiB test removed `.pending-...` at 7,471,104 bytes.
- A first long phone→desktop test exposed a Windows cancel-safety hole: `Session::recv` preserved
  partial-frame offsets across a heartbeat timeout, but send and receive shared the same ciphertext
  scratch buffer. A PING inserted between partial reads overwrote inbound ciphertext and caused
  `decrypt error`. Windows now owns separate fixed `cipher_in` / `cipher_out` buffers, with a
  regression test that cancels mid-body, performs a send, then resumes/decrypts the original frame.
- Re-running the long upload then exposed a second issue rather than ciphertext corruption:
  Android's one sender executor could be occupied for an entire file, leaving a PONG queued behind
  the transfer until Windows' 10-second probe deadline. Android now marks `pongPending` and services
  it **between file/image chunks on the same sender executor**, preserving the single-writer Noise
  invariant. A normal 4 MiB upload on that build completed and published correctly; a timed
  across-heartbeat device check is recorded separately once completed.
- Desktop→phone now has one receiver-side `FILE_RESULT` after whole-file publication. There is no
  per-chunk ACK. The resident daemon correlates it by `transfer_id`, keeps at most one desktop send
  awaiting publication, and resolves the local named-pipe caller only after the result arrives.
  A real 1 MiB test wrote the last Windows chunk at **18:07:56.028**, received Android publication
  confirmation at **18:07:56.800**, and the CLI exited successfully at **18:07:56.809** with
  `Sent to phone`. The exact-size file existed in Android Downloads before the test copy was removed.

### Relay waiter liveness and Windows Mihomo routing — 2026-08-26

- A real phone reboot reproduced a different reconnect failure after the role-byte migration. The
  phone repeatedly reached TYO as `role=> legacy=false`, but no Windows responder remained in the
  relay waiting map. Windows still showed its old `41113` socket as `ESTABLISHED`; TYO had already
  timed that waiter out. `wire::park()` had enabled TCP keepalive only *after* `peek()` returned,
  so a silently-dead parked responder could block forever and prevent `park_forever` from creating
  its replacement. Windows now enables keepalive immediately after connecting, before sending the
  relay preamble and entering the parked `peek()`.
- Restarting the repaired daemon immediately restored the live pair, and TYO then showed a fresh
  `role=< legacy=false` responder parked for the next reconnect. The phone notification returned
  to `Linked to LOG`.
- Transfer-speed diagnosis was corrected after confirming the phone already routes
  `tyo.414222.xyz:41113` through Bettbox/Mihomo. Windows Clash Party had TUN disabled, so the Rust
  raw `TcpStream` bypassed its HTTP/system proxy and connected to TYO directly. An isolated 4 MiB
  relay receive measured **10.6 KiB/s** on Windows DIRECT versus **362.8 KiB/s** when the Windows
  leg explicitly used Mihomo SOCKS5 at `127.0.0.1:7891`.
- `CONDUIT_RELAY_PROXY=socks5://127.0.0.1:7891` now routes only Windows Relay connections through
  SOCKS5; LAN traffic remains direct. SOCKS5 CONNECT uses the relay hostname rather than a locally
  resolved IP so Mihomo DOMAIN rules still apply. The user-level variable is persisted on the test
  machine. A real `conduit-daemon send` of 4 MiB then completed its daemon send in about **1.35 s**
  and the exact-size file appeared in Android Downloads.

### Battery-first multi-relay client implementation — 2026-08-26

- Windows now accepts `CONDUIT_RELAYS` and owns one independent parked responder per configured
  endpoint. A controlled local test started two Relay processes on ports 42113/42114; each recorded
  the desktop as `role=< legacy=false` simultaneously. The normal TYO/Mihomo daemon was restored
  afterwards and the phone reconnected.
- Android now reads an app-private Relay inventory at service start and keeps one candidate queue
  only for the current natural reconnect. It records real dial success/failure, short-session
  instability and completed bulk-payload goodput in `relay-quality.txt`; there is no timer/probe API.
- A controlled device test temporarily put an unreachable loopback endpoint ahead of TYO with an
  empty quality history. The same reconnect recorded `bad` as one dial failure, advanced to TYO,
  recorded a TYO success and restored `Linked to LOG` without waiting for the general retry backoff.
  The temporary inventory/history were then removed and the phone returned to the production
  single-TYO default.
- A subsequent real 4 MiB desktop→phone transfer updated TYO's passive goodput sample to about
  **2.51 MB/s** and produced the exact-size Downloads file, demonstrating that scoring uses real
  content rather than synthetic benchmark traffic.
- Only TYO currently has a public Conduit Relay listening on port 41113. US/WA/JP deployment and
  live cross-node selection remain separate outward-facing work.

### Windows sign-in autostart — 2026-08-26

- `conduit-daemon autostart install|remove|status` now manages one per-user
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Conduit` value. The Run command uses a
  short-lived hidden Windows PowerShell process to `Start-Process` the console-subsystem daemon
  hidden; PowerShell does not wait and therefore adds no steady-state process.
- The daemon binds TCP 41112 immediately after identity load, before clipboard, toast, local-control
  or Relay workers exist. That listener is also the single-instance gate. With one live daemon, a
  second normal launch exited in **0.04 s** with `os error 10048`; process inspection still showed
  exactly one daemon.
- The real current-user Run value was installed on LOG and its hidden launch command was executed
  manually as a login-equivalent check. It started exactly one daemon process successfully. The
  current development Run value points at `D:\Workspace\Conduit\target\debug\conduit-daemon.exe`;
  release packaging should reinstall it to the eventual stable installed path.

### Explorer file-send integration — 2026-08-26

- `conduit-daemon explorer install|remove|status` manages a per-user
  `HKCU\Software\Classes\*\shell\Conduit.SendToPhone` verb labelled **Send to phone with Conduit**.
  It is single-file today, matching the daemon's serialized explicit-send contract.
- The first hidden-PowerShell command design was rejected during testing because a legitimate
  apostrophe in a file path can be reinterpreted by `powershell.exe -Command`. The shipped design
  instead uses a separate **~283 KiB** GUI-subsystem `conduit-send.exe` beside the daemon. It receives
  the Explorer path as ordinary argv, starts `conduit-daemon.exe send <file>` with
  `CREATE_NO_WINDOW`, waits for remote publication, and exits. No helper remains resident.
- The real HKCU verb is installed on LOG. A **262,144-byte** test file named
  `Conduit O'Brien test.bin` traversed the helper successfully, appeared under the exact name in
  Android Downloads, and the resident daemon logged Android publication confirmation. The test file
  was then removed.

### Persistent Windows Relay/proxy configuration — 2026-08-26

- The daemon now reads `%LOCALAPPDATA%\Conduit\config.txt` once at process start. Supported keys are
  `relays=` and `relay_proxy=`; there is no file watcher or reload thread. Existing
  `CONDUIT_RELAYS`, `CONDUIT_RELAY`, and `CONDUIT_RELAY_PROXY` variables remain explicit development/
  compatibility overrides rather than the normal user path.
- `conduit-daemon config show`, `config relay-proxy <value|off>`, and `config relays <list|off>`
  provide a user-facing path without requiring registry/environment editing. Changes intentionally
  apply on daemon restart so configuration adds zero steady-state background work.
- LOG was migrated from its user `CONDUIT_RELAY_PROXY` variable into `config.txt`, then that user
  environment variable was removed. A daemon started with no process proxy variable logged
  `proxy="socks5://127.0.0.1:7891"`, connected to TYO with peer `127.0.0.1:7891`, and the phone
  remained `Linked to LOG`. Current config also records `relays=tyo.414222.xyz:41113`.

### Event-driven Windows control-surface status foundation — 2026-08-26

- The daemon now owns `%LOCALAPPDATA%\Conduit\status.txt`. It is rewritten only on actual session
  transitions or when the phone supplies its display name; there is no watcher, polling timer, or
  telemetry loop. `conduit-daemon status` reads the snapshot on demand.
- Android now announces its system device name through the existing encrypted `PAIR_REQUEST` after
  a completed Noise handshake, using the same single sender executor as every other outbound frame.
  This adds one tiny frame per session and no idle work.
- A real Relay session reported `daemon=running`, `state=linked`, `peer_name=OnePlus 12`,
  `path=relay`, and `relay=tyo.414222.xyz:41113`. A controlled phone disconnect changed the snapshot
  to `state=disconnected`; reconnect restored the full linked snapshot and `Linked to LOG`.

### On-demand Windows control surface — 2026-08-26

- `conduit-control.exe` is a separate GUI-subsystem binary. It owns no socket, notification thread,
  tray icon, watcher, periodic refresh timer, or transport state. The window reads `status.txt` and
  `config.txt` on open and only rereads them when the user selects **Refresh**.
- The current window shows daemon/link state, phone name, path/Relay, Relay endpoint list, SOCKS5
  proxy, autostart state, Explorer integration, Save/Refresh, and diagnostics-folder access. Saving
  writes the same `config.txt` that the daemon already reads on its next start; toggles invoke the
  existing daemon integration commands rather than adding a second settings backend.
- A real launch displayed `Desktop daemon: Running`, `Link: linked`, `Phone: OnePlus 12`, and
  `Path: relay · tyo.414222.xyz:41113`, with the current Mihomo proxy and both installed integration
  checkboxes. Sending WM_CLOSE through `CloseMainWindow()` returned true and process count became
  **0**, proving the UI does not become a hidden resident process.
- Fluent visual refinement is now complete without changing that lifecycle. The raw Win32 window
  reads the Windows app theme and system accent, uses a matching DWM title bar, rounded cards,
  Segoe UI Variable typography and Common Controls v6 through a short-lived activation context.
  Sizing comes from the actual HWND DPI; at the target machine's 125% scale the prior black/unpainted
  DPI gutter disappeared and the complete 640-DIP layout rendered in an 818×729 physical window.
- An early themed prototype exposed a real Win32 re-entrancy deadlock: `Refresh` held a UI mutex while
  `BM_SETCHECK` synchronously caused the parent color callback to request that same mutex. UI metadata
  is immutable after creation, so the mutex was removed entirely in favour of read-only
  `OnceLock<Ui>`. A real Refresh after that fix stayed responsive.
- While open, the control process sampled **3 threads**, **141 handles**, and about **12.3 MB working
  set**. A normal close left **0** `conduit-control` processes and removed its temporary Common
  Controls activation manifest. The daemon remains the only resident Conduit process.

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
- During the 2026-08-26 role-aware rollout, three consecutive Android process restarts closed and
  recreated real relay sessions. Immediately before the final active session, Windows logged
  `created=4 closed=4`; the final session then came up normally. This is additional churn evidence,
  not a substitute for the 48-hour/M2 gates.

### M0/M2 sampler implemented — 2026-08-26

- `scripts/soak.ps1` now records timestamped Windows thread/handle/working-set/private-memory/TCP
  counts and Android PID/thread/FD/RSS samples, plus raw Android and daemon lifecycle logs.
- Windows now logs `session created created=N closed=M` at session creation; Android similarly logs
  `session N opened: opened=N closed=M`. This lets the sampler see the current lifecycle gap while
  a session is active instead of only learning the counters on teardown.
- `-QuiescentBaseline` controls the non-exported Android service through the existing rooted test
  environment, waits a configurable settling interval before both baseline samples, excludes those
  waits from the requested soak duration, and optionally restores the link only after evidence is
  frozen into `summary.json`.
- A short attach self-test held both platforms flat over its sample window. A separate controlled
  quiescent→connected→quiescent self-test with 10-second settling ended at Windows
  `created=5 closed=5` and Android `opened=4 closed=4`; Windows threads returned 10→10 and Android
  threads/FDs returned 19→19 / 141→141. Windows handles ended one below baseline. Small RSS/private
  memory movement remained, as expected over a seconds-long diagnostic. This validates the
  collector, **not** the 48-hour milestone.
- The sampler now identifies the physical phone by `ro.serialno` and can follow a replacement ADB
  transport with `-AllowAdbFailover`. A live test started on `127.0.0.1:15557`, deliberately
  disconnected that transport while `15556` remained online, and continued on `15556` with
  `AdbFailoverCount=1` and 100% Android sample coverage. A second quiescent failover test still
  finished at Windows `created=11 closed=11` and Android `opened=9 closed=9`. The best-effort raw
  host logcat stream correctly reported that its original transport exited; lifecycle snapshots
  in the samples/final quiescent event preserved the invariant evidence across the transport swap.
- Android FD samples now include socket, anon-inode, APK and ashmem counts. This was added after
  real network-flap testing showed that total FDs could rise even while session/socket ownership
  remained balanced; exact `/proc/<pid>/fd` multiset comparison identified newly loaded third-party
  APK splits and ashmem rather than new network sockets.

### M2 short-cycle network-flap evidence — 2026-08-26

The full M2 milestone still needs broader/longer evidence, but the first controlled campaign is
now useful rather than blocked by relay/fake-DNS failures:

- The phone's saved Wi-Fi `www` gives it `192.168.137.x`, while the Windows host is on
  `192.168.17.x`, so this is a genuine **foreign Wi-Fi → empty mDNS burst → relay fallback** test,
  not an accidental LAN success.
- Before the repair, switching Wi-Fi on/off caused `Broken pipe` against Bettbox's
  `198.18.0.137` fake relay address. TYO recorded no matching phone arrivals. A direct probe to
  `138.3.214.175:41113` from the same phone did arrive, isolating the fault to the VPN fake-IP
  mapping rather than Conduit's role-aware relay.
- The installed repair preserves hostname DNS normally and substitutes `138.3.214.175` only when
  the relay resolves into `198.18.0.0/15`. Device logs now show
  `relay DNS ... -> fake 198.18.0.137; using 138.3.214.175`, followed by a real `session up` and
  an explicit-role `legacy=false` splice at TYO.
- Six Wi-Fi↔cellular transitions across two three-cycle runs kept lifecycle counters balanced.
  One warm run finished Windows `created=17 closed=17` and Android `opened=4 closed=4`; TCP count
  returned to baseline. A later 30-second-settle run again ended at `19/19` and `6/6`.
- Total Android FDs varied during those runs, but a one-cycle exact target diff showed **zero new
  sockets and zero new anon-inodes**; its +8 total came from +5 Reddit APK/split files and +3
  ashmem descriptors, consistent with notification icon/resource loading rather than transport.
- After adding FD-class sampling, a fresh foreign-Wi-Fi→cellular cycle ended with Windows
  threads **11→10**, handles **264→261**, TCP total unchanged, Android threads **19→17**, socket
  FDs **7→7**, anon-inode FDs unchanged, and both lifecycle gaps zero. Total Android FDs were +3,
  exactly accounted for by ashmem +3. Android sample coverage was 100%.

This establishes that the reproduced handover failure is fixed and provides clean short-cycle
socket/lifecycle evidence. It does **not** replace the longer M2 campaign or the separate 48-hour
M0 LAN run.

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

### Compatible migration deployed

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

The server-first migration was executed on 2026-08-26 and all three rollout stages were observed
live: old↔old, old-phone↔new-desktop, then new↔new. The live server also demonstrated explicit
same-role stale-waiter replacement. Legacy inference stays enabled for rollback/older-client
compatibility until the old-client window is deliberately closed. Network-flap endurance remains
part of M2 rather than part of protocol deployment.

### Live-state caveats

- Restarting the legacy daemon reproduced one old-relay fresh-park refusal at
  `13:29:59.932Z`, followed by a successful session on the normal retry at `13:30:15.441Z`.
  This is consistent with the already-reproduced same-role stale-waiter bug in the id-only
  47-byte relay: a new desktop can momentarily meet a stale desktop park. The compatible relay is
  now deployed and its live same-role replacement branch has been confirmed.
- Temporary daemon logs observed during testing were stale/buffered.  Timestamp and active
  process logging must be checked before diagnosing a current session from them.

## Known gaps and evidence still required

1. **M0 endurance:** 48 hours with zero net Android/Windows resource delta and lifecycle
   counters matching the invariant. The sampler is ready; the actual 48-hour evidence is pending.
2. **M2 flap resilience:** short foreign-Wi-Fi ↔ cellular cycles now pass with balanced lifecycle
   and zero socket/anon-inode growth; extend this to a longer campaign and include hotspot/default-
   network variants before marking M2 complete.
3. **Legacy relay retirement:** remove one-second 47-byte role inference only after old clients
   are no longer expected and M2 has supplied enough real reconnection evidence.
4. **Avatar proof:** capture a real incoming Nagram XF notification carrying a contact icon.
5. **UI polish:** fix light-surface status-bar icon appearance in the Android app itself.  This
   is distinct from the already corrected monochrome foreground-service notification icon.
6. **Windows operability:** add daemon autostart at login and later a non-resident Fluent UI.

## Documentation maintenance

This progress record is not a release note.  Update its dated evidence when a test is run,
when an unresolved item is actually resolved, or when deployment status changes.  Do not mark
a milestone complete based solely on source review or a single happy-path run.
