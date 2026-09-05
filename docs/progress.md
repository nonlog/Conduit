# Conduit development progress

> **Snapshot date:** 2026-08-26
> **Meaning of “verified”:** an observed test/device result, not an assumption inferred from
> source.  This record is intentionally more conservative than a feature checklist.

## Current position

Conduit has functioning implementation paths beyond the original pre-M0 description, but it
has **not** earned M0/M2 completion.  The central endurance requirement remains open: a
48-hour run must show no net thread, handle/FD, or session-lifecycle growth.

The latest functional implementation commit on local `master` is `62a4516`:

```text
Use MessagingStyle sender avatars
```

Local `master` remains ahead of `origin/master`; do not treat the source commits as published.
The protocol rollout itself **is deployed on the test/production path**: TYO now runs the
compatible relay, and the installed Android/Windows endpoints now send explicit roles. Legacy
47-byte inference remains enabled only as an upgrade bridge for older clients.

## Test evidence

| Area | Last recorded result | What it establishes | Limitation |
| --- | --- | --- | --- |
| Android JVM suite | **26 passed, 0 failed** | Noise transcript, frame limits, bounded/searchable history model, file/image validation including capture flags, throttled transfer-progress model, notification payload budget, wire behaviours, explicit initiator relay preamble, fake-IP relay fallback, passive multi-relay candidate/cooldown persistence, and file-publication result encoding. | No actual system-server hook, notification listener, Quick Settings host, or device radio lifecycle. |
| Windows daemon | **51 passed, 3 ignored, 0 failed** on the last full normal run | Rust transport, clipboard/image/file/toast helpers, desktop→phone outbound-file validation, file-finalisation cleanup failures, screenshot semantics, resource-bound assertions, explicit responder relay preamble, multi-relay config parsing/parking, cancel/resume receive safety across an intervening Noise send, SOCKS5 relay domain routing, autostart/Explorer helpers, event-driven control-surface status snapshots, and notification-action XML/activation parsing. | The three ignored tests exercise real Windows toasts and require interactive validation; the new action callback test was also run interactively on the target Windows machine. |
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
| Notification app icons / avatars | App icon and large-icon paths are implemented; sender `Person.icon` from public MessagingStyle messages is now the fallback when an app omits `largeIcon`. A sender-icon-only fixture matched the Windows face-cache PNG byte-for-byte. | A genuine next Nagram XF notification still needs the final opportunistic visual/path confirmation; existing private notifications are not replayed. |
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

### Direct Share desktop-name refresh — 2026-08-26

- Android's current dynamic sharing shortcut was inspected directly through
  `cmd shortcut get-shortcuts com.conduit.sync`; before the test it reported the expected
  `id=desktop`, `shortLabel=LOG`, long-lived/dynamic flags and the Conduit sharing category.
- To exercise the rename path without changing the user's Windows hostname or pairing, the resident
  daemon was restarted once with only its process-local `COMPUTERNAME=CONDUIT-RENAME-TEST`
  environment changed. The Noise identity/config remained the same. On the next relay handshake the
  phone consumed the desktop's `PAIR_REQUEST` name and the same shortcut id changed in place to
  `shortLabel=CONDUIT-RENAME-TEST`.
- The temporary daemon was then stopped and the ordinary daemon relaunched. The phone reconnected and
  the shortcut changed back to `shortLabel=LOG`. Final daemon state was again `linked` to
  `OnePlus 12` through TYO. The real Windows machine name was never modified.
- The current Conduit APK had already been reinstalled earlier in this development pass and the
  dynamic shortcut existed afterward, providing the corresponding app-update/reinstall evidence.
  A destructive identity wipe/re-pair would not exercise any additional name-publication code and
  was intentionally avoided.

### MessagingStyle sender-avatar fallback — 2026-08-26

- The pending Nagram check identified a real source-format mismatch rather than a Windows cache
  problem. A genuine Nagram X notification on the target phone exposed
  `Notification.EXTRA_MESSAGES` but `android.largeIcon=null`. The notification also referenced a
  real long-lived conversation shortcut, but Android's public `ShortcutInfo` API intentionally does
  not expose its icon to ordinary clients; no reflection or hidden-API workaround was added.
- `NotificationRelay` now reconstructs the platform `Notification.MessagingStyle.Message` records
  from `EXTRA_MESSAGES` and, when the normal large icon is absent, uses the newest sender
  `Person.icon`. The ranking snapshot is captured once per notification and reused for the existing
  silent-notification decision. There is no new thread, provider lookup, timer, AndroidX dependency,
  or polling path.
- Android build and JVM tests passed (**25/25**), and the resulting APK was installed on the target
  phone. Listener reconnect intentionally did **not** replay existing Nagram notifications, preserving
  Conduit's “no historical notification burst” behaviour.
- A temporary MessagingStyle fixture then posted a notification with **no `largeIcon`**; its only
  face was `Message.senderPerson.icon`. Windows created `faces/4e6be015c27e5126.png`, and the file's
  complete SHA-256 exactly matched the fixture's expected rasterised PNG:
  `4e6be015c27e512677d33dd72fd90f1d9f402ab4ff5cc116f64b4629ed8106c3`.
  This proves public MessagingStyle extraction, encrypted transfer and Windows face caching end to
  end without exposing real message content. The fixture app/project and test face file were removed.
