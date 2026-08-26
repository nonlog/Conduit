# Conduit backlog

> Priorities reflect the current 2026-08-26 working tree and deployed test environment. They are intentionally ordered by
> correctness and lifecycle safety—not by how easy a UI item would be to add.

## P0 — correctness and release safety

### 1. Establish endurance evidence

- **M0:** 48-hour LAN run with zero thread, handle/FD, socket, and memory growth after
  quiescence; `created == closed` except for the one active session.
- **M2:** repeat real 5G ↔ Wi-Fi/hotspot/default-network transitions and confirm the same
  invariant across reconnects and relay re-parking.
- Treat logs as evidence only when they are current.  Measure live process state and force real
  peer/socket loss; removing a forwarding rule alone does not close an established session.
- `scripts/soak.ps1` now supplies repeatable samples, lifecycle gaps, PID stability and Android
  coverage, and can follow ADB transport changes by physical-device serial. Short self-tests,
  including a live ADB transport failover, passed. It also classifies Android socket/anon-inode/
  APK/ashmem descriptors so notification resource caching is not mistaken for a network leak.
- The first real foreign-Wi-Fi↔cellular campaign is green after fixing Bettbox fake-IP relay DNS:
  six transitions kept lifecycle counts balanced, and a classified follow-up showed socket and
  anon-inode FD delta zero. What remains is a longer M2 campaign including hotspot/default-network
  variants, plus the independent 48-hour M0 LAN run.

## P1 — verification and operational quality

### 2. Prove Nagram XF contact-avatar rendering end to end

The extractor and Windows cache path exist.  Capture a genuine incoming Nagram XF notification
with a large contact icon and verify that the Windows toast shows the contact/avatar rather
than only the app icon.  If it fails, preserve payload/log evidence before changing extraction.

### 3. Verify Android light-surface status-bar icon contrast

The implementation now explicitly selects dark day-theme and light night-theme status/navigation
glyphs. The remaining work is an unlocked visual check of the Conduit Activity; do not use lockscreen
SystemUI appearance as evidence and do not regress the foreground-service notification artwork.

### 4. Windows daemon autostart at sign-in — complete

HKCU Run starts the daemon hidden in the interactive user session. Binding port 41112 happens before
long-lived workers and doubles as a zero-extra-resource single-instance gate; duplicate startup was
live-tested and exits immediately.

### 5. Thin Windows control surface — complete

`conduit-control.exe` is now a separate non-resident GUI-subsystem process reading event-written
status/config state on demand. It owns no transport, tray, watcher, or timer and exits fully with the
window. Its Fluent pass is complete with native Win32/DWM/Common Controls, Windows theme/accent and
correct 125% DPI sizing; no heavier resident UI framework was introduced.

### 6. Re-run device-specific persistence/permission checks after platform changes

- Verify `filesDir` persistence after an app reinstall/update.
- Re-grant and test `RECEIVE_SENSITIVE_NOTIFICATIONS` after every reinstall on Android 15+.
- Keep the in-app hide-content setting separate from Android platform redaction.
- Test Direct Share name refresh after pairing/desktop rename and a sharing provider that
  supplies a restrictive `content://` URI.

## P2 — intentionally deferred

| Item | Condition to revisit |
| --- | --- |
| Full desktop GUI beyond the thin control surface | Only if the lightweight surface proves insufficient. |
| Non-root clipboard fallback | M3 product/design decision first. Android 10+ blocks ordinary background clipboard reads unless focused/default IME; do not ship the former AccessibilityService-only idea as a fake fallback. |
| `MessagingStyle` history | Only if ordinary title/body plus avatar evidence proves insufficient. |
| General file browser/mount | Out of scope unless product scope is explicitly changed. |
| Telephony, SMS, screen mirroring, remote control/input, media control | Permanently out of scope. |

## Completed or already represented work

The architecture, development workflow, progress evidence, and this backlog were recorded in
this documentation pass.  Keep them current rather than opening a second parallel set of
planning documents. Existing source already covers text/image clipboard, native notification
toasts, notification filtering/privacy control, bidirectional explicit file transfer, Direct Share
desktop naming, searchable Android clipboard history, a Quick Settings link toggle, camera-photo
toast activation, screenshot → native toast → Snipping Tool, Explorer send-to-phone integration,
Windows sign-in autostart, the on-demand control surface, and notification actions/inline reply.
The screenshot path was verified on the target CPH2573 with clipboard non-interference and a
duplicate scanner callback check; consult [progress.md](progress.md) for the evidence level and
caveats rather than treating feature presence as blanket milestone completion.

Windows relay parking now enables TCP keepalive before waiting so a server-reaped responder cannot
leave the desktop permanently stuck on a zombie local `ESTABLISHED` socket. The Windows relay dial
also supports an optional SOCKS5 path (`CONDUIT_RELAY_PROXY`) without proxying LAN traffic; the
current test machine uses Clash Party/Mihomo `127.0.0.1:7891`, with a large measured relay-speed
improvement over DIRECT.

Two prior P0 investigations are now folded into completed evidence rather than left as parallel
backlog items. The old relay's transient fresh-park refusal was reproduced during a daemon
restart and matches the same-role stale-waiter failure already addressed by the compatible
role-byte migration. The compatible relay was deployed server-first on 2026-08-26; old↔old,
old-phone↔new-desktop, and new↔new sessions all succeeded, three real phone restart cycles stayed
clean, and an isolated live-server probe confirmed same-role waiter displacement. Keep the
one-second legacy inference path only until the old-client compatibility window is intentionally
closed. The supposed 259,737-byte
missing-file incident was replayed with the exact phone screenshot and proved to be an early
mid-transfer check: the final file appeared at eight seconds with an exact SHA-256 match. Commit
`d5554ec` additionally closes the genuine cleanup hole found in finalisation-error paths.
