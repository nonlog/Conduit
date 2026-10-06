# Conduit handoff

**Prepared:** 2026-10-06
**Repository:** `D:\Workspace\Conduit`  
**Branch:** `codex/non-lan-relay-m3-20261002`  
**Remote:** `https://github.com/nonlog/Conduit.git`

> **Maintenance rule:** keep this handoff current during active development, not only when a
> conversation is ending. Update it after a major implementation milestone, production/runtime
> change, important verification result, root-cause discovery, or change to the recommended next
> step. A new session should be able to resume safely from this file plus the linked docs even if
> the previous conversation ended abruptly.

## Settings launch incident — 2026-10-06

- The user confirms **Settings still cannot open**. The prior user resolution was for the
  boot loop; closing every recovery item, including Settings, was incorrect. Boot-loop recovery
  stays closed, and Settings is tracked separately. No new reboot/root-module change was made.
- Fresh launches at 16:23 reproduced `Resources$NotFoundException: String resource ID
  #0x7f120e0e` in `TopLevelSmartServicePreferenceController.getCustomizedTitle`. Its decompiled
  implementation reads the AI/Smart Service activity's **external package** resource metadata,
  not a Settings string. Earlier overlay/Settings-APK guesses are not established causes.
- A read-only framework probe identifies **`com.oplus.pantanal.ums` / AISettingActivity**:
  PackageManager supplies `homepage_title_multiple=0x7f120e0e`, which is absent from its current
  resources. The currently mounted, signed **UMS 16.59.6** APK's actual manifest declares
  **0x7f120596**, and that resource resolves correctly to `string/title_content`. Package
  registration and the current APK demonstrably disagree. The source of the mismatch is not
  established; do not blame an unrelated module without evidence.
- Settings APK `/system_ext/priv-app/Settings/Settings.apk` SHA-256 is
  `621D6063FC5B071D5E1FE81B89F56BB1B178DE0A2EAB643B25FD55325461883D`, identical to the Oct 5
  preserved copy. Session tool-call audit found Conduit notification-access and temporary
  accessibility secure-setting writes, but no Settings APK replacement, Settings-data clear,
  UMS install, or overlay/root-module modification. Conduit hooks are limited to system
  clipboard/OEM lease and its own activation marker. Existing LuckyTool/COUIExpressive Settings
  scopes were read but not altered and are not a verified cause.
- Evidence and read-only probe are in
  `C:\Users\www\AppData\Local\Temp\ConduitRecovery-20261006`: current Settings crash,
  package/overlay snapshots, decompiled controller, UMS manifest, `settings-resource-probe.txt`,
  and a copy of the currently mounted **UMS-current.apk** (SHA-256
  `3247FA771EDB0083B23D3EF2AED5554482A0EB0CDE644DFC376569EC129B1656`, valid OPPO signer).
- The user explicitly approved **data-preserving repair**. Registered UMS signer SHA-256
  `159e6f37c336f2f41272542dbc1dc85721dd567ed4ac68ac13f2e43003ce0dce` matched the copied APK.
  `adb install -r UMS-current.apk` succeeded at **16:39:58**; version remains **16.59.6**.
  It is now a standard updated system app with an active copy under `/data/app`; no system
  Settings APK was modified. UMS data inode **30597**, device-encrypted inode **30637**, and
  firstInstallTime **2026-09-25 11:51:56** are preserved. Explicit user enabled state was
  restored to **enabled=1**, matching the pre-install snapshot.
- After reinstall, the actual framework probe reports metadata **0x7f120596** and resolves the
  title resource correctly. Settings was force-stopped and cold-launched, with **no new Settings
  crash** during the immediate observation interval. The user then unlocked and explicitly
  confirmed **“设置主页已正常打开”**. Visible homepage restoration is therefore verified beyond
  the `am start` return. Settings APK SHA-256 remains unchanged after repair, and UMS retains
  SYSTEM / UPDATED_SYSTEM_APP / PRIVILEGED flags. There was no data clear, module edit,
  resource patch, or reboot. This Settings incident is resolved; do not reopen boot-loop work
  or disable unrelated root modules. The origin of the stale registration is still unproven.

## Notification fix checkpoint — 2026-10-05

- The user confirms the boot loop was caused by **another root module**, not Conduit, and has
  resolved it. Boot-loop recovery is closed; Settings is separately unresolved above. Do not
  modify unrelated modules or repeat recovery experiments.
- Installed source fix is committed as **`dc03ff3`** on `codex/non-lan-relay-m3-20261002`,
  based on `97ca8a4`. Any subsequent checkpoint commit changes only this handoff; use
  `git rev-parse HEAD` for the full current documentation HEAD. The source, regression check,
  and verified device results are committed locally. The worktree is clean at completion.
  Wake-lock and self-binding experiments remain rejected and reverted.
- The new debug APK was installed in place at **15:57:56** with SHA-256
  `EE1B020930E64F07B4728FC44D9500E79AE62E34D00CB3C192D64CD92D15D232`.
  Its signer matches the baseline; app-data inode **801765**, firstInstallTime, pairing, and
  settings are preserved. Current Conduit UID is **10061**, not historical 10550.
- Primary ADB is `192.168.31.229:5555`. `127.0.0.1:15556` is only the backup ADB transport,
  never a Conduit business route. The app is authenticated and `state=linked,path=lan` to
  Windows `192.168.31.127:41112`, verified through the installed daemon's actual `status` CLI.