- The remaining Nagram task is only the final naturally occurring real-notification confirmation.

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
  `HKCU\Software\Classes\*\shell\Conduit.SendToPhone` verb labelled **Send with Conduit**.
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
5. **Light/day visual proof:** implementation requests the correct day/night system-bar glyph mode;
   dark/night is visually confirmed, but the final unlocked day-theme check remains.
6. **Relay longevity/fleet:** long Relay+Mihomo idle/proxy-restart evidence and real additional-node
   deployment/failover remain verification/deployment work, not missing client implementation.

## Documentation maintenance

This progress record is not a release note.  Update its dated evidence when a test is run,
when an unresolved item is actually resolved, or when deployment status changes.  Do not mark
a milestone complete based solely on source review or a single happy-path run.

## 2026-08-26 reconnect recovery checkpoint

- Reproduced/diagnosed the long perceived outage as a combination of transient VPN/cellular reachability failure plus 5/10/20/.../300 s Android retry backoff.
- Added strict Windows PONG challenge semantics and bounded Android recovery backoff without introducing periodic work.
- Windows tests: 50 passed, 3 interactive toast tests ignored, 0 failed. Android JVM tests: 26 passed, 0 failed; debug APK assembled successfully.
- Installed both sides on the target pair. Controlled stable-session loss recovered to a new Relay/Noise session in about 18.7 s and status returned to linked.
- Post-recovery healthy-session check remained linked from 23:28:10 through 23:33:53 (>343 s), crossing the 240 s Relay PING boundary without a false disconnect; notification traffic still arrived at 23:33:30.

## 2026-08-26 MessagingStyle conversation-history checkpoint

- Functional commit `25db650` populates the existing `NotifNew.messages` field and extends
  `NotifUpdate` with the same bounded `TextMessage` history.
- Android reads only public `Notification.EXTRA_MESSAGES` already delivered with the posted event;
  it keeps the newest 3 non-empty records, caps sender at 80 characters and text at 320, and adds
  no query loop, provider read, timer, thread, or resident cache.
- The worst-case notification frame-budget JVM test remains green with all three bounded messages.
  Android JVM suite: **26 passed, 0 failed**; debug APK assembled successfully.
- Windows formats 2+ records as chronological `sender: text` lines inside the existing Toast body
  binding; a single record keeps the ordinary body to avoid duplication. Daemon tests:
  **51 passed, 3 interactive Toast tests ignored, 0 failed**.
- Real-device E2E used Android `cmd notification -S messaging`: the source notification had
  `android.messages=Bundle[] (3)` with Alice/Bob/Alice; after an explicit listener rebind Android
  logged `messages=3`, and the live Windows daemon decoded `pkg=com.android.shell messages=3`
  through the current TYO/Mihomo Noise session.
- Automatic non-root clipboard mirroring was removed from actionable implementation work rather
  than faked with AccessibilityService. Current Android only authorises background clipboard reads
  for focused/default-IME contexts; making Conduit the default IME solely for clipboard access would
  replace the user's normal keyboard and is deliberately deferred.
## 2026-08-27 sleep-aware reconnect observation

- After final daemon normalization, the phone entered sleep while a 60-second recovery retry was pending. No later retry ran while asleep, which is expected because the service uses `Handler.postDelayed`, not a wakeup alarm.
- Waking only to the lockscreen caused the overdue retry to execute immediately. TYO logged the phone splice at 00:36:33, Android completed Noise at 00:36:33.819, and Windows status returned to `linked`.
- This preserves the battery-first contract: Conduit does not wake a sleeping Android device merely to re-dial Relay. The pending long Relay/Mihomo verification should distinguish "asleep until natural wake" from a retry/backoff bug.
## 2026-08-27 Android + Windows UI redesign checkpoint

- Continued the existing uncommitted UI redesign without reset, clean, stash, or a replacement coding session. The working changes remain scoped to the Android Activity UI and native Windows control surface, plus this documentation checkpoint.
- Android now gives connection/route state the primary hierarchy, keeps transfers conditional on actual activity, and groups Clipboard, Privacy, Pairing, History, and Settings into quieter Material 3 sections. No new background execution path was added.
- Windows control now separates Connection, Relay Routing, and Windows Integration, with access keys and dialog-style keyboard traversal. It remains pure on-demand Win32/DWM/Common Controls with no WinUI/WebView, tray, timer, watcher, or periodic refresh.
- Android validation: `./gradlew.bat --no-daemon assembleDebug testDebugUnitTest` -> **BUILD SUCCESSFUL** (50 actionable tasks). The execution used `ANDROID_HOME`/`ANDROID_SDK_ROOT=D:\Android\Sdk`.
- Windows validation: `cargo test -p conduit-daemon` -> **51 passed, 0 failed, 3 ignored**; `cargo check -p conduit-daemon` -> success. Executed with MSVC environment variables from Scoop `portable-build-tools` without installing/upgrading any toolchains.
- Source formatting: `windows/conduit-daemon/src/bin/conduit-control.rs` was formatted individually using `rustfmt --edition 2021`. Unrelated Rust files were not formatted to avoid unnecessary workspace drift.
- Static forbidden-pattern audit found zero new timers, zero polling loops, zero scheduled workers, zero background threads, zero wake locks, and zero watchers in the UI diffs, strictly preserving the battery-first/event-driven architecture.
- Persisted design system artifact: `design-system/conduit/MASTER.md` persisted under repository root following `ui-ux-pro-max` search contract.
- Runtime deployment: the redesigned debug APK was installed in-place on the connected `CPH2573` test phone with `adb install -r`, then `com.conduit.sync/.MainActivity` was started for direct user inspection. No phone screenshot or screen capture was taken.
- Runtime launch confirmation: the redesigned native Windows control surface was built, launched, and confirmed responsive as a real top-level `Conduit Control` window. Independent visual inspection was not completed because the desktop-observation connector is currently blocked by caller-identity validation; do not treat process launch alone as visual approval. NO unlocked-phone foreground screenshot was captured or opened. Zero commits or pushes created.

