# Conduit TODO

> **Updated:** 2026-08-26  
> This file is the compact execution list for unfinished implementation and verification work.
> `backlog.md` remains the priority/rationale record; `progress.md` remains the evidence record.

## Pending implementation

### P0 / near-term product work

- [x] **Throttle Android file-transfer progress refreshes.**
  - Do not update Compose/SystemUI on every 32 KiB chunk.
  - Intermediate updates are capped at one per 250 ms (4 Hz).
  - Preserve final 100% and failure/completion notifications immediately.
- [x] **Multi-relay selection and failover — client implementation complete.**
  - Support a configured set of Relay endpoints rather than one hard-coded TYO endpoint.
  - **Do not run periodic speed tests or periodic probes on Android.** Conduit exists to avoid the
    CPU/radio/battery cost of Link to Windows; idle routing logic must remain event-driven.
  - Windows may keep one cheap parked responder per configured Relay because the desktop is the
    powered side. Android must keep only one active Relay path/session at a time.
  - Make Android the Relay-selection owner. Learn quality passively from events Conduit already has:
    connect/Noise success or failure, abnormal disconnects/timeouts, heartbeat health, and real
    screenshot/photo/image/file transfer completion/throughput.
  - Prefer a sticky historical winner. Do not switch because another endpoint has slightly lower
    latency; only fail over on meaningful degradation/failure or a clearly better accumulated score.
  - On a natural reconnect/network-change event, try historical candidates sequentially with bounded
    handshake deadlines. Do not wake the radio later just to re-benchmark a healthy active session.
  - Put repeatedly failing endpoints into a cooldown, then merely allow them to be tried again on a
    future natural reconnect; do not actively probe them when the phone would otherwise be idle.
  - Treat ICMP ping/RTT as, at most, a final tie-breaker. Reliability and real end-to-end content
    performance dominate because low-latency Relays can still suffer severe loss/retransmission.
  - Keep quality history coarse to the current network class/VPN path and age it with EWMA; avoid
    requiring extra Android permissions solely to fingerprint networks for scoring.
  - Keep LAN discovery/direct TCP strictly preferred when a real same-LAN path exists.
  - Keep proxy policy per endpoint/path; Windows currently uses local Mihomo SOCKS5 for Relay only.
  - Additional public Relay nodes are **not deployed yet**; production still has only TYO active.
- [x] **Remote completion ACK for desktop -> phone file sends.**
  - `conduit-daemon send <file>` now returns success only after Android publishes the Downloads row.
  - One `FILE_RESULT` is sent per whole file; there is no per-chunk ACK or stop-and-wait penalty.

### Windows operability / UX

- [x] **Windows daemon autostart at sign-in.**
  - Per-user HKCU Run entry starts the daemon hidden in the interactive login session.
  - Early 41112 bind is the zero-extra-resource single-instance gate; duplicate launches exit before
    clipboard/toast/Relay workers start.
- [x] **Thin Windows control surface — functional on-demand implementation complete.**
  - `conduit-control.exe` is a separate GUI-subsystem process from the daemon and exits completely
    when the window closes; there is no tray process, timer, watcher, or resident UI framework.
  - It surfaces live snapshot state/phone/path/Relay, Relay/proxy configuration, autostart and
    Explorer integration, plus manual Refresh/diagnostics actions.
  - Event-written `status.txt` and startup-read `config.txt` remain the only state seams.
- [x] **Fluent visual polish for the Windows control surface.**
  - Still pure/on-demand Win32: no WinUI, WebView, tray, watcher, or background refresh loop.
  - Uses Windows app-theme/accent settings, DWM dark/light title bar, rounded cards, Segoe UI
    Variable, themed common controls, and HWND-derived DPI sizing.
  - Target-machine visual check at 125% scaling showed the old black DPI gutter is gone and the full
    layout fits. Refresh remained responsive; normal close returned the process count to zero.
- [x] **Explorer / shell file-send integration.**
  - HKCU file context menu uses a tiny on-demand `conduit-send.exe` GUI helper.
  - The helper starts no transport; it invokes the existing hidden `send <path>` CLI, which reuses
    the resident daemon's named pipe and waits for Android's publication ACK.
- [x] **Relay/proxy settings UX foundation.**
  - `%LOCALAPPDATA%\Conduit\config.txt` is the normal persistent source; `conduit-daemon config`
    provides explicit show/set commands, while environment variables remain optional overrides.
  - LAN traffic is never proxied. The future thin control window should edit this same file rather
    than inventing a second settings store.

### Android / notification extensions

- [x] **Fix light-surface Android status-bar icon contrast — implementation complete.**
  - Day theme explicitly requests dark status/navigation glyphs; night theme explicitly requests
    light glyphs. This is app-window system-bar appearance, not notification small-icon artwork.
- [ ] **Non-root clipboard fallback.**
  - **Do not implement the old AccessibilityService-only plan.** Android 10+ returns `null` from
    `ClipboardManager` for background apps that are neither focused nor the default IME; enabling an
    accessibility service alone is not a documented exemption.
  - A real M3 fallback now needs an explicitly platform-authorised route (for example, an opt-in
    input-method design if that UX is acceptable) or a future Android API. Make that product decision
    before adding accessibility privileges that would not actually solve the problem.