- Windows runtime is `D:\Programs\Scoop\apps\conduit\current\conduit-daemon.exe` and its
  actual identity/config/history directory is **`D:\Programs\Scoop\persist\conduit\data`**.
  Preserve it. Uninstall rotated the phone identity; the resulting rapid EOF/reconnect loop
  was fixed by explicitly pairing both sides over LAN at 15:18:58. The daemon was restarted
  with the same binary/data and scoped logs captured in the diagnostic directory below.
- The remaining 5–10-second locked delay was reproduced **before NotificationRelay's callback**
  for UID 10061. A bounded 120-second root OEM lease prototype reduced a marker to **105 ms ±56 ms**
  after another 45 seconds locked. This established the candidate interface; it was not deployed
  as a resident worker. Immediate-after-screen-off tests are not acceptance evidence.
- The installed fix extends the existing optional **legacy LSPosed** `ClipboardHook`.
  It acquires ColorOS's `requestFrozenDelay(timeout=0)` only when Conduit's authenticated-link
  clipboard listener is registered. PackageManager UID must equal the caller's UID. The final
  listener removal or client Binder death cancels the lease. Firmware AIDL classes/constants
  are reflected; unsupported devices fail harmlessly without altering clipboard registration.
  Direct AIDL avoids the OEM manager's extra HandlerThread. There is no polling, wake lock,
  timer renewal, zygote hook, or new persistent process. The low-idle-cost constraint remains.
- Android `testDebugUnitTest`: **50 passed, 0 failed**; `assembleDebug` passed. The regression
  check covers UID spoofing, duplicate listeners, final-listener release, death/removal
  idempotence, reconnect, UID changes, and rejected acquisition. A read-only `app_process`
  check also constructed the actual APK bridge on this firmware without requesting a lease.
- Uninstall cleared LSPosed's Conduit module state/scope; `/data/adb/lspd/cli` is absent, so
  no live DB edit was used. The user enabled **System Framework + Conduit** and rebooted.
  New system_server at **16:06:24** logged 2 clipboard-access and **2 OEM protection hooks**;
  the app logged its LSPosed marker at 16:06:52. The actual OEM lease query returns
  **`Long.MAX_VALUE`**, confirming the connection-owned protection is loaded and active.
- Installed runtime lifecycle checks passed: DISCONNECT -> lease **0**; CONNECT -> **MAX**;
  app force-stop/Binder death -> **0**; launch -> **MAX**. For these explicit test actions,
  `am start-service` must run through `su -c` because SyncService is not exported. An ordinary
  shell rejection is not an OEM lease failure.
- Production acceptance: after uninterrupted screen off from **16:13:25 to 16:18:51 (+08)**,
  marker `CONDUIT_LOCKED_20261005_081851_742` reached Windows in **156 ms ±41 ms** and OEM
  protection still reported **MAX**. The phone offset was refreshed to **+684 ms** after reboot.
  Power state was Dozing/screen off, but DeviceIdle was ACTIVE because USB charging was present;
  this is sustained lockscreen evidence, not deep-Doze evidence. The synthetic-marker fixture
  `Test-LockedNotification.ps1` is bounded to 30 seconds and polls only Windows history.
- A separate bounded **forced deep-Doze** acceptance run **passed**. Initial
  `mForceIdle=false` and battery `UpdatesStopped=false` were verified; existing battery exemption
  is `user,com.conduit.sync,10061`. The test uses `cmd deviceidle force-idle deep`, holds IDLE
  for 90 seconds, measures one marker, and calls `unforce` in `finally`. With the screen still
  off since 16:13:25, marker `CONDUIT_LOCKED_20261005_082300_113` arrived at **16:23:00** in
  **114 ms ±41 ms**. Deep state was **IDLE before and after** the marker and lease remained MAX.
  The original automatic policy was verified restored: **mForceIdle=false**, DeviceIdle ACTIVE,
  screen still off. No battery override/whitelist was changed. Final clock offset +694.5 ms
  (77 ms round trip) is consistent with the earlier +684 ms estimate within measurement bounds.
- Local diagnostics are in `C:\Users\www\AppData\Local\Temp\ConduitRecovery-20261005`:
  lease/clock outputs, baseline and installed APK checks, LSPosed snapshot, daemon logs,
  `HansProbe.java`, and the bounded notification fixture. Raw logs may contain private data;
  report only synthetic marker timing. `notifications.tsv`'s first column is Android post time,
  not Windows receive time. Actual toast visibility has not been visually checked.
- The installed fix passed sustained screen-off and controlled deep-Doze acceptance on this
  firmware; disconnect/Binder-death cleanup was independently verified. Next: collect the
  user's normal incoming-notification feedback during longer battery-powered use. Overnight
  standby, non-LAN delivery with the new hook, and other OEM firmware are not established by
  these two marker samples. Root scopes and Android's existing battery exemption are required
  for the verified device path; do not claim generic non-root OEM-freeze immunity.
## Historical recovery checkpoint — 2026-10-05

The following entries describe the recovery investigation before the user's resolution above.
They are retained as evidence; their unresolved state and next steps are superseded.