## 2026-08-27 UI redesign v2 — rejected UI removed

- Real-device review rejected the previous redesign. That iteration is retained above only as history and is superseded by this v2 checkpoint.
- Reworked the project-specific design system away from dashboard/marketing patterns and toward native system-tool conventions: concise labels, Material 3 dynamic theming on Android, Windows 11/Fluent hierarchy on desktop, standard density, almost no explanatory copy.
- Android home now consists of a compact neutral connection row, conditional active-transfer UI, and two settings rows. Removed the large colored hero, `Phone companion · quiet idle`, the screenshot/clipboard workflow callout, desktop fingerprint, `This phone's identity`, phone fingerprint copy action, Pairing & Security section, and other helper paragraphs.
- Clipboard History now handles Android system Back with `BackHandler` and returns its local `page` state to `home` instead of letting Back immediately finish `MainActivity`. History filler such as `tap to copy` and the redundant overview line was removed.
- Windows control changed from three large full-width stacked cards to a 760×560 two-pane utility layout. The left pane is connection/status plus Diagnostics/Refresh; the right pane contains Relay fields and Windows integration toggles, with Save at bottom-right. Long operational explanations were removed.
- Replaced the rejected desktop chain logo with the Android app's actual visual identity. This was later superseded by the static multi-resolution Android-source icon assets documented below; do not restore the earlier runtime GDI app-icon rasteriser. Relay retains its three-node mark and Windows its four-pane mark.
- Fixed the clipped peer name on the left Connection panel: the peer now uses an 18pt semibold font in a 126×58 DIP area rather than the previous 24pt/122×38 DIP box. Runtime child-control inspection reports `PeerText=OnePlus 12`, width 126, height 58, with nonzero small/big window icon handles.
- Android final verification: `assembleDebug testDebugUnitTest` -> **BUILD SUCCESSFUL** (50 actionable tasks; 9 executed, 41 up-to-date). The final debug APK installed successfully with `adb install -r` and MainActivity was started. The phone was locked/dozing during final automation, so no unlocked foreground screenshot or gesture recording was performed.
- Windows final verification: changed source is `rustfmt --check` clean; daemon suite **51 passed, 0 failed, 3 ignored**; `cargo check -p conduit-daemon` succeeded; `git diff --check` succeeded.
- Low-power/static audit found no new Android timer/poll/thread/scheduled-work/wake mechanism and no Windows timer/poll/thread/watcher implementation. No WinUI, WebView, tray process, or resident control UI was added.
- Desktop-observation remains blocked by `CALLER_IDENTITY_REQUIRED`; the assistant therefore does not claim final visual approval of the Windows surface. No commit or push was made.

## 2026-08-27 Windows clipboard + notification identity repair

- Removed the unrequested foreground-testing rule that had been added to `AGENTS.md`; the file is back to its pre-turn content.
- Fixed Windows image clipboard sync at the actual failure seam. Image getters are now called inside bounded `with_clipboard_attempts` after the event-driven clipboard listener fires. Supported order is registered PNG -> DIBV5 -> DIB -> CF_BITMAP; legacy CF_BITMAP's complete BMP bytes use the BMP-capable converter rather than the DIB-only converter. Remote image clipboard writes now publish actual CF_DIB bytes.
- Physical-device proof: Windows saw a 518,536 B DIBV5, emitted a 3,367 B PNG, and OnePlus 12 logged both `image in` and `clip image in` at 3,367 B; `cache/clip.png` was present at the same size. This closes the Windows screenshot/image clipboard -> phone path without polling or a wake mechanism.
- Replaced the fuzzy runtime GDI app-logo reconstruction with generated static assets sourced from the exact Android launcher vector/gradient. `tools/generate_icon.py` emits a 512 px PNG and a 9-size ICO (16 through 256 px); `conduit-control.exe` loads the ICO sizes directly for native Windows icon slots.
- Added Windows notification application identity: `%LOCALAPPDATA%\Conduit\conduit-icon.png` is persisted from the same asset and registered as the AUMID `IconUri` with `IconBackgroundColor=FF2F6FE0`. Live registry inspection confirms all values.
- Promoted the mirrored Android source app label from tiny Toast attribution text to `hint-style="body"`. The new unit test verifies that `placement="attribution"` is absent.
- A controlled real Android notification traversed OnePlus -> Relay -> Windows after the changes (`notif out com.android.shell` / `notif in app=Shell`) with no toast failure, then the temporary source notification was snoozed away.
- Windows suite after this checkpoint: **52 passed, 0 failed, 3 ignored**; `cargo check`, all-bin build and `git diff --check` passed. Final daemon was relaunched outside the AgentDock command job via `Win32_Process.Create`; PID 35832 / Session 2 / parent `WmiPrvSE`, responding and `linked` through TYO. The earlier temporary launch task was deleted and no scheduled task remains. No commit or push.

