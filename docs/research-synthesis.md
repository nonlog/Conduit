# Companion: Android <-> Windows — Architecture & v1 Scope

Device is rooted, ADB available. Play Store policy intentionally not a constraint.

## 1. v1 Scope

**Ships in v1 (LAN-only, no relay):**
- Bidirectional TEXT clipboard sync
- Bidirectional IMAGE clipboard sync (up to 10 MiB, chunked)
- Android notifications -> Windows native toasts with update + dismiss, including reply/open action forwarding
- LAN P2P discovery + direct TLS-over-TCP with Noise (no server)
- Hard lifecycle instrumentation that proves `created==closed` over 7-day soak

**Explicitly deferred:**
- **Relay / off-LAN / CGNAT traversal.** Deferred trigger: LAN v1 soaks 7 days on both platforms with `activeSessions <=1`, `fd delta 0`, `handle delta 0`, `tasksAlive` at baseline, and 100+ image round-trips without growth, plus at least one real user request for off-LAN. Relay adds WebSocket framing, queue caps, and a second transport to audit — that is the exact lifecycle surface that caused Phone Link's `libbasix-thread / pDCT / asio / ICE` leak. LAN covers >90% of desk use. Do not pay the complexity tax in v1.
- **ICE / STUN / TURN.** Deferred indefinitely. Success behind symmetric CGNAT is 10-30% without TURN; coturn is a native daemon with its own thread/socket/timer lifecycle to audit. If relay ever ships, it is a dumb byte forwarder over TLS/WebSocket on 443, not ICE. Reason: ICE reintroduces the leak the project exists to eliminate.
- **Accessibility-service clipboard fallback.** Deferred (not built in v1). Since the device is rooted, the shell-UID worker is sufficient. The accessibility heuristic (`ClipboardDetection` regex on "copy", toast `TYPE_NOTIFICATION_STATE_CHANGED`, 100ms debounce, invisible `ClipboardChangeActivity` focus steal) is fragile, flickers, and requires manual user enable. Build it only if you later target non-root devices.
- **File browsing / telephony / SMS / mirroring / media control.** Never.

Decision rationale: one transport (LAN TCP), one crypto (Noise), one discovery burst (mDNS) — all owned by a single actor per platform. Fewer codepaths = fewer leaked handles.

## 2. Wire Protocol

**Framing (both LAN and future relay, identical):**
```
[4-byte BE uint32 length N][N bytes Noise transport ciphertext]
```
Ciphertext decrypts to one `Envelope` protobuf. `N` checked before allocation: `N > 1 MiB` -> drop connection, increment `allocReject`. No newline-delimited JSON (Sefirah/KDE artifact). No `read_to_end`.

**Serialization:** `protobuf` (`prost` on Rust, `protobuf-lite`/`wire` on Kotlin). Chosen over JSON/CBOR because image bytes are native `bytes` fields (no 33% base64 bloat, no double String alloc on Android). Schema lives in `proto/companion.proto` and is the single source of truth.

**`Envelope` (outer):**
```proto
message Envelope {
  uint64 message_id = 1;   // random 64-bit, for dedup + ACK
  uint64 ack_for    = 2;   // 0 = not an ACK
  Kind   kind       = 3;
  bytes  payload    = 4;   // serialized inner message
}
enum Kind { PING=0; PONG=1; PAIR_REQUEST=2; PAIR_RESPONSE=3;
            CLIP_TEXT=10; CLIP_IMAGE_HEADER=11; CLIP_IMAGE_CHUNK=12;
            NOTIF_NEW=20; NOTIF_UPDATE=21; NOTIF_REMOVE=22; NOTIF_ACTION=23; }
```

**Inner messages:**

```proto
message PairRequest  { string device_id=1; string device_name=2; bytes static_pub=3; string version=4; }
message PairResponse { bool accept=1; bytes static_pub=2; }  // fingerprint verified OOB

message ClipText { string text=1; uint64 timestamp_ms=2; string mime="text/plain"=3; }
message ClipImageHeader { string mime=1; uint32 total_bytes=2; uint32 chunk_size=3; uint32 chunk_count=4;
                          string file_name=5; uint64 timestamp_ms=6; }  // chunk_size fixed 64 KiB
message ClipImageChunk  { uint32 index=1; bytes data=2; bytes header_id=3; } // header_id = hash of header

message NotifNew    { string key=1; string package=2; string tag=3; string group_key=4;
                      string title=5; string text=6; string timestamp_ms=7;
                      repeated TextMessage messages=8; // MessagingStyle history
                      bytes app_icon_png=9;  // only on first-seen package, else empty
                      bytes large_icon_png=10;
                      repeated NotifAction actions=11; }
message NotifUpdate { string key=1; string title=2; string text=3; } // reuses same key
message NotifRemove { string key=1; string tag=2; string package=3; }
message NotifAction { string key=1; uint32 action_index=2; string reply_text=3; } // reply if RemoteInput

message TextMessage { string sender=1; string text=2; }
message NotifActionDesc { string label=1; uint32 index=2; bool has_remote_input=3; string result_key=4; }
```

