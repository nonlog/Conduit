# Conduit backlog

> Priorities reflect the current 2026-08-25 working tree.  They are intentionally ordered by
> correctness and lifecycle safety—not by how easy a UI item would be to add.

## P0 — correctness and release safety

### 1. Roll out the compatible relay role-byte migration safely

Commit `86a2b86` completes the local protocol work. The relay accepts both the deployed 47-byte
`CDT1 + rendezvous ID` form and the new 48-byte `CDT1 + role + rendezvous ID` form. Legacy peers
are classified without consuming Noise bytes, Android now emits `>` (initiator), Windows emits
`<` (responder), and the relay suite covers old↔old plus both mixed upgrade orders. The stale
same-role bug is covered for both explicit and legacy phone reconnects.

Remaining rollout work:

1. With explicit approval, deploy the **compatible relay first** while installed endpoints are
   still using the old 47-byte form.
2. Verify those old endpoints continue to connect through the new server.
3. Upgrade one endpoint and prove a mixed old/new session, then upgrade the other endpoint and
   prove new/new operation.
4. Reproduce a stale same-side reconnect on the real relay and prove it displaces rather than
   self-splices; also prove the normal opposite-role splice.
5. Fold the result into M2 cellular ↔ Wi-Fi/hotspot flap evidence.
6. Retire the one-second legacy inference path only after old clients are no longer expected.

**Do not install/restart the role-aware client builds against the currently deployed old relay.**
It does not understand the extra role byte. No production rollout has been performed yet.

### 2. Establish endurance evidence

- **M0:** 48-hour LAN run with zero thread, handle/FD, socket, and memory growth after
  quiescence; `created == closed` except for the one active session.
- **M2:** repeat real 5G ↔ Wi-Fi/hotspot/default-network transitions and confirm the same
  invariant across reconnects and relay re-parking.
- Treat logs as evidence only when they are current.  Measure live process state and force real
  peer/socket loss; removing a forwarding rule alone does not close an established session.

### 3. Diagnose relay fresh-park refusal/recovery

A production relay period appeared to refuse new parks and later recovered.  Capture current
server process state, socket count, and timestamped logs around the next recurrence.  Determine
whether the cause is waiter-map capacity, stale connections, upstream networking, or logging
misinterpretation before changing timeout/retry policy.

### 4. Resolve the logged-but-missing received-file incident

A roughly 259,737-byte file was logged as written yet was not found on disk.  Reproduce with a
known destination and timestamped trace, then account for all paths: Downloads resolution,
rename, collision suffixes, toast location, antivirus/indexer involvement, and post-transfer
observation timing.  Do not describe phone → PC file transfer as fully reliable until this is
understood.

## P1 — verification and operational quality

### 5. Prove Nagram XF contact-avatar rendering end to end

The extractor and Windows cache path exist.  Capture a genuine incoming Nagram XF notification
with a large contact icon and verify that the Windows toast shows the contact/avatar rather
than only the app icon.  If it fails, preserve payload/log evidence before changing extraction.

### 6. Correct Android light-surface status-bar icon contrast

The system notification's monochrome `ic_stat_link` visual size was already corrected.  This
remaining issue is different: the Android app’s own light surface has incorrect white
status-bar icons.  Fix only the app-window system-bar appearance; do not regress the
foreground-service notification artwork.

### 7. Add Windows daemon autostart at sign-in

Keep the daemon headless and non-resident in UI terms.  Design the smallest Windows-native
sign-in mechanism that starts one daemon process, reports failure honestly, and avoids creating
multiple listeners/relay parkers on repeated login or manual launch.

### 8. Add a thin Fluent Windows control surface

The intended shape is a separate, non-resident tray/settings process, not a UI framework linked
into `conduit-daemon`.  It should eventually surface pairing identity, status, diagnostics, and
configuration without becoming another transport owner.  Reference Sefirah only for visual
language; preserve the clean implementation and non-GPL licensing options.

### 9. Re-run device-specific persistence/permission checks after platform changes

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