## 2026-08-27 Windows app packaging / identity hardening

- Added a real per-user Windows install layout. User entry is `%LOCALAPPDATA%\Programs\Conduit\Conduit.exe`; hidden daemon and send helper remain internal siblings. Start menu now contains `Conduit.lnk` with `System.AppUserModel.ID=Conduit.Desktop` and the shared ICO.
- Converted the daemon to Windows GUI subsystem and changed HKCU Run to launch it directly. Verified `PE Subsystem=2`, no daemon console, and one linked hidden daemon process. Launching `Conduit.exe` starts the daemon on demand if absent and closing the GUI does not stop it.
- Fixed DPI icon softness by generating 8x-supersampled dedicated ICO sizes and loading exact monitor-pixel HICONs for both 34-DIP and 44-DIP marks instead of stretching one 48px handle.
- Rebuilt only the stale Conduit notification identity cache after the correct shortcut existed. Notification-center pixel verification found a 20x20 Conduit violet/blue header icon cluster; closed-panel control had zero matching pixels. Temporary screenshots/test artifacts were removed.
- Final Windows suite remains **52 passed / 0 failed / 3 ignored** and release all-bin build plus `git diff --check` passed. No commit or push.

## 2026-08-27 release-candidate integration checkpoint

- Desktop name bug fixed: Windows now reads `ActiveComputerName` instead of assuming the process
  inherited `COMPUTERNAME`. With that environment variable deliberately removed, the daemon still
  advertised `LOG`; the real phone persisted `peer-name.txt = LOG` after reconnect.
- US / TYO / WA are all deployed and TCP-reachable on 41113. Windows parks all three responders.
  Android keeps one active Relay and `RelayQualityStore` v2 selects passively from real dial/session
  success, failure streak/cooldown, unstable sessions, real transfer goodput and real session-up
  EWMA. Forced reconnect evidence created independent records for all three nodes.
- Explorer integration now renders `Send with Conduit` with the Conduit icon and installed helper
  path. The Windows 11 context menu was visually checked on a real file.
- Optional tray integration is daemon-owned and event-driven. Settings persists `tray_icon`; when
  disabled, no tray thread is created. Its context menu is intentionally icon-free and contains
  `Open Conduit` plus `Exit Conduit`; Exit was verified to stop the daemon.
- Android/Windows product artwork now shares the Fluent `Phone Desktop` geometry. Windows uses a
  multi-resolution coloured application asset and dedicated monochrome regular tray glyphs.
- Final pre-release automated verification: Android **27 passed / 0 failed**; Windows **53 passed /
  0 failed / 3 ignored**; `cargo check`, release all-bin build, and `git diff --check` passed.
- No unlocked-phone foreground screenshot was captured during this checkpoint.
## 2026-08-27 v0.1.0 published and installed from `www`

- Pushed Conduit release commit `e55b17c` and tag `v0.1.0`; GitHub Release includes the Windows x64
  Scoop package and current debug-signed Android APK.
- Added `bucket/conduit.json` to `nonlog/scoop-www` in `b752b5d`, then installed `www/conduit 0.1.0`.
- Verified Scoop `current`, HKCU Run, Start Menu AUMID shortcut, and Explorer verb all reference
  `D:\Programs\Scoop\apps\conduit\current`. Removed the obsolete manual program directory only after
  those checks; `%LOCALAPPDATA%\Conduit` user data remains intact.
- Final installed daemon is responsive and linked to OnePlus 12; observed route after install was US.
### 2026-08-27 — Sefirah-reference UI pass 1

- Inspected the current Sefirah desktop two-pane device-control layout and Sefirah-Android device-first Material 3 home hierarchy.
- Reworked Conduit Android Home into device card -> active transfers -> grouped settings, reducing section noise and keeping the real peer/link state first.
- Reworked native Windows control UI into a wider device-left/settings-right split with compact Relay/Windows cards and no decorative section glyphs.
- Validation: Android build/unit tests successful and installed to the OnePlus test device; Windows 53 passed, 0 failed, 3 ignored, `cargo check` and release control build successful.
- No unlocked-phone foreground screenshot was captured during this Sefirah-reference UI refactor.

## 2026-08-28 share-target, device-name casing, and webpage handoff

