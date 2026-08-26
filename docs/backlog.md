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
  including a live ADB transport failover, passed; what remains is the real 48-hour run and
  deliberate network transition campaign, not more sampler scaffolding.

## P1 — verification and operational quality

### 2. Prove Nagram XF contact-avatar rendering end to end

The extractor and Windows cache path exist.  Capture a genuine incoming Nagram XF notification
with a large contact icon and verify that the Windows toast shows the contact/avatar rather
than only the app icon.  If it fails, preserve payload/log evidence before changing extraction.

### 3. Correct Android light-surface status-bar icon contrast

The system notification's monochrome `ic_stat_link` visual size was already corrected.  This
remaining issue is different: the Android app’s own light surface has incorrect white
status-bar icons.  Fix only the app-window system-bar appearance; do not regress the
foreground-service notification artwork.

### 4. Add Windows daemon autostart at sign-in

Keep the daemon headless and non-resident in UI terms.  Design the smallest Windows-native
sign-in mechanism that starts one daemon process, reports failure honestly, and avoids creating
multiple listeners/relay parkers on repeated login or manual launch.

### 5. Add a thin Fluent Windows control surface

The intended shape is a separate, non-resident tray/settings process, not a UI framework linked
into `conduit-daemon`.  It should eventually surface pairing identity, status, diagnostics, and
configuration without becoming another transport owner.  Reference Sefirah only for visual
language; preserve the clean implementation and non-GPL licensing options.

### 6. Re-run device-specific persistence/permission checks after platform changes

- Verify `filesDir` persistence after an app reinstall/update.
- Re-grant and test `RECEIVE_SENSITIVE_NOTIFICATIONS` after every reinstall on Android 15+.
- Keep the in-app hide-content setting separate from Android platform redaction.
- Test Direct Share name refresh after pairing/desktop rename and a sharing provider that
  supplies a restrictive `content://` URI.

## P2 — intentionally deferred

| Item | Condition to revisit |
| --- | --- |
| Desktop → phone file sending | Only when explicitly requested again; no GUI/right-click sender work before then. |
| Windows right-click file send / full GUI | After core transport, screenshot, and endurance work are proven. |
| Non-root clipboard fallback | M3, via an AccessibilityService path that can work without KernelSU/LSPosed. |
| Notification actions and inline reply | After the core notification mirror is stable and lifecycle evidence exists. |
| `MessagingStyle` history | Only if ordinary title/body plus avatar evidence proves insufficient. |
| General file browser/mount | Out of scope unless product scope is explicitly changed. |
| Telephony, SMS, screen mirroring, remote control/input, media control | Permanently out of scope. |

## Completed or already represented work

The architecture, development workflow, progress evidence, and this backlog were recorded in
this documentation pass.  Keep them current rather than opening a second parallel set of
planning documents. Existing source already covers text/image clipboard, native notification
toasts, notification filtering/privacy control, one-way Android share-sheet files, Direct Share
desktop naming, camera-photo toast activation, and screenshot → native toast → Snipping Tool.
The screenshot path was verified on the target CPH2573 with clipboard non-interference and a
duplicate scanner callback check; consult [progress.md](progress.md) for the evidence level and
caveats rather than treating feature presence as blanket milestone completion.

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
