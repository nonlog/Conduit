# conduit

A deliberately small Android ↔ Windows companion. Three features, and it does not grow:

1. **Text clipboard**, both directions, automatic — no send button.
2. **Image clipboard**, both directions — Windows screenshot pastes on the phone, and back.
3. **Android notifications → native Windows toasts**, including update and dismissal.

That is the whole product. Telephony, SMS, screen mirroring, remote input, file browsing and
media control are out of scope permanently, not "later".

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

Pre-M0. See `docs/decisions.md` for the plan and `docs/research-synthesis.md` for the
verified API-level research behind it.

| | Scope | Done when |
|---|---|---|
| M0 | LAN text clipboard, both directions | 48 h run, fd/handle/thread delta 0 |
| M1 | Image clipboard + notifications → toast | no new threads vs M0 |
| M2 | Relay on a VPS | survives cellular↔LAN flapping without leaking a session |
| M3 | AccessibilityService clipboard fallback | works with LSPosed absent |

## Provenance

Clean implementation. KDE Connect and Sefirah were read for protocol and architecture ideas;
no code was copied from them. That distinction is why the git history matters, and it is why
the license stays open as an option.