- Android Direct Share no longer reuses the adaptive launcher resource. The `Log` shortcut now uses dedicated `drawable/ic_share_target`, whose phone/desktop foreground is sized to match the normal Conduit app mark in the chooser rather than being over-zoomed.
- The in-app device tile now uses dedicated `ic_phone_desktop` without the launcher's 68% safe-zone inset. Its Compose box is 36 dp inside the existing 56 dp tile, producing a substantially larger visible glyph while retaining normal padding.
- Windows device naming now prefers TCP/IP `Hostname`, which preserves the casing configured in Windows Settings (`Log`), and falls back to `ActiveComputerName` only when needed. A real encrypted reconnect updated the phone's persisted `peer-name.txt` to `Log`; the daemon also logged mDNS advertising with `host=Log`.
- Webpage shares are now a distinct `SHARED_URL` wire payload instead of clipboard text. Android accepts only bounded `http`/`https` URLs, carries the source page title/device name, and Windows validates again before showing a native Conduit toast with `Open in New Tab` protocol activation.
- Chromium's `Send Tab To Self` / `Send to your devices` is a Chrome Sync-internal component, not a public third-party API. Conduit therefore does not alter Chrome profile/sync state; Windows hands the URL to the registered browser. On this machine both HTTP and HTTPS are registered to `ChromeHTML`, so the action opens Chrome.
- End-to-end test with `https://rmpc.mierak.dev/`: desktop log recorded `shared URL in` followed by `shared URL toast shown`; Windows clipboard sequence remained unchanged. Temporary ADB reverse was removed afterwards and the production daemon returned to Relay automatically.
- Validation: Android unit suite is 29 passed / 0 failed; Windows is 54 passed / 0 failed / 3 ignored, plus `cargo check` and release daemon build. No unlocked-phone foreground screenshot was captured.
## 2026-08-28 live settings apply + Shared Links history checkpoint

- Windows Relay/proxy/tray settings now apply to the already-running daemon through the existing local named pipe. Saving from `Conduit.exe` no longer shows the former "Restart the desktop daemon" success modal.
- Relay configuration is rebuilt in place: old relay parking workers are cancelled and replaced with the new endpoint/proxy set. A live Relay session is ended only when routing settings actually change so the phone can reconnect through the new route; a live LAN session is left alone.
- The optional tray icon can be disabled or re-enabled in place. No daemon restart, config watcher, polling loop or periodic timer was added.
- Same-process proof: tray toggle and Relay reorder/restore all completed with daemon PID `25060` unchanged. Daemon logs explicitly recorded `tray icon disabled/enabled without daemon restart` and `relay configuration applied without daemon restart`.
- Phone -> Windows web shares now also persist a bounded desktop history at `%LOCALAPPDATA%\Conduit\shared-links.tsv`: newest first, maximum 100 entries, de-duplicated by URL, with URL/title/source-device/timestamp. Unsafe/non-web schemes are refused.
- The on-demand native Windows control adds a `Shared links` list in the device pane with selected-URL detail, `Open`, double-click open, and `Clear`. Opening delegates to the Windows default browser; clearing asks for confirmation. The list is read only when the control surface refreshes/opens, so it adds no resident watcher or timer.
- Real-device validation: OnePlus 12 shared `https://rmpc.mierak.dev/`; the daemon logged both `shared URL in` and `shared URL toast shown`, and the history file was created with that URL and source device. The synthetic ADB test title was truncated by shell argument quoting, not by the history format.
- Windows validation: daemon suite **56 passed / 0 failed / 3 ignored**; the control/shared-history module adds **2 passed / 0 failed**; `cargo check`, release all-bin build, rustfmt for changed files, and `git diff --check` pass.
- The development release binaries were overlaid onto the current Scoop install for validation. Final detached daemon PID is `9172`; the phone automatically recovered to normal Relay via `tyo.414222.xyz:41113`. Temporary ADB reverse was removed.
- Automated visible-window inspection of the final control layout was unavailable from the current non-interactive execution session; this is an automation-session limitation, not a functional failure. Build/data-path validation is complete.
- No unlocked-phone foreground screenshot was captured during this task.
## 2026-08-28 Sefirah-structure UI overhaul

- Replaced the previous subtle Sefirah-inspired pass with a structural overhaul based on the actual Sefirah shells/components.
- Android now mirrors Sefirah-Android's main information architecture: persistent Home / Devices / Settings destinations, a Sefirah-style 56 dp circular device card with a compact sync toggle, device controls on Home, a dedicated Devices page, and grouped Settings cards. Clipboard History remains a real child destination with Back returning to the current main shell.
- Windows now mirrors Sefirah desktop's main shell geometry: a persistent left device control centre with a phone-frame silhouette, a right-side top navigation strip, and a rounded layered content surface. Shared Links and Settings are separate top-level pages; Relay and Windows integration live under Settings.
- The Windows implementation remains native Win32/DWM/Common Controls and on-demand; the Android implementation remains Material 3. No WebView, WinUI runtime, polling UI, timer, watcher, or background navigation process was introduced.
- Validation: Android `assembleDebug + testDebugUnitTest` succeeded and the APK was installed on the OnePlus test device. Windows `cargo test` passed 56/0/3 ignored plus 2/0 shared-link control tests; `cargo check`, release all-bin build, source rustfmt, and `git diff --check` passed.
- The Scoop-installed `Conduit.exe` is overlaid with this development UI for direct review. No commit or push was made.
- No unlocked-phone foreground screenshot was captured during this UI overhaul.

## 2026-08-28 desktop notification/history + caption controls checkpoint