- **Recovery is incomplete.** The user now reports normal boot still loops after uninstalling
  Conduit, and Settings cannot open even after the safe-mode boot. Do not treat a safe-mode
  desktop as restored normal phone operation or permanent module disable as the solution.
  Respect the uninstall; do not reinstall Conduit without a renewed request. Current LAN ADB
  dropped during the user's intervening phone actions; reconnect only when safe-mode boot
  and Wi-Fi are available. USB/FRP have not become usable.
- Settings failure was reproduced at 14:26:29 by `am start -W -a android.settings.SETTINGS`:
  `com.android.settings` immediately crashed with `Resources$NotFoundException: String resource
  ID #0x7f120e0e` from `TopLevelSmartServicePreferenceController.getCustomizedTitle`.
  `Status: ok` from Activity Manager did not mean the UI remained open. Evidence:
  `settings-crash-current.log`, `settings-package-current.txt`, `overlay-current.txt` and
  `overlay-list-current.txt` in the recovery directory below. The active stock Settings
  overlay is `/product/overlay/SettingsResCommon_Sys.apk`; no Material You overlay was listed
  for Settings in the captured safe-mode state. Exact resource mismatch cause is unresolved.
- `/data/adb/ksu/log` contains previous-boot `logcat.old.log` and `dmesg.old.log` plus Zygisk
  `modules_info.old` / `znctx.old`. Preserve them as soon as ADB is restored. Attempts to
  capture them were interrupted by ADB becoming offline; the local `ksu-logcat.old.log` is
  empty and is not usable evidence. Similarly, no Settings APK/resource dump was retrieved.
- **Phone has booted in KernelSU safe mode.** After an assistant-issued diagnostic reboot,
  the user reported a repeated OnePlus/ColorOS logo boot loop. The user now confirms successful
  boot after the KSU Volume Down gesture. LAN ADB `192.168.31.229:5555` is restored;
  `sys.boot_completed=1` and system_server PID `3502` remained stable across checks. All 11 KSU
  modules are disabled, verified by both the user and `ksud module list`. Keep them disabled
  while identifying the trigger. Do not resume notification experiments, issue diagnostic
  reboot commands, wipe data, flash firmware, or remove all root modules.
- Repository HEAD remains `97ca8a4` on `codex/non-lan-relay-m3-20261002`. The rejected wake-lock
  and self-binding experiments have been reverted in source. The phone's installed APK was
  pulled and matched the saved self-binding experiment before rollback. It has now been
  replaced with the clean `97ca8a4` APK via `adb install -r -t`, with matching signer verified.
  The installed SHA-256 is `FB3D27241E6359A5A106101DC04011731808D934F3BD6EA1CBEBAB73A67326B8`.
  The app data inode and original install time are unchanged; no uninstall or data clear
  occurred. Only this handoff is modified in the tracked worktree.
- Genuine locked-screen testing disproved the wake-lock and self-binding hypotheses: Hans
  still froze UID 10550 in `LcdOff`; the wake lock was force-released by Oplus. One unique
  notification marker logged at `12:31:51.715` reached `conduit.notif` at `12:31:56.288`,
  just after a `Packet` unfreeze at `12:31:56.279`. Earlier baseline evidence recorded a
  21.4-second callback delay. Immediate-after-screen-off tests are not acceptance evidence.
- Boot-loop native tombstones are now preserved. Several `zygote64` crashes at 14:02–14:03
  report `JNI FatalError called: (system_server) Not allowlisted (64)` for
  `/my_product/cust/IN/overlay/GmsConfigOverlaySearchSelector/GmsConfigOverlaySearchSelector.apk`.
  Camera/media services also abort because hwservicemanager is unavailable. These identify
  the immediate fatal condition, but not the responsible module/configuration or its relation
  to the Conduit experiment. The successful boot preceded the APK rollback; do not claim
  that the rollback fixed the boot loop or blame a specific module without further evidence.
- Evidence and APK backups are in
  `C:\Users\www\AppData\Local\Temp\ConduitRecovery-20261005`: post-safe-mode crash/system/kernel
  logs, module states, native tombstone headers, full zygote tombstones `00` and `12`, full
  tombstones `13`–`15`, package dumps before/after rollback, and both APK variants. These
  local diagnostic files may contain private data; do not publish raw logs. `/sys/fs/pstore`
  was empty. USB ADB/fastboot remains absent; FRP backup ADB `127.0.0.1:15556` is offline.
  Neither connection status is a Conduit business-route diagnosis.
- User confirms **KernelSU / KernelSU Next**. The KernelSU built-in safe-mode
  gesture is repeated **press-and-release** of Volume Down more than three times after
  the first boot splash, before the boot animation. Earlier generic hold-Volume-Down
  advice must not be substituted for this gesture. Kernel logs now explicitly confirm
  `KEY_VOLUMEDOWN pressed max times, safe mode detected!`. The official
  procedure is documented at `https://kernelsu.org/zh_CN/guide/rescue-from-bootloop.html`.
- Safest next action: reconnect after safe-mode boot, preserve the previous-boot KSU logs,
  inspect the actual Settings APK/resources and overlay state, then perform a scoped,
  reversible repair and restore required root-module functions. Notification latency
  remains unresolved; do not use immediate-after-screen-off tests or resume OEM freeze
  experiments while phone boot stability is still being established.

## Previous snapshot — 2026-10-04

