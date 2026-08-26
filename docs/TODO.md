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
- [ ] **Thin Fluent Windows control surface.**
  - Separate process from the daemon.
  - Surface link status, peer identity, diagnostics, Relay/proxy configuration, and basic controls.
- [x] **Explorer / shell file-send integration.**
  - HKCU file context menu uses a tiny on-demand `conduit-send.exe` GUI helper.
  - The helper starts no transport; it invokes the existing hidden `send <path>` CLI, which reuses
    the resident daemon's named pipe and waits for Android's publication ACK.
- [ ] **Relay/proxy settings UX.**
  - Replace machine-only environment-variable setup with an explicit user-facing configuration path.
  - Do not proxy LAN traffic.

### Android / notification extensions

- [x] **Fix light-surface Android status-bar icon contrast — implementation complete.**
  - Day theme explicitly requests dark status/navigation glyphs; night theme explicitly requests
    light glyphs. This is app-window system-bar appearance, not notification small-icon artwork.
- [ ] **Non-root clipboard fallback.**
  - Planned M3 AccessibilityService path for devices without KernelSU/LSPosed.
- [ ] **Windows notification actions / inline reply.**
  - Keep notification mirror semantics and lifecycle safety first.
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
  - APK resources are verified and installed, but the device was locked during this implementation
    pass, so do not substitute lockscreen SystemUI appearance for the Conduit Activity check.
- [ ] **Nagram XF contact-avatar end-to-end proof.**
  - Capture a genuine notification carrying a large contact icon and verify the Windows toast uses it.
- [ ] **Camera-photo -> Windows toast -> Snipping Tool regression.**
  - Re-run an interactive real-device capture after the recent shared capture/toast changes.
- [ ] **Direct Share desktop-name refresh.**
  - Verify desktop rename / re-pair / reinstall scenarios.
- [ ] **Restrictive `content://` provider sharing.**
  - Exercise providers whose URI grants cannot be trivially re-granted.
- [ ] **Large bidirectional file-transfer stress.**
  - Verify long sends across heartbeat boundaries on the current proxy-accelerated Relay path.
  - Confirm progress notifications remain correct after throttling.
  - Confirm interrupted transfers leave no `.part`, reserved 0-byte destination, or pending MediaStore row.

## Protocol / migration cleanup

- [ ] **Retire legacy 47-byte Relay preamble inference.**
  - Only after old clients are no longer expected and M2 reconnection evidence is sufficient.
  - Keep the role-aware `CDT1 + role + rendezvous ID` contract.

## Deliberately out of scope

- General remote filesystem browsing/mounting unless product scope is explicitly changed.
- Telephony, SMS, screen mirroring, remote control/input, and media control.