- Replaced the decorative Windows `Notifications` placeholder with a real bounded local history. The daemon now writes `data\notifications.tsv` only when `NotifNew` / `NotifUpdate` events arrive; entries retain timestamp, notification key, package, source-app name, title and body, are newest-first, de-duplicated by notification key, sanitised, and capped at 100. No polling loop or periodic timer was added.
- The WPF left pane now renders notification cards with the cached Android source-app icon, source application name, age, title and body. `Clear all` is a real action that deletes this local history and immediately updates the open UI. App icons reuse the daemon's existing bounded `<data>\icons` cache. While the control window is open, one kernel-backed `FileSystemWatcher` listens only for `notifications.tsv` changes and is disposed on window close, so new cards appear immediately without Refresh and without a polling timer or resident background UI.
- Mirrored Windows toast identity was corrected at the same seam: the Android application icon now owns `appLogoOverride` when available instead of being replaced by a contact avatar; the source app is a subtle line above the notification title, while the actual notification title uses toast `title` styling.
- Replaced fragile private-font caption glyphs with normal visible `−`, `□`/`❐`, and `×` controls. UI Automation exercised Maximize -> Restore -> Minimize -> Restore -> Close successfully (`showCmd` 3 -> 1 -> 2 and process exit on Close). A full PC-only PrintWindow capture also shows all three caption controls.
- WPF is explicitly Per-Monitor-V2 DPI aware. The custom text-box template no longer double-applies its content margin, so the SOCKS5 value is visible, and the current text layout/render settings are tuned for the machine's 125% display scaling.
- End-to-end live proof: a real Android `com.android.shell` notification traversed the existing OnePlus 12 -> TYO Relay -> Windows path. `notifications.tsv` recorded `Shell / Conduit history E2E / Notification history is now live`, the package icon was persisted in the expected content-addressed icon cache, and the WPF notification card displayed the source icon/name/title/body. The test history row was cleared through the actual UI action afterward and the temporary Android test notifications were snoozed.
- Verification: full daemon suite **59 passed / 0 failed / 3 ignored** after adding notification-history tests; the later toast identity refinements passed their targeted tests, `cargo check`, release daemon build, `dotnet build`, WPF single-file publish and `git diff --check`. The installed Scoop development binaries were overlaid in place. Final daemon PID `25580` is responsive and linked to `OnePlus 12` via `tyo.414222.xyz:41113`.
- No unlocked-phone foreground screenshot was captured or opened. Desktop-only captures were used for WPF visual verification.

## 2026-08-28 Explorer file-send false failure + desktop UI hardening

- Fixed the Windows Explorer `Send with Conduit` false-failure path. `conduit-send.exe` no longer inherits Explorer's missing/invalid standard handles; it captures the child command's stdout/stderr and preserves the daemon's real failure reason instead of replacing every error with the generic "make sure the daemon is linked" message.
- The daemon's `send` command now treats its success text as best-effort output. A GUI-subsystem command can therefore finish successfully after the phone confirms publication even when no stdout handle exists; a post-transfer `println!` failure can no longer turn a completed transfer into a reported failure.
- Explorer helper retry is limited to pre-request local-control-pipe availability. Session/file/publication failures are not blindly retried, avoiding duplicate transfers. If the helper genuinely fails it uses a Conduit Windows toast, with the WPF control window's inline error banner as fallback; the old Win32 `MessageBox` path and the attempted `TaskDialog` fallback are both absent.
- Real installed-path proof over the production TYO Relay: launching `D:\Programs\Scoop\apps\conduit\current\conduit-send.exe` against a temporary file exited `0` in about 1.1 s, the exact file appeared in OnePlus 12 `/sdcard/Download`, and both test copies were removed afterward. A missing-file negative test exited `1` in about 0.25 s without leaving a helper process or opening a blocking legacy dialog.
- Desktop UI received a broader Sefirah-aligned cleanup rather than only the reported controls: native Windows non-client title bar/caption buttons, Segoe UI Variable + ClearType text, stable integer type sizes, compact `Relay · TYO` route text, a real Send-file device action, cleaned left-pane actions, modern inline status/error banners, and an in-window shared-link clear confirmation. All WPF `MessageBox` use was removed. Settings retains visible Relay/SOCKS5 fields; PC-only visual verification showed the proxy value `socks5://127.0.0.1:7891` clearly rendered.
- Verification: daemon suite **59 passed / 0 failed / 3 ignored**, send-helper regressions **2 passed / 0 failed**, `cargo check`, release daemon/helper builds, WPF Release build/publish, `cargo fmt --check`, and `git diff --check` pass. The development binaries are overlaid into the current Scoop install; the daemon is linked to OnePlus 12 via `tyo.414222.xyz:41113`.
- No unlocked-phone foreground screenshot was captured or opened; visual checks used only the Windows desktop window.

## 2026-08-28 desktop UI framework replacement — Uno / WinUI 3