- `codex/non-lan-relay-m3-20261002` contains notification fix commit `ed9748b` and the
  ColorOS settings fallback implementation `9f728ec`; both are pushed to `origin`. GitHub Actions run
  `37182119252` passed Android, Windows x64, and Relay Linux for this branch.
- The current live path is direct LAN: Windows `192.168.31.127:41112` ↔ phone
  `192.168.31.229:<ephemeral-port>`, with daemon status `state=linked,path=lan`. The historical
  phone ADB address `192.168.1.250:5555` is no longer on the current subnet. `127.0.0.1:15556`
  and `15557` are FRP STCP ADB backup transports used by diagnostics; they are not Conduit
  notification or Relay sockets. Conduit uses `127.0.0.1` only for the explicit `adb reverse`
  debugging bypass documented in `docs/development.md`.
- The repeated Nagram XF notification was reproduced under one Android key with identical
  rendered content. `NotificationRelay` now ignores a changed `postTime` when comparing the
  cached rendered payload, while still sending the original timestamp and allowing changed
  content through. `NotificationRelayTest` and the Android JVM suite pass.
- A locked-screen test showed the remaining ten-to-twenty-four-second delay occurs before
  `NotificationRelay` runs: Oplus `Hans` freezes UID 10550 and later unfreezes it for a packet.
  Android Doze whitelist, `RUN_ANY_IN_BACKGROUND=allow`, connected-device FGS, and the added
  `remoteMessaging` FGS type all remained active while Hans still froze the process. The
  installed APK therefore contains the routing and duplicate fixes, but the device-specific
  delay still requires an OEM Hans/auto-freeze exemption or policy change.
- The comparison baseline is now recorded: this OnePlus's Phone Link path includes the system/
  privileged `com.oplus.linker` package (UID 10159), while Sefirah 3.1.0 has a connected-device
  foreground network service, `WAKE_LOCK`, and Shizuku/worker bridge components. Conduit is a
  normal user UID without those vendor privileges, so its Hans freeze is an architectural
  compatibility gap rather than a LAN/ADB route selection issue.
- The installed debug APK now adds a usable **Settings > Background delivery** entry for this
  OnePlus. The app checks `oplus.software.pms_app_frozen` and `oppo.appdisable.support` before
  offering the legacy `oplus.intent.action.settings.APP_FORZEN_OPLUS_SETTINGS` page. This device
  reports neither feature, so the row is labelled **Background battery settings** and opens
  `android.settings.VIEW_ADVANCED_POWER_USAGE_DETAIL` for Conduit with the standard
  `request_ignore_background_restriction` focus; the OEM app-freeze page remains available on
  models that expose it. The APK was rebuilt and installed through the LAN ADB target
  `192.168.31.229:5555`. A device A/B check also enabled Conduit's AccessibilityService for one
  screen-off interval; Hans still froze UID 10550 after about 26 seconds, so accessibility is not
  treated as a fix. After the user confirms the background setting, repeat the unique-marker
  locked-screen notification test.

- The branch's earlier Relay/UI checkpoint remains in the same pushed history; the current
  notification commits and their successful CI run are recorded above.
- GitHub Actions master run `36249356300` and tag/release run `36249808688` both completed
  successfully. The GitHub Release contains the Android debug APK, Windows x64 package, Linux
  Relay binary and checksums. No local build output is authoritative.
- The release version is `0.1.4`; Android `versionCode` is `5`, Windows `ApplicationVersion` is
  `5`, and the sparse Share Target package is `0.1.4.0`.
- Notification-latency investigation on 2026-10-01 found the Android and Windows wall clocks
  within about 65 ms during a synchronized read, so the observed ten-second-plus delay is not
  a timestamp conversion artifact. `NotificationRelay` previously loaded application/contact
  icons and encoded PNGs inside `onNotificationPosted`; the committed fix builds that payload on
  Link's existing serialized sender thread instead, keeping the system callback short.
  The app now also exposes Android's standard `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` request in
  Settings > Background delivery. The request is user-approved and supplements the OEM-specific
  "allow background activity" setting; it cannot silently disable Oplus FastFreeze.
- The non-LAN reconnect failure found on 2026-10-02 was caused by Bettbox's VPN DNS returning
  `198.18.0.0/15` fake addresses for every production Relay. `Link.kt` used to reject those
  answers when no explicit fallback was configured, so the built-in hostname-only Relay catalog
  could never dial. The fix preserves the fake address when no fallback is present so the VPN can
  map it back to the hostname; an explicit user fallback still takes precedence. Device probes
  reached all four Relay ports through the fake-IP mapping.
- The Android bottom navigation now uses Material 3 selected/unselected Home and Settings icons,
  semantic color roles, the selected `secondaryContainer` indicator, and explicit content
  descriptions. This is a focused navigation refresh; it does not add a new UI dependency or
  change the transport/service lifecycle.
- GitHub Actions run `36967899497` passed the Android, Windows, and Relay jobs. Its Android debug
  APK was installed on the test phone at 13:19. With Wi-Fi disabled, the phone linked through
  `conduit-wa.414222.xyz:41113`; Windows reported `state=linked`, `path=relay`. Wi-Fi was then
  re-enabled and the pair returned to `path=lan`.
- The current Windows control surface is the self-contained `windows/conduit-ui` Uno Platform /
  WinUI 3 application. The Rust `conduit-daemon` remains the resident transport and integration
  process; the UI is on-demand and owns no transport.