**Ids / dedup / ACK:**
- `device_id = BASE64URL(SHA256(static_pubkey))` lowercase. Stable, comparable for tiebreak.
- `message_id` random per send. Receiver keeps `LRU< (peer_id,message_id) , 1024>` dedup. Duplicate -> ACK but drop.
- Every non-ACK expects ACK within 5s or retransmit with same `message_id` (at most 2 retries). Receiver ACKs on decrypt+parse success. This replaces any polling.

**Image transfer:**
- Threshold: all images chunked. `chunk_size=64 KiB`, `chunk_count = ceil(total/64K)`. Header + chunks share same `header_id`. Chunks carry `stream_id` so a `ClipText` or `NotifNew` arriving mid-image does not head-of-line block.
- Sender streams from FD, never `readBytes()` whole file. Receiver reassembles to temp file (`%TEMP%` / `cacheDir`) then publishes to clipboard.
- Text normalized `CRLF -> LF` before send (Windows `DataPackage.SetText` compat).

**Hard caps (enforced before alloc, kill session on violation):**
- `MAX_FRAME = 1 MiB` (one Envelope ciphertext)
- `MAX_MESSAGE = 10 MiB` reassembled image (header `total_bytes` > 10 MiB -> reject)
- In-flight decode buffer per peer `8 MiB`; per-device queue `16 messages` or `32 MiB` (bounded `mpsc::channel(16)` / `Channel(16)`), per-connection buffered bytes `8 MiB`, per-device rate `10 msg/s + 5 MiB/s` token bucket
- Pipe materialization cap `32 MiB` (Android `asRegularFilePfd` cache)

## 3. Crypto and Pairing

**Chosen scheme: Noise `XX` for pairing, `IK`/`KK` thereafter.** Chosen over TLS 1.3 + pinned self-signed certs because Noise has no X.509 parsing, no CA store, no session-ticket cache, and the same framing runs over LAN TCP and future relay WS. Forward secrecy via ephemeral `e` in every handshake.

- Cipher: `Noise_XX_25519_ChaChaPoly_BLAKE2s` for pairing, `Noise_IK_25519_ChaChaPoly_BLAKE2s` (initiator knows responder static) after. Prologue = `app_version` string (e.g., `"companion/1.0"`). Rekey after `2^32` messages. Pinned exactly on both sides; mismatch -> `decrypt failed` with no debug — pin the pattern string literal in both builds and test with known vector.
- Crates/libs: `snow` (Rust, `0.10.x`), `noise-java` / `com.southernstorm:noise` (Kotlin JVM). Both configured to `ChaChaPoly/BLAKE2s` explicitly.
- Interop pitfall taken: `snow` defaults to `ChaChaPoly/BLAKE2s` but `noise-java` offers `AESGCM/SHA256` — hardcode `ChaChaPoly/BLAKE2s` on both; any one-byte prologue drift breaks handshake.

**Key storage:**
- Android: `EC P-256` static keypair generated once, stored in `EncryptedFile` under `filesDir/noise_static.bin` with `MODE_PRIVATE` (0600), backed up only via device backup exclusion. `device_id` derived as above. No AndroidKeyStore (would tie to hardware rotation).
- Windows: same keypair in `%LOCALAPPDATA%\Companion\identity.bin` with ACL `CurrentUser` only (DPAPI optional for v1.1; file ACL sufficient for v1). On cert rotation, wipe `trusted_peers.json`.

**Pairing UX:**
1. Both sides generate keypair on first run, advertise `device_id`, `device_name`, `port` in mDNS TXT.
2. User taps "Pair" on Android, shows QR: `companion://pair?v=1&id=<device_id>&pk=<base64url static_pub>&port=<port>&addrs=<ip1,ip2>` plus 8-hex fingerprint `SHA256(sorted(pubA,pubB))[0:4]`.
3. Windows scans QR (or manual IP entry), performs `XX`, displays 8-hex code. User confirms match on both screens -> both persist `peer_static_pub` in `trusted_peers.json` / `SharedPreferences`.
4. Subsequent connects use `IK` (or `KK` if both know each other) — no QR.

**Trust persistence:** `trusted_peers: Map<device_id, {static_pub, device_name, last_seen}>` on both sides. Pin is the raw static pub, not a cert. Removing trust deletes entry; next connect requires re-pair.

**Relay visibility (when later added):** Relay sees only `device_id` (hash of pub) and opaque Noise frames. It performs `id == BASE64URL(SHA256(handshake.static_pub))` check during handshake but never sees session keys or plaintext. It is untrusted transport.

## 4. Connection State Machine

**Single-flight invariant:** One `ConnectionManager` actor per process owns at most ONE `liveSession` and ONE `inFlightDial`. Any new event enqueues to the actor; actor aborts current dial before starting next. This is the fix for Phone Link's concurrent-dial leak.

```
IDLE --(NetworkCallback.onAvailable / mDNS browse result / user tap)--> RESOLVING
RESOLVING --(peer found, tcpPort known)--> CONNECTING
CONNECTING --(TCP 5s timeout ok)--> HANDSHAKING
HANDSHAKING --(Noise 5s timeout ok)--> LIVE
LIVE --(idle 60s)--> send PING, expect PONG in 10s else close
ANY --(TCP fail / Noise fail / keepalive timeout / peer close)--> BACKOFF
BACKOFF --(timer fires)--> RESOLVING
ANY --(auth fail / 5 consecutive fails / NetworkCallback.onLost / Screen OFF)--> WAITING_EXTERNAL
WAITING_EXTERNAL --(onAvailable / SCREEN_ON / new mDNS result / user action)--> RESOLVING
```