- The previous WPF/Win32-lookalike desktop control surface is superseded. `windows/conduit-ui` was rebuilt as a real **Uno Platform + WinUI 3 / Windows App SDK** application, matching Sefirah's desktop technology family instead of imitating its visuals through WPF templates.
- Current desktop project stack: .NET 10, `Uno.Sdk 6.8.0-dev.21`, `Microsoft.WindowsAppSDK 2.0.1`, Uno Toolkit, CommunityToolkit WinUI SettingsControls `8.2.251219`, Uno Fluent fonts, MVVM. The resident backend remains the Rust `conduit-daemon`; the UI remains on-demand.
- The shell now follows Sefirah's structure with a 360-DIP persistent device/notification pane, top WinUI `NavigationView`, native Windows caption buttons, WinUI/Mica backdrop, rounded layered content surface, and a secondary Settings `NavigationView` using real `SettingsCard` controls.
- Notification history now uses native WinUI `ScrollViewer + ItemsRepeater` and WinUI theme resources. There is no WPF scrollbar/template. Notification cards reuse the daemon's cached Android source-app icon and show source app, age, title, and body. While the UI is open, one event-driven `FileSystemWatcher` updates status/history; it is disposed on window close and adds no polling timer.
- The missing Windows notification attribution icon was traced to a missing Start Menu shortcut. `install-windows.ps1` was rerun for the Scoop install, recreating `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Conduit.lnk` with `System.AppUserModel.ID=Conduit.Desktop`, the installed `Conduit.exe` target, and `conduit-icon.ico`. The registry AUMID identity still points to the installed Conduit PNG.
- Desktop-only visual proof after the identity repair shows the Windows Notification Center group header as **Conduit with the purple Conduit app icon**. The synthetic verification toast was removed afterward. No phone screenshot was captured or opened.
- The current Scoop install was overlaid with the new WinUI publish while preserving its persistent `data` junction. The detached daemon restarted successfully and is linked to OnePlus 12 over Relay.
- Installed-path file-transfer proof after the UI migration: `conduit-send.exe` exited 0 in ~0.8 s; the exact temporary file appeared in OnePlus 12 `/sdcard/Download`; both temporary copies were then removed.
- Verification: Uno Release restore/publish succeeds with 0 build errors; full daemon suite **59 passed / 0 failed / 3 ignored**, send-helper regressions **2 passed / 0 failed**, `cargo fmt --check`, and `git diff --check` pass. No commit or push was made in this checkpoint.

### Sefirah-parity WinUI visual closure — 2026-08-29

- The Windows control surface now uses Sefirah's actual Uno/WinUI layout conventions rather than a WPF approximation: 32-DIP custom title bar, 360-DIP device/notification pane, top `NavigationView`, compact Settings rail, CommunityToolkit `SettingsCard` rows, Mica/theme resources, and native caption controls.
- General Settings now gets the full right-hand content width at the normal 1280x760 window size. Relay and SOCKS5 values render completely; horizontal scrolling is disabled. Received-file location is user-selectable, while the displayed Conduit data path is `D:\Programs\Scoop\apps\conduit\current\data` and remains backed by the Scoop persist junction.
- Shared Links rows open directly on click and the redundant bottom selected-link/Open controls were removed. Notification history continues to render the real Android source application icon and uses the native WinUI scrolling surface.
- The latest self-contained WinUI build was published and overlaid onto the installed Scoop app without restarting the resident daemon or disturbing persistent data. The live session remained linked to OnePlus 12 over WA Relay.
- Installed `conduit-send.exe` was re-tested end-to-end: exit `0` in about 1.0 s, exact temporary file confirmed in Android Downloads, then local and remote test files removed.
- Regression: daemon **59 passed / 0 failed / 3 ignored**, send helper **2/2 passed**, `cargo fmt --check`, `git diff --check`, Release build and WinUI self-contained publish all pass. No commit or push was made.

### GitHub build authority + Log-only integration gate — 2026-08-30