- Android built-in Relay defaults are `conduit-us`, `conduit-wa`, `conduit-tyo` and `conduit-jp`
  on port `41113`, with no embedded production origin IPv4 fallback. Live node deployment must
  be checked separately before claiming that every default endpoint is reachable.
- Commit `43600c997e62fe2549d652cfc2028e514545f24d` removed the production Relay origin IPs from
  the current source, tests and public documents. Older Git history is intentionally unchanged.
- The dated entries below preserve historical evidence. When they conflict with this snapshot or
  the current source, use this snapshot and the current code.

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

3. The compatible Relay migration is **deployed on the tested path**. The installed Android/Windows
   endpoints use explicit roles; keep legacy inference for older clients until M2 evidence and
   deliberate retirement. The source catalog has four hostname-based defaults, but live reachability
   of every endpoint must be verified separately.
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
   timer-driven scoring. Multi-Relay client selection is implemented as passive quality learning
   and sticky failover: Android keeps one active session and learns only from real
   connection/session/content-transfer events, including real time-to-session-up EWMA. The source
   defaults contain four hostname-based endpoints; confirm live deployment before treating the
   whole catalog as reachable.
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
11. The primary control surface is the self-contained `windows/conduit-ui` Uno Platform / WinUI 3
    application. It is on-demand only: it owns no transport and does not add a resident UI process
    after the window closes. The Rust `conduit-control` binary remains a source-level compatibility
    path, but it is not the current packaged UI. Preserve the event-driven status/config seams and
    the no-polling lifecycle.
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
HEAD:                      8987c1b Release Conduit 0.1.4
origin/master:             8987c1b Release Conduit 0.1.4
tag:                       v0.1.4
previous security cleanup: 43600c9 Remove embedded Relay origin IPs
```

The published `master` line includes the tested persistence fix, screenshot implementation,
compatible Relay migration, M0/M2 sampling, bidirectional file-transfer UX, long-transfer
heartbeat fixes, Windows parked-socket keepalive, Windows Relay SOCKS5 support, notification
actions/inline reply, bounded MessagingStyle conversation history, Android boot recovery and the
production-origin cleanup. A future code change must still use the GitHub Actions build gate.

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
  polled status service. `conduit-daemon status` reports daemon/link/phone/path/Relay state on
  demand. Android announces its device name once per encrypted session. The packaged
  `windows/conduit-ui` Uno/WinUI 3 surface consumes this seam as an on-demand GUI and exits fully
  when its window closes; it does not own transport state. Keep the status/config seams event-driven
  and do not turn the UI into a resident polling service.
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

The following entries are dated historical evidence. They remain useful for reproducing checks, but
the current release baseline is the snapshot at the top of this file.

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
- Bettbox fake-IP handover history: the relay hostname resolved into `198.18.0.0/15` and could
  produce `Broken pipe` after underlying-network changes without reaching the relay. Built-in
  production relays now ship only stable DNS names and ports, with no embedded origin fallback.
  A synthetic answer without an explicit user fallback fails that candidate so normal multi-relay
  reconnect can advance to the next hostname. The selection logic remains JVM-tested with
  documentation-only addresses.
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
## 2026-08-27 v0.1.0 release / Scoop deployment

- Functional release commit: `e55b17c` (`Prepare Conduit 0.1.0 desktop release`), pushed to
  `nonlog/Conduit`; annotated tag / GitHub Release: `v0.1.0`.
- Release assets: `Conduit-0.1.0-windows-x64.zip` and the current debug-signed Android APK.
- `nonlog/scoop-www` commit `b752b5d` adds `bucket/conduit.json`; the Windows ZIP SHA-256 is
  `189d2e2bfc48c9896e4adf28f5a1cf4a35356b6cda42935afbf783b529acc5e8`.
- The target Windows machine is now installed through Scoop as `www/conduit 0.1.0` at
  `D:\Programs\Scoop\apps\conduit\current`. The previous manual install under
  `%LOCALAPPDATA%\Programs\Conduit` was removed after all live references migrated.
- HKCU Run, the AUMID Start Menu shortcut, and Explorer `Send with Conduit` all point at the Scoop
  `current` path. `%LOCALAPPDATA%\Conduit` was deliberately retained for identity/history/config.
- Final installed daemon was detached from the AgentDock job via WMI only for this remote validation;
  it remained responsive and linked to `OnePlus 12` over `us.414222.xyz:41113`. The phone still
  persisted desktop name `LOG`.
## 2026-08-27 Sefirah-reference UI refactor — pass 1

- `shrimqy/Sefirah` (Windows) and `shrimqy/Sefirah-Android` are now the UI reference baseline for hierarchy and information architecture, not a source for copied branding or feature scope.
- Android home was restructured to device-first hierarchy: a large Material 3 device card, active transfers only when present, then one compact settings card. Clipboard-history navigation/back behavior is preserved.
- Windows control UI was restructured to a Sefirah-like two-pane utility: persistent device/status pane on the left, `Settings` content with Relay and Windows cards on the right. It remains native Win32/DWM/Common Controls and on-demand only.
- Android `assembleDebug + testDebugUnitTest` passes and the APK was installed on the OnePlus test phone. Windows daemon tests pass 53/0/3 ignored; `cargo check` and release `conduit-control` build pass.
- No polling, UI timer, watcher, resident control UI, WinUI, or WebView was introduced.
- No unlocked-phone foreground screenshot was captured during this Sefirah-reference UI refactor.

- The Scoop-installed `Conduit.exe` is temporarily overlaid with this pass-1 development GUI for local visual review; package version remains 0.1.0 and no new Release/bucket update has been published.

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

## 2026-08-28 Chrome webpage share preview-image precedence fix

- Root cause of "Chrome share page -> PC receives an image": `ShareActivity` collected both
  `EXTRA_STREAM`/`ClipData` URIs and `EXTRA_TEXT`, then unconditionally handled any URI before text.
  Chrome can attach a preview-image URI to a normal webpage share, so the auxiliary preview was
  transferred as a file and the real page URL never reached the `SHARED_URL` path.
- `ShareActivity` now classifies a valid bounded `http`/`https` `EXTRA_TEXT` as the webpage payload
  before auxiliary URIs when the intent looks like a page share (text MIME type or non-empty
  page title/subject). A real image/file share still keeps URI precedence when there is no page
  signal, so an image whose caption happens to be a URL is not automatically reclassified.
- Added JVM regressions for Chrome-like `URL + title + image/png + URI` and text/plain page shares,
  plus a guard that preserves ordinary image/file sharing.
- Verification: Android `testDebugUnitTest + assembleDebug` succeeded with **31 passed / 0 failed**.
  The debug APK was installed in-place on the connected OnePlus. A synthetic Chrome-like
  `ACTION_SEND image/png` carrying an http URL, title, and auxiliary `EXTRA_STREAM` reached the
  running Windows daemon as a Shared Link (`https://example.com/conduit-chrome-share-test`) rather
  than a file/image; the temporary history row was removed afterward. The live link remained on
  TYO Relay. No phone screenshot was captured or opened.