**Timeouts & backoff (jitter +-30%):**
- TCP dial `5s`, Noise handshake `5s`, read timeout `70s`, keepalive `PING @ 60s idle`, `PONG` deadline `10s`.
- Exponential backoff: `1s, 2s, 4s, 8s, 16s, 30s, 60s, 120s, cap 300s`. Reset to `1s` only after `5 min` stable `LIVE`.
- mDNS browse burst `5-10s` then `stopServiceDiscovery` / `ServiceDaemon::shutdown`. Not continuous.
- Payload ephemeral port (if ever used for bulk) timeout `30s` — close listener if peer never connects (KDE `BUG 516765`).

**Simultaneous-connect (glare) tiebreak:**
Both peers dial on mutual discovery. If outbound `CONNECTING` and inbound accept arrives: compare `local_id` vs `remote_id` lexicographically. Higher id keeps outbound, closes inbound (deterministic). First `HANDSHAKING` success wins; loser aborts and closes its socket before allocating a `Session`. Must `bind port before advertising tcpPort` (listen-before-announce), else fallback reverse UDP not needed.

**Invariants that guarantee `created==closed`:**
1. Single-flight / single-session (above).
2. Half-open TCP detection: `TCP_KEEPIDLE 30s / KEEPINTVL 10s / KEEPCNT 3` via `socket2` (`SIO_KEEPALIVE_VALS` on Windows) + app `PING`. Without this, dead peer stays `ESTABLISHED` for 2h (KDE default `7200s` -> `~7875s`; with tuning `~60s`).
3. Socket owned by cancellable scope/task, closed in `try/finally` / `Drop` on cancellation. No `GlobalScope` / no `spawn` per packet without `JoinHandle`.
4. Discovery 0/1 gauge: exactly one `DiscoveryListener` / `mdns-sd` browse, paired `start`/`stop`. `FAILURE_ALREADY_ACTIVE` / `FAILURE_MAX_LIMIT` are leak signals.
5. Bounded framing + queue caps before allocation (section 2).

**Runtime counters to expose (both platforms, scraped via logcat / tray debug page / `handle.exe`):**
`connectionsCreated` vs `connectionsClosed` (must `==` after quiesce), `activeSessions` (0|1), `activeSockets` gauge, `dialAttempts / dialSuccess / dialFail / dialCancel / dialCancelGlare`, `backoffState`, `fdCount` (`/proc/self/fd` or `Debug.getFdCount`) / `handleCount` (`GetProcessHandleCount`) + `threadCount`/`tasksAlive` + `RSS MiB`, `discoveryActive` 0|1 + `discoveryStarts/Stops` + Nsd failures, `networkCallbackRegistered` 0|1 + `onAvailable/onLost`, `queueDepth` + `bufferedBytes` + `allocReject` + `rateLimitedDrops` + `deadPeerDetected` + `keepalivePing Sent/Ack` + `dedupeHits`.

**Soak assertion:** WiFi 50 flaps + 7-day sleep cycles, `activeSessions` never 2, `fd`/`handle`/`thread` delta 0, `created-closed <= activeSessions`.

## 5. Android Implementation Plan

**Components:**
- `ClipboardSyncService : Service` — `FGS` `type=connectedDevice` (not `dataSync`; `dataSync` caps at 6h/24h on Android 15 and throws `RemoteServiceException`). Holds the single `ConnectionManager` `CoroutineScope(SupervisorJob()+Dispatchers.IO)`.
- `NotificationListener : NotificationListenerService` — thin, delegates `onNotificationPosted/Removed/Connected/Disconnected` to `NotificationFeature`. Nearly zero idle (system `ManagedServices` binds on demand, no wakelock).
- `WorkerService` (shell-UID `app_process` entry, `libcompanion_worker.so --apk=<sourceDir>`) — only clipboard reader.
- `ConnectionManager` actor — owns discovery + dial + session, enforces invariants.

**Manifest entries (exact):**
```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE"/>
<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE"/>
<uses-permission android:name="android.permission.CHANGE_NETWORK_STATE"/>
<uses-permission android:name="android.permission.ACCESS_WIFI_STATE"/>
<uses-permission android:name="android.permission.CHANGE_WIFI_MULTICAST_STATE"/>
<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>

<service android:name=".clipboard.WorkerService" android:exported="false"/>
<service android:name=".service.ClipboardSyncService"
         android:foregroundServiceType="connectedDevice"
         android:exported="false"/>
<service android:name=".notification.NotificationListener"
         android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE"
         android:exported="false">
  <intent-filter><action android:name="android.service.notification.NotificationListenerService"/></intent-filter>
</service>
```