- Added a clean-run GitHub Actions pipeline for Android, Windows x64, and Linux x64 Relay. Toolchain versions and Uno/NuGet inputs are pinned in the repository; local `.tools`, Scoop contents and Log SDK caches are not build inputs.
- First-run failures were used to remove hidden assumptions (`yes | sdkmanager` under `pipefail`, and Debug Uno restore reused by Release publish). Run `33319898971` on commit `f7830fe` subsequently passed Android, Windows Rust + Uno/WinUI, and Relay jobs on GitHub-hosted runners.
- Added GitHub artifact packaging/version validation and a tag release path compatible with the existing `www` Scoop manifest. Tag releases publish both aggregate checksums and a per-Windows-zip `.sha256`; the current bucket may continue computing the hash automatically or consume the published hash later.
- Added `scripts/install-github-build.ps1` for Log. It downloads a successful Actions build (optionally through Log's Mihomo proxy), installs the Windows package into the existing Scoop path without touching the persisted `data` junction, refreshes Windows integration, detaches the installed daemon from AgentDock's invoking command job through `Win32_Process.Create`, and installs the APK through ADB. It does not compile anything.
- Log verification used run `33319898971` artifacts only: Android 0.1.0 installed, WinUI launched/responded, `data` remained linked to `D:\Programs\Scoop\persist\conduit\data`, the installed daemon linked to OnePlus 12 over WA Relay, and installed `conduit-send.exe` delivered a temporary file to Android Downloads with cleanup afterward.
- Final operating model: GitHub is the mandatory development/build gate; Log remains the mandatory install/Scoop-update/real-device/Windows-integration test gate.

### Android Home merge + Windows launch/icon optimization — 2026-09-01

- Merged Android Home/Devices into a single Home surface; bottom navigation is now Home + Settings. Clipboard home card shows the latest preview, count, age, and explicit send/receive direction icon instead of only `Clipboard <count>`.
- Deferred Windows status/config/history initialization until after the first WinUI frame and enabled ReadyToRun for the self-contained desktop publish. Tray Open now reuses an existing current WinUI window.
- Rebuilt Windows icon generation around size-specific Fluent vectors; GitHub CI regenerates assets before compilation, and tray ICOs include native frames through 64px. Log runs at 125% DPI and now selects 20x20 directly.
- GitHub clean run `33412249555` is green across Android, Relay, Windows Rust, Uno/WinUI ReadyToRun publish, packaging and artifacts. No Log-local build was used.
- After installing those artifacts on Log, UIAutomator confirmed Devices is gone and the latest clipboard preview plus `Sent to desktop` semantics are present. Real daemon-tray cold-open timing over five launches averaged **567.5 ms** (509.1-665.1 ms), and repeated tray Open with the UI already running kept one process.
- Installed-path `conduit-send.exe` regression: exit 0 in 720 ms, exact file confirmed in Android Downloads, both test copies cleaned; daemon stayed linked to OnePlus 12 via US Relay.

## 2026-09-01 UI/UX polish — managed Relay/proxy, monoline identity, transfer progress

- Windows identity is being simplified to a transparent single-colour Fluent phone/desktop mark instead of the previous purple rounded-square tile. The CI icon renderer keeps native 16/20/24+ frames, while the notification-area icon remains a transparent black/white glyph chosen from Windows light/dark theme. The Start Menu shortcut, notification attribution identity and Explorer `Send with Conduit` verb all consume the same high-resolution generated icon family.
- The Windows General page no longer exposes the raw `relays=` string as a single-line text box. It presents four managed Relay points (US, WA, TYO, JP) with individual selection controls. Production DNS aliases are `conduit-us.414222.xyz`, `conduit-wa.414222.xyz`, `conduit-tyo.414222.xyz`, and `conduit-jp.414222.xyz`, all on port 41113; the legacy hostnames stay valid behind those aliases.
- Relay proxy UX is now explicit: `System proxy`, `Manual SOCKS5`, or `Direct`. `relay_proxy=system` resolves the enabled Windows Internet Settings proxy at daemon reload/start and uses a SOCKS/SOCKS5 endpoint; a manual value remains backward compatible. LAN sessions continue to stay direct.
- Explorer file send is being upgraded from final-error-only feedback to event-driven transfer progress. The resident daemon forwards bounded real byte progress over the existing named pipe, and the tiny `conduit-send.exe` helper owns one replace-in-place Windows progress toast. Android already had byte progress internally; completion/failure now remains visible briefly instead of immediately cancelling the transfer notification, so fast transfers still leave user-visible feedback.
- The GitHub contributors REST endpoint was checked before changing history: the live default-branch contributors are `codex`, `claude`, and `nonlog`; there is currently no `claude[bot]` contributor or `claude[bot]` commit on `master`. The screenshot entry is therefore treated as stale GitHub contributor UI/cache, not a reason to rewrite valid repository history.
- This checkpoint is intentionally pre-validation. Clean GitHub Actions, GitHub artifact installation on Log, Relay/proxy migration, desktop/Android notification verification, and final measurements are still required before this section is marked complete.

## 2026-09-05 explicit device pairing and management

- Added a real one-phone/one-desktop trust lifecycle instead of treating the first successful handshake as an implicit permanent pair. Android and Windows now expose Pair/Pair new, Cancel pairing, and Forget; disconnect remains a temporary transport action and does not erase trust.
- Windows persists the authenticated phone id/name beside its existing identity. Android keeps its existing `peer.txt`/`peer-name.txt`. Forget removes only the relationship and share target; each side's own Noise `identity.bin`, settings, histories, and Scoop-persist data remain intact. `pairing-v2` prevents a forgotten legacy phone from being silently re-admitted.
- Real-device network inspection showed the OnePlus and LOG are normally on different private subnets (`192.168.1.x` vs `192.168.17.x`), so LAN-only onboarding would fail in the actual deployment. Pairing therefore uses a temporary six-digit code by default. The code hashes to a two-minute Relay rendezvous using the existing opaque relay protocol; no Relay server change or permanent pairing service was added. Same-LAN mDNS remains a fallback.
- Both sides require an authenticated mutual identity hello after Noise XX and before the session becomes application-visible. A new peer must be in an explicit pairing window; the announced device id must match the Noise static key. Normal reconnects continue to accept only the pinned peer id.
- Pairing adds no permanent polling. Windows creates temporary Relay parking tasks only for the two-minute user-opened window; Android's pairing retries also exist only inside that window. Normal battery-first reconnect/parking behavior is unchanged.
- Cross-language pairing-code tests pin `123456` to rendezvous `3sLbGZON6YWYSIrLdCIGl7TWmbRLGLVRBqCwooefYBY`. GitHub Actions run `33893777102` passes Android tests/APK, Windows daemon tests/Rust build/Uno publish/package, and Relay tests/build.
- The latest branch artifacts were installed on LOG and OnePlus without changing either identity, remembered peer, config, or the Scoop `data` junction. The upgraded pair continues to establish trusted LAN/Relay sessions. Automated interaction with Android's pairing dialog is restricted in the current control environment, so the final code-entry/Forget/re-pair UI flow remains a manual visual/function check rather than claimed automated proof. No unlocked-phone screenshot was captured.