- [x] **Windows notification actions / inline reply.**
  - Android mirrors only bounded action descriptors; PendingIntents never leave the phone.
  - Windows uses the already-resident toast thread's `ToastNotification::Activated` callback; no
    COM local server, helper process, polling loop, or extra resident UI was added.
  - A click is sent only through the currently live Noise session. Android resolves the current
    `StatusBarNotification` on demand and refuses stale index/label/RemoteInput targets.
  - Real device E2E verified both a free-form reply (`REPLY=Conduit reply E2E`) and a normal
    `Mark read` PendingIntent (`MARK`). Action-list changes silently rebuild the same Windows tag.
- [ ] **MessagingStyle conversation history.**
  - Revisit only if title/body/avatar rendering proves insufficient.

## Pending verification

### Release / lifecycle gates

- [ ] **M0: 48-hour same-LAN endurance run.**
  - After quiescence: no net thread, handle/FD, socket, or session-lifecycle growth.
  - `created == closed` / `opened == closed`; one active session may create a +1 gap while live.
  - Requires a true same-LAN phone/Windows setup; the saved `www` Wi-Fi is currently a foreign LAN.
- [ ] **M2: extended network-flap campaign.**
  - Repeat Wi-Fi <-> cellular, hotspot, default-network changes, phone reboot, and Relay re-parking.
  - Exercise the repaired Windows parked-socket keepalive path.
  - Keep Android socket/anon-inode FD classification in the evidence.
- [ ] **Long-duration Relay + Mihomo stability.**
  - Verify the Windows SOCKS5 Relay path survives proxy restarts, network changes, and long idle periods.
  - Verify fallback/recovery when the configured proxy is temporarily unavailable.
- [ ] **Deploy and validate the additional compatible Relay nodes.**
  - Current production fleet has only TYO listening on 41113; US/WA/JP are not yet running Conduit Relay.
  - Deploy only with explicit outward-facing approval, then configure Windows `CONDUIT_RELAYS` and
    Android `files/relays.txt`, and exercise real cross-node failover.

### Feature-specific verification

- [ ] **Visually confirm Android light/dark system-bar contrast while the phone is unlocked.**
  - **Dark/night side is now visually confirmed** on the unlocked Conduit Activity: dark surface,
    light status-bar glyphs and light navigation gesture bar render correctly.
  - The light/day side still needs a visual check. Compiled day resources already request dark
    status/navigation glyphs; do not substitute lockscreen SystemUI appearance for that final check.
- [ ] **Nagram XF contact-avatar end-to-end proof.**
  - A genuine target-device Nagram X notification was inspected structurally: it carried
    `Notification.EXTRA_MESSAGES` while `android.largeIcon=null`, so the former large-icon-only
    extractor could never have shown its contact face.
  - Android now falls back to the newest public `MessagingStyle.Message.senderPerson.icon` without
    AndroidX, hidden APIs, shortcut queries or polling. A sender-icon-only fixture produced an exact
    PNG match in the Windows `faces/` cache, proving Android extraction → Noise → Windows avatar.
  - Still pending only the opportunistic check on the **next naturally posted Nagram notification**.
    Do not replay an existing private notification merely to force this checkbox green.
- [ ] **Camera-photo -> Windows toast -> Snipping Tool regression.**
  - Re-run an interactive real-device capture after the recent shared capture/toast changes.
- [x] **Direct Share desktop-name refresh.**
  - `cmd shortcut get-shortcuts com.conduit.sync` initially reported `shortLabel=LOG`.
  - With the same desktop identity and only a temporary daemon `COMPUTERNAME` environment override,
    the next encrypted handshake changed the shortcut to `CONDUIT-RENAME-TEST`; restoring the normal
    daemon changed it back to `LOG`. The Windows machine name, pairing key and stored peer id were not
    modified.
  - The current APK had already been reinstalled earlier in this pass and the shortcut was present
    afterward, so republishing after app update/reinstall is also covered. Destructive unpair/re-pair
    is not needed just to prove the name-update mechanism.
- [x] **Restrictive `content://` provider sharing.**
  - A temporary Android fixture exposed a **non-exported** provider with `grantUriPermissions=true`.
    Direct shell access was rejected with `SecurityException: provider ... is not exported`.
  - The fixture then explicitly shared the same URI to Conduit's `ShareActivity` with a one-time read
    grant. `ShareActivity` forwarded the grant to `SyncService`; Windows received the 1 MiB private
    file and its SHA-256 exactly matched the provider source. The fixture and artifacts were removed.
- [x] **Large bidirectional file-transfer stress.**
  - 64 MiB PC→phone completed through TYO/Mihomo in 20.33 s with exact SHA-256 and remote
    publication ACK.
  - The same 64 MiB file phone→PC took about 4 min 46 s, crossed the Relay heartbeat boundary,
    published atomically, kept the session linked, and matched SHA-256 exactly.
  - Progress throttling is unit/device-covered separately; prior interrupted 64 MiB receive and
    Windows finalisation tests already prove pending/partial/placeholder cleanup. Stress artifacts
    were removed from the phone, workspace and Windows Downloads after verification.

## Protocol / migration cleanup

- [ ] **Retire legacy 47-byte Relay preamble inference.**
  - Only after old clients are no longer expected and M2 reconnection evidence is sufficient.
  - Keep the role-aware `CDT1 + role + rendezvous ID` contract.

## Deliberately out of scope

- General remote filesystem browsing/mounting unless product scope is explicitly changed.
- Telephony, SMS, screen mirroring, remote control/input, and media control.