**Clipboard read/write — verified path:**
- WRITE (`setPrimaryClip`) has no focus check on API 10-16 — always allowed. Text: `ClipData.newPlainText("Companion", text)`. Image: decode Base64 -> `cacheDir/clipboard_<uuid>.<ext>` temp file -> `FileProvider` URI `getUriForFile(..., "${appId}.fileprovider", file)` (`grantUriPermissions=true`, `@xml/file_paths` `<cache-path path="."/>`) -> `ClipData.newUri(contentResolver,"Companion image", uri)` (system grants `FLAG_GRANT_READ_URI_PERMISSION` to pasting app). This works from background.
- READ (API 29+ gate): `getPrimaryClip()` returns `null` unless focused/IME/shell. Verified gate is `clipboardAccessAllowed(uid, pkg, OP_READ_CLIPBOARD)` requiring `READ_CLIPBOARD_IN_BACKGROUND` or default IME or `isDefaultDeviceAndUidFocused` etc. `cmd appops set <pkg> READ_CLIPBOARD allow` (op 29) is **not sufficient** (needs focus check `&&` AppOps). So v1 uses Sefirah's shell-worker:
  - Build `worker` module to `libcompanion_worker.so`. `FakeContext` returns `getPackageName()="com.android.shell"`, `getOpPackageName()="com.android.shell"`, `getAttributionSource()=new AttributionSource.Builder(Process.SHELL_UID).setPackageName("com.android.shell").build()`, rewrites `ClipboardManager.mContext` via reflection (`ActivityThread.systemMain` + `getSystemContext`).
  - `WorkerService.main()` entry via `app_process`: `app_process / <lib> --apk=<sourceDir>`. Registers `ClipboardManager.addPrimaryClipChangedListener` on shell `Looper`, filters `shouldSkip` by timestamp dedup + `suppressCount`.
  - Loop prevention: `volatile suppressOutbound` / `suppressCount` integer, cleared in `finally`; dedup via `ClipDescription.getTimestamp()` to skip Android 12 double-fire.
  - Lifecycle bounded via `UidObserver` + `linkToDeath` + `HOST_GONE_RECHECK_MS=10s`; kill `-9` host must exit worker (prove in test).

**Notifications:**
- `NotificationListener.onNotificationPosted` -> `New` (create-or-update), `onNotificationRemoved` -> `Removed`. Filter `FLAG_ONGOING_EVENT | FLAG_FOREGROUND_SERVICE | FLAG_LOCAL_ONLY | FLAG_GROUP_SUMMARY` + `MediaStyle` via `EXTRA_TEMPLATE=="android.app.Notification$MediaStyle"`. Title fallback `EXTRA_TITLE -> EXTRA_TITLE_BIG`, text `EXTRA_TEXT -> EXTRA_BIG_TEXT -> EXTRA_SUB_TEXT`. MessagingStyle `EXTRA_MESSAGES` -> `sender/text`. Drop if title empty. Resend `appIcon` PNG Base64 only on first-seen `package+versionCode` (LRU 64), `largeIcon` only when non-null, downscaled to 256px.
- Android 15+ redaction is narrow: only when `NotificationAssistantService` sets `Adjustment.KEY_SENSITIVE_CONTENT=true` and flag `redact_sensitive_notifications_from_untrusted_listeners` is on, and listener is untrusted. Untrusted -> NMS sends `redacted` title `"Sensitive notification content hidden"`, blank actions, single empty `MessagingStyle`. Fix on rooted device is AppOps, not `pm grant` (signature|role):
  ```bash
  adb shell appops set <pkg> RECEIVE_SENSITIVE_NOTIFICATIONS allow
  # verify: adb shell appops get <pkg> RECEIVE_SENSITIVE_NOTIFICATIONS == allow
  # then toggle NLS off/on to refresh mTrustedListenerUids cache
  ```
  `pm grant` appears to succeed but is ignored.

**Setup commands (root/ADB, one-time):**
```bash
# 1. Grant notification access (append, do not overwrite existing colon list)
adb shell settings get secure enabled_notification_listeners
adb shell settings put secure enabled_notification_listeners '<existing>:<pkg>/.notification.NotificationListener'
# alternative helper: adb shell cmd notification allow_listener <pkg>/.notification.NotificationListener

# 2. Allow sensitive notifications (AppOps, not pm grant)
adb shell appops set <pkg> RECEIVE_SENSITIVE_NOTIFICATIONS allow
# or: su -c "cmd appops set <pkg> RECEIVE_SENSITIVE_NOTIFICATIONS allow"

# 3. Start shell clipboard worker (WorkerStarter equivalent; no Shizuku needed if root)
adb shell app_process -Djava.class.path=/data/app/<pkg>-*/base.apk \
  /system/bin com.example.companion.worker.WorkerService --apk=/data/app/<pkg>-*/base.apk
# For Shizuku variant: bind UserService via Shizuku, same entry

# 4. Exempt from Doze for testing (optional, test both whitelisted and not)
adb shell dumpsys deviceidle whitelist +<pkg>
adb shell am set-standby-bucket <pkg> active
```