- Real Chrome UI reproduction should still be manually spot-checked by the user; the installed
  build contains this fix and is ready for that check.

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

## 2026-08-29 Sefirah-parity WinUI visual closure

- Continued the Uno/WinUI rewrite by comparing Conduit directly against Sefirah's `Views/MainPage.xaml`, `Views/SettingsPage.xaml`, `Views/Settings/GeneralPage.xaml`, `UserControls/DeviceControlCenter.xaml`, and `UserControls/TitleBar.xaml` rather than styling from screenshots alone.
- Title-bar sizing now matches Sefirah's 32-DIP bar with a 25x25 app mark and 14-point caption text. The left device panel removes the duplicated connection-state line, keeps the 54x100 phone silhouette, and uses the compact route/status presentation.
- Settings now uses a compact WinUI `NavigationView` rail at this window size, with native hamburger/General/About items and full-width CommunityToolkit `SettingsCard` content. `General` uses a 28-point heading and Sefirah-like 40-DIP content inset. Relay and SOCKS5 fields are fully readable and no horizontal scrolling is allowed.
- Shared Links is now a direct-action list: clicking a row opens the URL and the redundant selected-link footer / separate `Open` CTA was removed. This follows the design-system rule that clickable rows should not duplicate navigation with a second button.
- Desktop-only visual verification covered the top of General, the bottom settings rows, Shared Links, native caption buttons and notification cards. `Received files` exposes `Default` and `Select location`; `Conduit data` resolves to `D:\Programs\Scoop\apps\conduit\current\data`, which remains the Scoop persist junction. Notification history visibly uses the Android source-app icon.
- The latest self-contained WinUI publish was overlaid onto `D:\Programs\Scoop\apps\conduit\current` without replacing the `data` junction. The daemon stayed resident and linked to OnePlus 12 over WA Relay.
- Installed-path file-send E2E after the visual closure: `conduit-send.exe` exited `0` in about 1.0 s, the exact temporary file appeared in `/sdcard/Download`, and the test copies were removed afterward.
- Final regression for this checkpoint: daemon **59 passed / 0 failed / 3 ignored**, send helper **2 passed / 0 failed**, `cargo fmt --check`, `git diff --check`, WinUI Release build and self-contained publish all pass. No commit or push was made.

## 2026-08-30 GitHub build-authority migration

