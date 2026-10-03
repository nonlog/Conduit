# conduit

A deliberately small Android ↔ Windows companion. The current release includes:

1. **Text and image clipboard**, both directions, automatic where the platform permits.
2. **Android notifications → native Windows toasts**, including updates, dismissal, bounded
   actions and inline reply.
3. **Explicit file transfer**, both directions, through Android sharing, Windows Explorer and
   the Windows Share Target.
4. **Phone photos and screenshots → Windows capture toasts**, with optional Snipping Tool
   activation.
5. **Webpage sharing**, pairing management, LAN mDNS and encrypted Relay fallback.

The product boundary remains small. Telephony, SMS, screen mirroring, remote input, media
control and a mounted or browsable remote filesystem are out of scope.

## Why it exists

Phone Link works, but its native transport leaks: `libbasix-thread`, `pDCT IO thread`,
`udp(asio)` and `ICE Agent` threads accumulate until the phone is sluggish — observed at
1,069 targets / 1,755 threads / ~122% CPU / ~610 MB swap. conduit's differentiator is not
more features. It is: *the features you actually use, about as reliable, on a background that
stays simple, stable and transparent — and does not get heavier over weeks.*

So the hard requirements are non-functional:

- Android: idle CPU ≈ 0. No polling, no periodic network scans, no long WakeLocks, few
  threads, no session churn on network change.
- Windows: light long-running core, UI decoupled and non-resident, bounded threads / handles
  / sockets / memory over days.
- One connection lifecycle, provably closed. `connectionsCreated == connectionsClosed` after
  quiesce, or the difference is the single active session. This is the whole point.

## Shape

| Component | Language | Runs on |
|---|---|---|
| `android/` | Kotlin | phone |
| `windows/` | Rust + C# / Uno Platform | Windows (light resident daemon + on-demand WinUI 3 control surface) |
| `relay/` | Rust | a VPS, for when LAN is unavailable |
| `proto/` | protobuf | wire contract, single source of truth |

LAN direct connection is preferred. The relay is a dumb byte forwarder for when the phone is
on cellular or a foreign network: both ends dial it outbound over TCP, it pairs them by
`device_id` and copies opaque frames. Clipboard and notification content is end-to-end
encrypted with Noise, so the relay cannot read what it carries. No ICE, no STUN, no TURN —
that machinery is precisely the leak this project exists to avoid.

## Status

The current release is **v0.1.4**, tagged at `8987c1b` and built and published by GitHub
Actions. The successful release workflow was run `36249808688`; the preceding master build was
`36249356300`.

The release has working clipboard, notification, file-transfer, capture-toast, webpage-share,
pairing and Relay paths. The remaining evidence gates are deliberately separate from packaging:

- M0 still needs a true same-LAN 48-hour endurance run with balanced lifecycle/resource counts.
- M2 still needs a longer Wi-Fi/cellular/hotspot and Relay re-parking campaign.
- Long-duration Relay/proxy stability, final light-theme visual proof, and a few natural-device
  notification/camera checks remain open.

See `docs/TODO.md` for the current checklist and `docs/CONDUIT_HANDOFF.md` for the resume
checkpoint. Formal Android, Windows and Relay builds must continue to run in GitHub Actions.

## Provenance

Clean implementation. KDE Connect and Sefirah were read for protocol and architecture ideas;
no code was copied from them. That distinction is why the git history matters, and it is why
the license stays open as an option.