**Connection triggers (no polling, no periodic scan, no long wakelock):**
- `ConnectivityManager.registerNetworkCallback(NetworkRequest TRANSPORT_WIFI, cb)` — dynamic, process-bound, throttled when cached. On `onAvailable` (trusted WiFi SSID) -> enqueue `RESOLVING` burst (`NsdManager.discoverServices` 5-10s then `stopServiceDiscovery`). On `onLost` -> abort dial, close session, `WAITING_EXTERNAL`.
- `ACTION_SCREEN_ON/OFF` via `Context.registerReceiver` (manifest blocked since 26) — gate socket: drop `LIVE` on screen off + no trusted WiFi is cheapest idle; or keep `connectedDevice FGS` socket if user prefers real-time PC->phone when screen off (settings toggle).
- `NsdManager` discovery is push/callback, but `mdnsd` holds multicast subscription; continuous browsing prevents WiFi sleep — hence burst only.
- Clipboard/notification callbacks are event-driven; each enqueues to `ConnectionManager` channel, which coalesces.

**Idle-cost story:** `NotificationListener` ~0 (no wakelock, callbacks on post/remove). `connectedDevice FGS` holds one `TCP+Noise` session only on trusted WiFi + (screen on OR user opt-in). No heartbeat in Doze (wake locks ignored, alarms deferred 9 min); TCP keepalive deferred, detection fires on next maintenance window + app `PING`. Battery-optimal vs KDE periodic broadcast (needs `MulticastLock` + radio wakeup per broadcast) and vs `dataSync FGS` (6h cap).

**Leak-proof Kotlin rules:** single `SupervisorJob` scope per `Service`, never `GlobalScope`; `Socket.use{}`, `try/finally { socket.close(); nsd.stopServiceDiscovery(l); cm.unregisterNetworkCallback(cb); clipboard.removePrimaryClipChangedListener(l) }`; `StrictMode.VmPolicy.detectLeakedClosableObjects().penaltyLog()`; counters in `FdCounters.kt`.

## 6. Windows Implementation Plan

**Daemon structure:** Single daemon process `companion-daemon.exe` (Rust, `windows-service` not needed; runs as user). Separate settings UI `companion-settings.exe` (thin `egui` or `tauri` with `WebView2` that spawns on tray double-click, exits on close — never embedded in daemon or it adds 8-15 threads + 60-120 MB RSS permanently).

Threads in daemon (bounded, provable):
- 1 message-pump thread (`HWND_MESSAGE`) handling `WM_CLIPBOARDUPDATE` + `WM_TRAYICON (WM_APP+N)` + `WM_QUERYENDSESSION` + `WM_ENDSESSION`. Tray icon (`tray-icon 0.24`) and clipboard listener (`AddClipboardFormatListener`) share this one `HWND`.
- `tokio` runtime: `Builder::new_multi_thread().worker_threads(2).max_blocking_threads(2)` (default per-core + blocking pool would violate tiny bound). Daemon is `1 peer TCP + 1 mDNS socket`; 2 workers suffice.
- No polling timer thread. No `ICE` thread.

**Chosen crates (minimal):**
- `windows = { version="0.58", features=["Win32_Foundation","Win32_UI_WindowsAndMessaging","Win32_System_SystemServices","Win32_System_DataExchange","Win32_Graphics_Gdi","Win32_UI_Shell","Win32_System_Threading","Win32_Networking_WinSock"] }`
- `tokio` (runtime, `sync::mpsc::channel(16)` bounded), `socket2` (keepalive tuning), `snow` (Noise), `prost` (protobuf), `mdns-sd 0.21` (no async runtime, `flume` channel), `tray-icon 0.24` (wraps `Shell_NotifyIconW`), `arboard` text+image helpers only for `CF_DIBV5` encode/decode (or raw `windows` `BITMAPV5HEADER` with masks `0x00ff0000/0x0000ff00/0x000000ff/0xff000000`, 32bpp `BI_BITFIELDS`, `bV5CSType = LCS_sRGB`, flip vertically for Word compat), `serde` only for settings JSON.

Why these: `clipboard-win` (74 stars) and `arboard` handle `OpenClipboard` contention retries (`ERROR_ACCESS_DENIED 0x800401D0`) — mandatory. `tray-icon` maintained (398 stars). `mdns-sd` pure Rust, no C dep like `astro-dnssd`. `snow` < `rustls` for pinned mutual auth (no X.509, no `aws-lc/ring`).