- Development/build output is now GitHub-owned. `.github/workflows/build.yml` builds Android on Ubuntu, Windows Rust + Uno/WinUI on `windows-latest`, and the Linux Relay on Ubuntu. Clean-run artifacts are `conduit-android-debug`, `conduit-windows-x64`, and `conduit-relay-linux-x64`; Log is no longer part of the build graph.
- The first clean run exposed two hidden local-environment assumptions and was intentionally allowed to finish for diagnosis: Android's `yes | sdkmanager` failed only because `pipefail` treated `yes`'s expected broken pipe as an error, and WinUI Release publish had restored Uno DevServer assets under Debug configuration. The workflow now invokes `sdkmanager` directly and restores the WinUI project with `-p:Configuration=Release` before `--no-restore` publish.
- Clean GitHub run `33319898971` for commit `f7830fe` then passed all build jobs: Android tests/APK, Windows Rust format/tests/release binaries + Uno restore/publish/package, and Relay tests/release binary. The non-tag Release job was correctly skipped.
- GitHub Actions dependencies were moved to their current Node-24-compatible major releases. Toolchains are repository-pinned with Rust `1.98.0`, .NET SDK `10.0.400`, Uno SDK `6.8.0-dev.21`, and a project-local NuGet source definition.
- `scripts/package-windows.ps1` produces the same `Conduit-$version-windows-x64.zip` layout expected by the existing Scoop manifest. Tag builds also emit `SHA256SUMS.txt` and `Conduit-$version-windows-x64.zip.sha256`; the current `www` bucket can continue computing its hash during autoupdate, or later consume the published per-asset hash directly.
- `scripts/install-github-build.ps1` is the Log-only development installer: it selects a successful Actions run, downloads its Windows/APK artifacts, overlays only program files into the existing Scoop installation, preserves the persisted `data` junction, refreshes normal Windows integration, and installs the APK over ADB. `-DownloadProxy` supports Log's local Mihomo proxy without changing Conduit Relay routing.
- Real Log validation used only GitHub run `33319898971` artifacts, not a local rebuild. Android `0.1.0` installed successfully over ADB; the Windows Actions package installed into `D:\Programs\Scoop\apps\conduit\current`; `data` remained a junction to `D:\Programs\Scoop\persist\conduit\data`; the WinUI `Conduit.exe` launched and responded; the installed daemon linked to OnePlus 12 over WA Relay after the phone service was restarted following the APK package replacement.
- Installed-path file-send E2E after the migration passed: `conduit-send.exe` exited `0`, the exact temporary file appeared in OnePlus 12 `/sdcard/Download`, and both test copies were removed. AgentDock command jobs reap ordinary child processes when the job exits, so the Log-only development installer now re-launches the installed daemon through `Win32_Process.Create` after installation. A second real install proved that detached process survives the invoking job and relinks to OnePlus 12 over WA Relay; no scheduled task or resident build agent was added.
- Stable installation/update remains deliberately separate: tagged GitHub Release assets feed the existing Scoop manifest, while `scoop update conduit` and all real UI/notification/Explorer/ADB/end-to-end validation continue to run on Log.

## 2026-09-01 Android home merge + Windows launch/icon optimization

- Android Home and Devices were consolidated because both pages repeated the same device/connection card. The bottom navigation now has only Home and Settings; there is no separate Devices page.
- The Home clipboard control is now a full-width recent-item preview rather than a sparse `Clipboard <count>` button. It shows the newest clipboard preview (up to two lines), history count, relative age, and a trailing upload/download glyph with accessibility semantics `Sent to desktop` or `Received from desktop`. The whole card still opens clipboard history.
- Windows on-demand UI startup was moved to a first-frame-first path: `MainPage_Loaded` presents the WinUI shell before low-priority status/config/history initialization. The publish pipeline now uses .NET ReadyToRun; restore and publish both carry `PublishReadyToRun=true` so the runtime pack is restored deterministically on GitHub-hosted runners.
- The resident tray's Open command now reuses the current Uno/WinUI window by title instead of only looking for the obsolete `ConduitControlWindow` class. Repeated tray activation therefore restores the existing process instead of spawning duplicates.
- Windows icon generation was moved into the GitHub Windows job before Rust/WinUI compilation. Small brand icons now use size-specific Fluent 16/20/24 vector sources and a larger 82-86% glyph footprint instead of scaling one inset 24px glyph at 65%. Tray ICOs now contain native 16/20/24/28/32/40/48/64 frames. At Log's 125% system DPI the daemon deterministically requests the native 20x20 frame, avoiding Explorer upscaling a 16px bitmap. The Start Menu shortcut continues to reference the installed `conduit-icon.ico`.
- CI surfaced two optimization-specific mistakes before deployment: `GetSystemMetricsForDpi` was not exposed by the pinned `windows-sys`, so tray sizing was replaced with a fixed 16px-to-system-DPI mapping; ReadyToRun initially failed because restore had not requested the R2R runtime pack. Both were corrected in GitHub and clean run `33412249555` passed Android, Relay, Windows Rust tests/release, Uno restore/ReadyToRun publish, package, and artifact upload.
- Log installation used only run `33412249555` artifacts. ADB/UIAutomator verification (no phone screenshot) found Home and Settings, no Devices tab, a real newest clipboard preview, and `Sent to desktop` on the direction icon.
- Real tray-path timing on Log after installation: five cold UI launches from the daemon's WM_COMMAND Open path were 665.1 / 582.5 / 553.1 / 527.8 / 509.1 ms, average **567.5 ms**. The earlier pre-optimization direct-executable reference averaged 836.3 ms; the user's prior tray observation was 2-3 s. An already-open UI remained one process after another tray Open command.
- Installed WinUI stayed responsive and loaded OnePlus 12 / Connected / Relay state after deferred initialization. Final installed-path file-send regression exited 0 in 720 ms, created the exact file in OnePlus 12 Downloads, then removed both local and remote test files. The daemon remained linked over US Relay.

## 2026-09-01 UI/UX polish — managed Relay/proxy, monoline identity, transfer progress