**Clipboard observe/read/write:**
- Observe: `AddClipboardFormatListener(hwndMessage)` (requires `GetMessage/DispatchMessage` pump; `HWND_MESSAGE` is leak-proof). On `WM_CLIPBOARDUPDATE` dispatch via bounded channel to `ConnectionManager` actor; coalesce bursts.
- Contention: retry `OpenClipboard` with exponential backoff `5-10 attempts, 5-10ms sleep`, then give up — don't busy-loop (Excel holds clipboard).
- Read TEXT: `Clipboard.GetContent()` / `GetText(StandardDataFormats.Text)` normalize `CRLF->LF`.
- Read IMAGE: check `StorageItems` first; validate extension via allowlist; `>2 MiB` path goes chunked anyway now (all via proto). Fallback `Bitmap` via `BitmapDecoder` -> re-encode `PNG` preserving alpha. Never eagerly `readBytes()` whole without cap.
- Write TEXT: `DataPackage.SetText`.
- Write IMAGE: `CF_DIBV5` (not `CF_HDROP`). Sefirah's `StorageItems` file-drop appears as file attachment in Word/Chrome, not pasteable bitmap. Use `CF_DIBV5` + `CFSTR_PNG` for alpha fidelity (Snipping Tool screenshots need `BITMAPV5HEADER.bV5AlphaMask`). Temp PNG under `%LOCALAPPDATA%\Companion\clip\clipboard_<uuid>.png` then `SetStorageItems` only as secondary format.
- Echo-loop guard: not `bool isInternalUpdate` with `Low` dispatch (racy, Sefirah issue #264 infinite loop). Use per-device generation counter + `DataPackage` timestamp / `PackageFamilyName` check; ignore our own `SetContent` within `500ms` window keyed by hash.

**Toast raise/update/remove — identity that is truly mandatory (verified):**
Unpackaged `CreateToastNotifierWithId(aumid)` silently drops on Win10 19041 through Win11 24H2 without a Start Menu shortcut carrying `System.AppUserModel.ID` and `SetCurrentProcessExplicitAppUserModelID(same AUMID)` called before UI. `HKCU\Software\Classes\AppUserModelId` registry-only is **not sufficient** (that's for taskbar `HostProcess`, toast platform validates `IPropertyStore` on the shortcut). Without it: no toast, no Action Center entry, no history -> no update/remove, wrong icon/name, no activation.

At install/first-run:
1. Create shortcut at `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Companion.lnk` via `IShellLinkW` + `IPropertyStore::SetValue(PKEY_AppUserModel_ID, "Company.Companion.Daemon")` (<=128 chars, no spaces, PascalCase). `AUMID` is constant forever.
2. At process start, before tray window: `SetCurrentProcessExplicitAppUserModelID(L"Company.Companion.Daemon")`.
3. If using `WinAppSDK AppNotificationManager`, bootstrap `WindowsAppRuntime` Singleton and call `Register()` before `Show`; otherwise use `windows` crate `ToastNotificationManager::CreateToastNotifierWithId` / `ToastNotificationHistory::Remove(tag,group,appId)`.
4. Toast image: `file://` or `ms-appdata:///temp/<guid>.png` local file that stays alive while toast visible (remote `http` blocked for unpackaged).
5. Update: `ToastNotifier::Update(NotificationData, tag, group)` / `AppNotificationManager::UpdateAsync(progressData, tag, group)` with `tag=notificationKey`, `group=groupKey`, `ExpiresOnReboot=true`.
6. Remove: `ToastNotificationHistory::Remove(tag,group,appId)` / `RemoveByTagAndGroupAsync`. Normalize keys (empty string vs null) or stale toasts remain.
7. Click handling without relaunch requires separate `COM LocalServer32 INotificationActivationCallback CLSID` registration — optional for v1 (v1 can relaunch `companion-settings.exe --toast-activated`); add COM only if in-process dismiss needed.

**Tray/settings:**
- Tray icon lives on the message-pump thread. Double-click spawns `companion-settings.exe` as separate process (IPC via `127.0.0.1:<port>/settings` or named pipe). Settings writes `settings.json` and signals daemon via channel; daemon never hosts `WebView2`.
- Autostart: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` string `Companion = "<exe> --daemon"` (lightest; Task Scheduler adds COM handle leak risk; Startup folder needs second shortcut duplicating AUMID).
- Single-instance: `CreateMutexW("Local\\CompanionDaemon-<user-sid>")` + `ERROR_ALREADY_EXISTS` check before creating pump window (avoid ghost `HWND`). Second instance does `FindWindow` + `PostMessage(WM_USER_SHOW_SETTINGS)` then exits.
- Shutdown: `WndProc` handles `WM_QUERYENDSESSION`/`WM_ENDSESSION` + `SetConsoleCtrlHandler(CTRL_LOGOFF_EVENT)` -> cancel `CancellationToken`, `JoinHandle::abort` with 2s timeout, `RemoveClipboardFormatListener`, `Shell_NotifyIcon(NIM_DELETE)`, `mdns ServiceDaemon::shutdown`, `mpsc` drop. No `spawn` without handle.

## 7. Repo Layout

Fresh project, no GPL reuse. Clean reimplementation, API-call granularity.

```
D:\Workspace\general\companion\
  README.md
  proto\
    companion.proto                # single source of truth, codegen both sides
  android\                         # Kotlin, Android Studio / Gradle
    app\
      src\main\
        AndroidManifest.xml
        java\com\example\companion\
          service\
            ClipboardSyncService.kt      # FGS connectedDevice, holds ConnectionManager
          notification\
            NotificationListener.kt      # thin Hilt wrapper -> NotificationFeature
            NotificationFeature.kt       # filtering, icon LRU, actions/reply
          clipboard\
            ClipboardFeature.kt          # send/receive, image chunking, suppress logic
            ClipboardChangeActivity.kt   # NOT built in v1 (deferred), stub only
            ClipboardDetection.kt        # NOT built in v1
          worker\
            WorkerStarter.kt             # builds app_process command from nativeLibraryDir
            WorkerManager.kt             # RequestWorkerLaunch broadcast, suppressNextOutbound
            ShizukuHelper.kt             # optional, not required if root
          connection\
            ConnectionManager.kt         # single actor, state machine, backoff, glare
          net\
            NsdBrowse.kt                 # discoverServices 5-10s burst + stop
            NetworkCallbackManager.kt    # register/unregister, onAvailable/onLost
          crypto\
            NoiseChannel.kt              # XX->IK, ChaChaPoly/BLAKE2s, prologue
          util\
            FdCounters.kt                # /proc/self/fd, StrictMode
            BitmapHelper.kt              # drawableToBitmap, compress
          settings\
            DevicePreferences.kt
        res\xml\
          file_paths.xml                 # <cache-path path="."/>
          clipservice.xml                # NOT in v1 manifest (deferred)
      worker\                          # ndk arm64
        src\main\java\com\example\companion\worker\
          FakeContext.java               # shell UID spoof
          WorkerService.java             # app_process entry, Looper, ClipboardListener
        src\main\jniLibs\arm64-v8a\libcompanion_worker.so  # built from worker module
      build.gradle.kts
      settings.gradle.kts
  windows\                             # Rust
    Cargo.toml                         # workspace
    daemon\
      Cargo.toml
      src\
        main.rs                        # SetCurrentProcessExplicitAppUserModelID, mutex, pump spawn
        manager.rs                     # ConnectionManager actor, state machine, single-flight
        transport.rs                   # socket2 keepalive, Noise framing, MAX_FRAME check
        mdns.rs                        # mdns-sd browse/publish, virtual-adapter filter
        clipboard.rs                   # AddClipboardFormatListener pump, CF_DIBV5, retry
        toast.rs                       # ToastNotifier + ToastHistory, AUMID, file:// image
        tray.rs                        # tray-icon, WndProc shared with clipboard
        settings.rs                    # settings.json, autostart registry, IPC
        metrics.rs                     # counters, handle/RSS gauges
      build.rs                         # embed manifest, AUMID
    settings-ui\                       # separate process, egui/tauri
      Cargo.toml
      src\main.rs
    installer\
      install.ps1                      # creates Start Menu shortcut with PKEY_AppUserModel_ID, HKCU Run
  scripts\
    adb-setup.ps1                      # appops + notification listener + worker launch
    soak-android.sh
    soak-windows.ps1
  docs\
    pairing.md
    lifecycle.md
```

**Build/toolchain:**
- Android: `Android Studio Hedgehog+`, `AGP 8.x`, `Kotlin 2.x`, `NDK r26` (for `libcompanion_worker.so`), `minSdk 29` (clipboard gate), `targetSdk 35`, `protobuf-lite` codegen via `wire`/`protobuf-gradle-plugin`.
- Windows: `Rust 1.78+`, `cargo`, `windows-rs 0.58`, `tokio 1.x`, `snow 0.10`, `prost 0.13`, `mdns-sd 0.21`, `tray-icon 0.24`, `socket2 0.5`, `prost-build` for proto, `cargo install cargo-audit`. Daemon built `release` with `lto=true, panic=abort`. Installer is `install.ps1` + optional `MSIX` later (not v1).

## 8. Risks and Open Questions

Only survivors after adversarial verification, each with cheapest experiment.

1. **mDNS virtual-adapter black hole.** Windows `GetAdaptersAddresses` advertising `vEthernet (WSL/Hyper-V)`, `Tailscale`, `VPN TAP` causes discovery success but TCP dial `21s` hang. Cheapest experiment: on Win11 24H2 with Tailscale 1.80 + WSL2, run `daemon --debug-mdns` that logs advertised IPs; verify filtering `IfOperStatusUp && IfType != SOFTWARE_LOOPBACK && FriendlyName !~ vEthernet*/WSL/Tailscale` and that browse binds to specific interface IPs (check `mdns-sd` interface selection). Success = browse result's IP dials in `<100ms` or correctly skipped.

2. **Android `NsdManager` OEM flakiness after Doze/30 min.** Some OEMs stop resolving. Cheapest experiment: on Pixel + Samsung, start `discoverServices`, force Doze via `adb shell dumpsys deviceidle force-idle`, wait `30 min`, check `onServiceLost` + re-browse burst after `NetworkCallback.onAvailable` re-registers. If `FAILURE_MAX_LIMIT=3` appears, confirm `stopServiceDiscovery` paired counter restores.

3. **Noise prologue/cipher mismatch silent failure.** `snow ChaChaPoly/BLAKE2s` vs `noise-java AESGCM/SHA256` yields `decrypt failed` with no string. Cheapest experiment: interop test in CI that does `XX` handshake with known vector (`prologue="companion/1.0"`) between Rust `snow` and Kotlin `noise-java`, asserts `transport` round-trip. One byte `prologue` change must fail.

4. **Unpackaged toast AUMID silent drop.** Without Start Menu shortcut `IPropertyStore` + `SetCurrentProcessExplicitAppUserModelID`, `CreateToastNotifierWithId` drops silently, `Update`/`Remove` have nothing to target. Cheapest experiment: on Win10 21H2 + Win11 23H2 clean VM, run `companion-daemon --toast-test` before and after `install.ps1` creates shortcut; verify `ToastNotificationHistory.GetHistory()` contains entry only after wiring.

5. **Clipboard `CF_DIBV5` alpha fidelity.** `StorageItems` file-drop not pasteable as bitmap; `CF_DIBV5` with `BITMAPV5HEADER` masks must preserve Snipping Tool alpha. Cheapest experiment: paste transparent screenshot into Word + Chrome; verify alpha not lost vs plain `CF_DIB`.

6. **Coalesced/batched `NetworkCallback` after WiFi roam.** `onAvailable` may be delayed seconds. Cheapest experiment: toggle airplane mode `50x` via `adb shell cmd connectivity airplane-mode`, assert `activeSessions` never `2`, `dialAttempts - (success+cancel+fail)==0`, and no duplicate `message_id` delivery (dedupe LRU hit).

7. **Hidden-API blacklist on newer Android for `FakeContext` reflection.** `ActivityThread.systemMain`, `ClipboardManager.mContext` field blocked. Cheapest experiment: on target device (API 34-36), run worker with `adb shell cmd hidden_api` check; if blocked, whitelist via root `settings put global hidden_api_policy 1` or use `whitelist` overlay — verify reflection succeeds or worker is dead on arrival.

## 9. First Implementable Milestone

**Milestone M0: LAN TEXT clipboard round-trip, provably leak-free, in 2 days.**

*Feature:* Copy text on Android -> appears on Windows clipboard; copy text on Windows -> appears on Android clipboard. No images, no notifications, no tray UI beyond "connected" icon.

*Path:*
1. `proto/companion.proto` with `Envelope` + `ClipText` only, `MAX_FRAME 1 MiB`.
2. Android: `FakeContext` + `WorkerService` (`libcompanion_worker.so --apk`), `ClipboardSyncService (connectedDevice FGS)`, `ConnectionManager` actor (`RESOLVING 5s -> CONNECTING 5s -> HANDSHAKING 5s -> LIVE`), `NsdBrowse` burst, `NetworkCallbackManager`, `NoiseChannel (XX)` with pinned string `Noise_XX_25519_ChaChaPoly_BLAKE2s` prologue `companion/1.0`.
3. Windows: `daemon` with single `HWND_MESSAGE` pump (`AddClipboardFormatListener` + `tray-icon`), `tokio` 2 workers, `socket2` keepalive `30/10/3`, `Noise` same pattern, `mdns-sd` advertise/browse, virtual-adapter filter.
4. Wire: `4-byte BE len + ciphertext`, dedup LRU 1024, ACK 5s, `suppressOutbound` loop prevention, `CRLF->LF` normalize.
5. Metrics endpoint: `127.0.0.1:51717/metrics` JSON with counters from section 4.

*How to test (manual, no harness):*
```powershell
# Terminal 1 - Windows soak
cargo run -p companion-daemon -- --debug-metrics
# verify: handle.exe -p companion-daemon ; (Get-Process companion-daemon).Threads.Count

# Terminal 2 - Android
adb shell am start-foreground-service -n com.example.companion/.service.ClipboardSyncService
adb shell appops get com.example.companion RECEIVE_SENSITIVE_NOTIFICATIONS  # allow (not needed for M0 but verify)
adb logcat -s Companion:* FdCounters:*
# Check: /proc/$(pidof com.example.companion)/fd | wc -l

# Test A: Android -> Windows
adb shell input text "hello-from-android-$(date +%s)"
# Expect: Windows clipboard == "hello-from-android-..." within 500ms (discovery 5s first time, <200ms thereafter)

# Test B: Windows -> Android (PowerShell)
Set-Clipboard "hello-from-windows-$(Get-Date -Format o)"
# Expect: adb shell cmd clipboard get -> same text

# Leak soak: 100 round-trips + 20 WiFi flaps
for i in 1..50 { adb shell input text "t$i"; sleep 0.5; Set-Clipboard "w$i"; sleep 0.5 }
adb shell svc wifi disable; sleep 3; adb shell svc wifi enable;  # repeat 20x via script
# Assert: metrics connectionsCreated==connectionsClosed or diff==activeSessions (0|1)
# Assert: fd delta 0, handle delta 0, tasksAlive == baseline (1 manager + 2 per peer or 0), RSS < 50 MiB
# Assert: no duplicate delivery (message_id dedupe hits == retransmits)
# Assert: kill peer via iptables DROP not RST -> deadPeerDetected within 60-90s
```

Exit criteria: 48h continuous run (with screen off/on cycles) shows `fd`/`handle`/`thread` delta 0, no `FAILURE_ALREADY_ACTIVE`, no `allocReject`, and 100% text fidelity both directions. Then add `ClipImageChunk` (64 KiB) and `NotifNew/Remove` on same framing without new threads.

**Absolute paths for M0 ownership:**
- `D:\Workspace\general\companion\proto\companion.proto`
- `D:\Workspace\general\companion\android\app\src\main\java\com\example\companion\connection\ConnectionManager.kt`
- `D:\Workspace\general\companion\android\worker\src\main\java\com\example\companion\worker\FakeContext.java`
- `D:\Workspace\general\companion\android\worker\src\main\java\com\example\companion\worker\WorkerService.java`
- `D:\Workspace\general\companion\windows\daemon\src\manager.rs`
- `D:\Workspace\general\companion\windows\daemon\src\transport.rs`
- `D:\Workspace\general\companion\windows\daemon\src\clipboard.rs`
- `D:\Workspace\general\companion\windows\daemon\src\mdns.rs`