- Windows identity is being simplified to a transparent single-colour Fluent phone/desktop mark instead of the previous purple rounded-square tile. The CI icon renderer keeps native 16/20/24+ frames, while the notification-area icon remains a transparent black/white glyph chosen from Windows light/dark theme. The Start Menu shortcut, notification attribution identity and Explorer `Send with Conduit` verb all consume the same high-resolution generated icon family.
- The Windows General page no longer exposes the raw `relays=` string as a single-line text box. It presents four managed Relay points (US, WA, TYO, JP) with individual selection controls. Production DNS aliases are `conduit-us.414222.xyz`, `conduit-wa.414222.xyz`, `conduit-tyo.414222.xyz`, and `conduit-jp.414222.xyz`, all on port 41113; the legacy hostnames stay valid behind those aliases.
- Relay proxy UX is now explicit: `System proxy`, `Manual SOCKS5`, or `Direct`. `relay_proxy=system` resolves the enabled Windows Internet Settings proxy at daemon reload/start and uses a SOCKS/SOCKS5 endpoint; a manual value remains backward compatible. LAN sessions continue to stay direct.
- Explorer file send is being upgraded from final-error-only feedback to event-driven transfer progress. The resident daemon forwards bounded real byte progress over the existing named pipe, and the tiny `conduit-send.exe` helper owns one replace-in-place Windows progress toast. Android already had byte progress internally; completion/failure now remains visible briefly instead of immediately cancelling the transfer notification, so fast transfers still leave user-visible feedback.
- The GitHub contributors REST endpoint was checked before changing history: the live default-branch contributors are `codex`, `claude`, and `nonlog`; there is currently no `claude[bot]` contributor or `claude[bot]` commit on `master`. The screenshot entry is therefore treated as stale GitHub contributor UI/cache, not a reason to rewrite valid repository history.
- This checkpoint is intentionally pre-validation. Clean GitHub Actions, GitHub artifact installation on Log, Relay/proxy migration, desktop/Android notification verification, and final measurements are still required before this section is marked complete.

## 2026-09-04 UI redesign — Material 3 mobile, Fluent 2 desktop, and drag-and-drop file transfer

- **Android companion UI overhaul (`MainActivity.kt`):**
  - Followed Sefirah & Material 3 guidelines (`MASTER.md`). Replaced the dual `<=>` icon ambiguity with a dedicated PC/desktop monitor vector asset (`ic_desktop.xml`) on the device card.
  - Fixed severe layout squeezing bug where the connection route pill (`Relay · WA`) was forced into a vertical single-character wrap beside long state labels. Redesigned `SefirahDeviceCard` with a 3-tier vertical hierarchy: Desktop Name (19sp bold) -> Connection state with colored status dot (ð¢ Linked / ð¡ Reconnecting / ⚪ Idle) -> Independent route chip (`⚡ LAN Direct` / `ð Relay · WA`).
  - Added dedicated `ic_photo.xml` vector icon, replacing the accidental use of `ic_share_target.xml` whose 108x108 background rectangle had rendered as an opaque tinted black square under Compose.
  - When linked, `SefirahDeviceCard` directly exposes `Send files` and `Send photo` action buttons without requiring a separate redundant middle card or navigating to system share sheets.
  - Grouped settings into clean Material 3 surface containers for Sync & Privacy and About.
- **Windows control surface redesign (`MainPage.xaml`, `MainPage.xaml.cs`):**
  - Enhanced the left-pane phone hardware silhouette with an authentic top speaker/camera punch-hole bar, rounded corner frame, and live wallpaper mapping.
  - Added native Explorer-to-phone Drag & Drop support (`DropZone`): dropping any file into the left pane triggers immediate send to the connected phone via `ViewModel.SendFileAsync`.
  - Upgraded connection status to a Fluent secondary badge chip with route indicator icon and responsive refresh.
  - Notification cards styled with rounded icon containers, medium font weights, and clear-all action.
- **Git & GitHub CI/CD:**
  - Branch: `codex/ui-redesign`, base: `master`; open Pull Request: **PR #4**.
  - The redesign was consolidated into commit `182591e` from Gemini Code Assist. Review fixes are committed separately with the required `Codex <codex@openai.com>` author/committer identity before merge.
  - Review fixes remove clipboard actions that could copy only the bounded history preview, apply the existing 16-file sender-queue cap and a fresh connection check to the in-app Android picker, and make Windows drag-and-drop send every dropped file sequentially instead of silently ignoring files after the first.
  - Merge/deployment is gated on a clean GitHub Actions run for the reviewed head. Runtime installation on Log uses only that GitHub-built artifact; no Log-local build output is authoritative.


## 2026-09-04 Tray tooltip repair & v0.1.1 Scoop rollout

- **Tray icon blank banner root cause:** In `tray.rs`, using `NOTIFYICON_VERSION_4` causes Windows Vista and later to suppress standard `szTip` tooltips into a blank banner placeholder unless `NIF_SHOWTIP` (0x80) is explicitly specified in `uFlags`. Added `NIF_SHOWTIP` to `windows_sys::Win32::UI::Shell` imports and `data.uFlags`.
- **Version 0.1.1 tag & release:** Bumped version to `0.1.1` in `Cargo.toml`, `android/app/build.gradle.kts` and `Conduit.csproj`. Pushed tag `v0.1.1` to GitHub, successfully triggering the GitHub Actions build and release workflow.
- **Excavator workflow & www bucket update:** Triggered the `Excavator` workflow in `nonlog/scoop-www` via workflow dispatch. It autoupdated `bucket/conduit.json` to version `0.1.1` with SHA-256 `37e45a021f356ef1b0ac9977d12d74a59e01a833efa00386df0ddc162a9dd71c`.
- **Desktop client deployment:** Upgraded the installed desktop companion via `scoop update conduit` to `0.1.1`. The new resident daemon (PID 7236) is running and the tray tooltip displays "Conduit" cleanly.
