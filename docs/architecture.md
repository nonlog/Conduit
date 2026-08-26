# Conduit architecture

> **Status:** implementation-oriented map as of 2026-08-26. It describes the current
> source tree and the role-aware relay protocol now deployed on the test/production path.
> Decision rationale and research evidence live in [decisions.md](decisions.md) and
> [research-synthesis.md](research-synthesis.md); this document avoids restating them.

## Purpose and boundaries

Conduit is a small Android ↔ Windows companion designed around one operational property:
a long-running link must remain boring.  It synchronises text and image clipboard contents,
mirrors Android notifications as native Windows toasts, and transfers explicit files in both
directions. New camera photos and phone screenshots are separately surfaced as Windows toasts
that can open the captured image in Snipping Tool.

It intentionally does **not** implement telephony, SMS, screen mirroring, remote control,
media control, filesystem mounting, remote input, or a general file browser. File transfer is
explicit and user-initiated rather than a mounted/browsable remote filesystem.

The design constraints are:

- **low Android idle cost is the primary product requirement**: Conduit was started because Link to
  Windows used too much phone CPU and caused lag, heat, and battery drain;
- no polling or recurring network scans on Android;
- no periodic throughput tests or background Relay benchmarks on Android;
- no long wake locks;
- a bounded number of sockets, worker threads, queued frames, and retained bytes;
- a single active peer session on each side; and
- after quiescence, `opened == closed` / `created == closed`.

## Topology

```text
                              LAN: mDNS + TCP 41112
┌───────────────────────┐  ─────────────────────────  ┌──────────────────────────┐
│ Android phone         │                               │ Windows desktop          │
│                       │                               │                          │
│ Compose activity      │                               │ conduit-daemon           │
│ SyncService           │   Noise XX encrypted frames  │ TCP listener             │
│ Notification listener │ ◀──────────────────────────▶ │ clipboard bridge         │
│ Content observers     │                               │ native toast thread      │
│ ShareActivity         │                               │ file receiver            │
└───────────────────────┘                               └──────────────────────────┘
         │                                                          │
         │ foreign LAN / cellular                                  │
         └──── outbound TCP ─────┐                    ┌────────────┘
                                  ▼                    ▼
                         ┌──────────────────────┐
                         │ Conduit relay        │
                         │ TCP 41113            │
                         │ rendezvous + splice  │
                         │ opaque byte copying  │
                         └──────────────────────┘
```

The relay is not a proxy protocol endpoint after rendezvous.  It holds neither a Noise
static key nor protobuf support and forwards opaque bytes with `copy_bidirectional`.
Noise runs end-to-end between phone and desktop; relay operators cannot read clipboard,
notification, image, or file contents.

## Components

| Area | Main pieces | Responsibility |
| --- | --- | --- |
| Android UI | `MainActivity`, `History`, `Settings`, `TransferStatus` | Compose status and transfer progress, peer identity, a separate searchable clipboard-history page, connect/disconnect, and user-owned notification-content privacy choice. |
| Android service | `SyncService`, `LinkTileService` | Owns the link for the app process; receives clipboard/default-network events; starts/stops discovery, camera/screenshot observation and reconnect scheduling; exposes the same connect/disconnect intent through a Quick Settings tile. |
| Android transport | `Link`, `WireSession`, `Noise` | One socket/session at a time; one reader thread and one single-thread sender executor; Noise XX framing and dispatch. |
| Android integration | `ClipboardHook`, `NotificationRelay`, `Discovery`, `Photos`, `Screenshots`, `ShareActivity` | LSPosed clipboard permission escape, system notification callbacks, bounded mDNS discovery, edge-triggered camera/screenshot observation, and URI-grant-safe sharing. |
| Wire contract | [`../proto/conduit.proto`](../proto/conduit.proto) | Single protobuf schema consumed by Android and Rust. |
| Windows daemon | `main.rs`, `wire.rs`, `clip.rs`, `image.rs`, `file.rs`, `control.rs` | mDNS advertising, LAN listener, relay parking, Noise session, native clipboard bridge, bounded image/file receive paths, and a local named-pipe command seam for desktop→phone file sends. |
| Windows notifications | `toast.rs` | Dedicated COM/MTA toast owner, AUMID registration, icon/avatar cache, notification update/removal, capture/Snipping-Tool activation, and file activation. |
| Relay | `relay/src/main.rs` | Fixed-size rendezvous preamble validation, one waiting socket per key, and blind TCP splicing. |

## Session lifecycle and routing

### Route selection

1. `SyncService` receives a default-network callback or an explicit Connect action.
2. On Wi-Fi or Ethernet, `Discovery` starts one eight-second mDNS burst for `_conduit._tcp`.
   It stops on the first resolved desktop or its deadline.
3. On mobile data, and after an empty LAN burst, the phone dials the relay directly.
4. Relay use requires the desktop's remembered device ID, obtained from a prior completed
   direct handshake.  An unpaired phone must pair on a LAN first.

### Multi-Relay selection: passive, sticky, battery-first

Multi-Relay support must not turn routing into a background benchmark service. The desktop is the
powered side, so it may park one responder on every configured Relay. Android remains the selection
owner and keeps only one active Relay/session. This guarantees that whichever endpoint the phone
chooses already has the same desktop waiting there without making the phone maintain several idle
TCP paths.

Android does **not** periodically ping, speed-test, or probe Relay nodes. It updates per-endpoint
quality only from work that was going to happen anyway:

- TCP/Relay/Noise connection success or failure and time-to-session-up;
- abnormal session close, heartbeat/PONG timeout, or clean long-lived session evidence;
- real screenshot, camera-photo, image-clipboard, and explicit-file transfer completion/throughput;
- natural default-network changes and reconnect attempts.

The selector is intentionally sticky. A healthy current Relay stays selected even if another node
has slightly better latency. Reliability and real end-to-end content performance dominate; RTT is
only a tie-breaker because a low-RTT path can still have loss/retransmission severe enough to make
actual transfers unusable. Repeated failures put an endpoint into a cooldown. Cooldown expiry does
not schedule a probe: it merely makes the endpoint eligible again the next time a real reconnect is
already necessary.

On a reconnect, Android tries the historically best eligible candidate first with a bounded
connection/handshake deadline, then moves to the next candidate if it fails. Unknown/stale endpoints
are explored only at these natural reconnect points or after observed degradation; a healthy idle
session is never torn down just to gather a better score. Per-network history should remain coarse
(transport/VPN class plus age-decayed EWMA) rather than requiring extra Android permissions solely
to identify SSIDs or perform routing telemetry.

The implementation reads Android's optional app-private `files/relays.txt` once when
`SyncService` starts. Each non-comment line is:

```text
id|hostname|port|optional-fallback-ipv4
```

With no file (or no valid entries), TYO remains the single production default. Windows accepts a
comma/semicolon-separated `CONDUIT_RELAYS` and parks one independent responder task per endpoint;
the older singular `CONDUIT_RELAY` remains a compatibility fallback. An explicitly empty
`CONDUIT_RELAYS` disables Relay parking.

`RelayQualityStore` persists only event-driven observations in `relay-quality.txt`: successful or
failed real dials, short unstable sessions, and completed real image/file goodput. Two consecutive
dial failures put one endpoint into a 30-minute cooldown. A failed candidate advances to the next
one inside the reconnect that is already underway; the cooldown clock itself never schedules work.

This design deliberately favours "boring while idle" over continuously chasing the theoretical
fastest Relay. The dominant use cases are clipboard/notification traffic and phone photo/screenshot
handoff, so stable low-wakeup connectivity matters more than maximum bulk-file benchmark throughput.

Windows may optionally route **only** its relay dial through SOCKS5 by setting
`CONDUIT_RELAY_PROXY=socks5://host:port`. The SOCKS CONNECT request carries the relay hostname, so
Mihomo/Clash DOMAIN rules remain usable. LAN listener traffic never enters this path and therefore
remains direct. A parked Windows responder enables kernel TCP keepalive before it begins waiting;
this prevents a relay/NAT path that died silently from leaving `park()` blocked forever on a local
socket that still appears `ESTABLISHED`.

The relay remains configured by hostname (`tyo.414222.xyz`). On the tested phone, Bettbox uses
the benchmark range `198.18.0.0/15` as fake-IP DNS. During an underlying Wi-Fi/cellular handover,
that local fake mapping can accept a connect and fail it before any preamble reaches TYO. Android
therefore treats **only** a `198.18/15` answer for the relay as synthetic and substitutes the
relay's pinned public fallback (`138.3.214.175`). Ordinary public DNS answers are left untouched.
The resulting socket is still a normal Android `Socket`, so VPN/routing policy continues to own
the data path; the fallback bypasses the broken fake address, not the VPN.

A network loss closes the active socket but preserves the Android `Link` object and its
single sender executor.  This is deliberate: reconnecting reuses those resources instead of
creating a fresh transport stack.  Retries are one bounded `Handler` callback with
exponential backoff; `uptimeMillis` does not advance while the phone is asleep, and no wake
lock is acquired.

### Ownership rules

```text
Android Link
  sender executor ── only caller of WireSession.send
  reader thread   ── only caller of WireSession.recv
  Socket().use {} ── owns every session socket

Windows daemon
  one active serve task ── a newer arrival aborts and awaits the prior task
  SessionGuard::Drop    ── increments the closed counter on every exit path
```

The reader and writer never share Noise counters concurrently. A PONG requested by the Android
reader is posted to the sender executor rather than written directly. On Windows, receive and
send ciphertext have separate fixed scratch buffers: a heartbeat is allowed to cancel a partial
receive, send a PING, then resume the same ciphertext frame without overwriting its prefix. On
Android, closing the socket interrupts the blocked reader; `Socket().use {}` then performs the
common cleanup. On Windows, `SessionGuard` makes a normal return, error, panic, or task abort
count as a closed session.

Idle work is blocked I/O rather than a poll loop.  The desktop sends an application PING when
**it** has been silent; the phone uses a matching read deadline.  The idle cadence is
path-specific: LAN is faster, while relay traffic trades delayed failure detection for fewer
cellular radio wakes.

## Data plane

### Common wire format

After Noise XX (`Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue `conduit/1`) completes, every
record is:

```text
[4-byte big-endian ciphertext length][Noise transport ciphertext]
```

The encrypted plaintext is one protobuf `Envelope`.  Frame length is validated before an
allocation.  `MAX_FRAME` is 65,535 bytes and usable plaintext is 65,519 bytes after the
ChaChaPoly tag, so payload producers must account for protobuf overhead.  This is why image
and file chunks are 32 KiB rather than 64 KiB.

| Flow | Android source | Wire messages | Windows result |
| --- | --- | --- | --- |
| Text clipboard | Primary-clip callback, LF normalisation | `CLIP_TEXT` | Native clipboard write; matching text is suppressed on the return path. |
| Image clipboard | `content://` URI read on sender executor | `CLIP_IMAGE_HEADER`, `CLIP_IMAGE_CHUNK` | Validated bounded assembly, PNG normalisation, native clipboard write. |
| Android notification | `NotificationListenerService` callback | `NOTIF_NEW`, `NOTIF_UPDATE`, `NOTIF_REMOVE` | Native Windows toast, in-place update, then removal from history. |
| Phone file share | Share sheet URI grant | `FILE_OFFER`, `FILE_CHUNK` | Sequential disk write to Downloads; a completed-transfer toast opens the containing folder. |
| Desktop file send | `conduit-daemon send <path>` → local named pipe | `FILE_OFFER`, `FILE_CHUNK` | Android streams into a pending MediaStore Downloads row and publishes only after exact byte/chunk completion. |
| Camera photo | `MediaStore` `ContentObserver` | Image header/chunks with `photo=true` | Staged hero image; toast activation uses a shared-storage token to open Snipping Tool. |
| Phone screenshot | Dedicated `MediaStore` `ContentObserver` filtered to `Pictures/Screenshots/%` + `Screenshot_` | Image header/chunks with `photo=true,screenshot=true` | Screenshot-specific native toast; click hands the staged image to Snipping Tool through the same shared-storage-token protocol. |

Notification mirroring intentionally has no offline queue.  A notification that did not have
a live desktop session is stale by the time a future session arrives.  The phone filters its
own, ongoing, group-summary, media, and Android-silent notifications before any bytes are
sent.  App icons are sent once per package; large icons supply contact/avatar art when
available, subject to a frame-safe byte cap.

Android user-visible notifications are separated by purpose. The foreground link notification
uses the `Link` channel and says `Linked to <desktop name>` only while a Noise session is up; it
is removed on disconnect/retry. File transfer progress uses a separate `File transfers` channel,
with distinct upload/download status-bar icons, its own notification IDs, and byte/percent
progress. The Quick Settings tile is a control surface only and does not own a second transport.

`Photos` and `Screenshots` are deliberately separate event sources. `Photos` accepts `DCIM/%`;
`Screenshots` accepts only new rows under `Pictures/Screenshots/%` whose display name starts
with `Screenshot_`, consuming MediaStore ids once so scanner re-notification does not duplicate
a capture. Screenshots set the legacy `photo=true` bit as a backward-safety marker so an old
desktop still keeps them out of the clipboard, while the explicit `screenshot=true` field gives
new peers correct user-facing semantics.

## State and resource bounds

| Resource | Bound / rule |
| --- | --- |
| Android active transport | One socket, one reader thread, one sender executor per service lifetime. |
| Android send queue | 64 entries, discard-oldest; fresh clipboard/notification state is preferred to stale state. |
| mDNS | One discovery burst, stopped on success or after 8 seconds. |
| Clipboard history | 100 previews, each at most 200 characters; stored in `filesDir/history.json`. |
| Android settings | Two `key=value` settings in `filesDir/settings.txt`; `SharedPreferences` is not used on the tested device. |
| Images | Header validation and a 10 MiB ceiling; chunked transfer prevents oversized Noise frames. |
| Files | 32 KiB chunks, 512 MiB maximum. One incoming file assembly per session per receiver; outbound sends are serialized. Android uses a pending MediaStore row and Windows uses a scratch file, both deleted if the session/transfer fails before publication. |
| Windows toast cache | App icon files are package-keyed; contact-avatar cache is capped at 128 files. |
| Capture activation | Camera photos and screenshots share one staged capture file, one toast tag, and one shared-storage token at a time. |
| Relay waiters | Maximum 256 waiting sockets, with bounded preamble read deadline. |

## Security and trust boundaries

- **Peer identity and relay confidentiality:** Noise XX authenticates the completed session
  by static key.  The rendezvous ID is a base64url hash of a public static key and is not a
  secret.  Compare the displayed fingerprint out of band when pairing.
- **Relay:** treats all post-preamble bytes as opaque ciphertext.  It should never gain a
  protobuf or Noise dependency.
- **Android clipboard:** background clipboard access depends on the intentionally scoped
  LSPosed hook on the rooted target device. Android 10+ returns no clipboard data to an ordinary
  background app unless it currently has input focus or is the default IME; an
  `AccessibilityService` by itself does not remove that `ClipboardManager` restriction. A future
  non-root fallback therefore needs a genuinely platform-authorised input path (or a future API),
  not a cosmetic accessibility-service wrapper around the same blocked call.
- **Android notifications:** notification content may be redacted by Android before Conduit
  receives it.  The per-install `RECEIVE_SENSITIVE_NOTIFICATIONS` AppOp is distinct from
  Conduit's own *Hide notification content* setting.
- **Share URIs:** an incoming share can reference an unreadable or non-regrantable `content://`
  URI.  `ShareActivity` forwards a read grant with `ClipData`, uses `newRawUri`, and catches
  failure so a bad share cannot terminate a live session.
- **Remote content:** image/file headers are validated before allocation or writing. File names
  are sanitised and reserved with `create_new`; the final scratch→destination publication owns
  and removes its placeholder on failure, and `Drop` cleans an unpublished scratch even after
  its handle was closed. A toast for a received file opens its folder, never executes the
  peer-chosen file. Toast XML escapes peer-derived markup.
- **Persistence:** identity, settings, peer metadata, and history are app-private files.
  History uses a bounded whole-file rewrite; it is intentionally not yet an atomic rename.

## Relay preamble: deployed contract and compatible migration

The legacy preamble remains accepted during the migration window:

```text
CDT1 + 43-character base64url desktop rendezvous ID
```

Commit `86a2b86` implements the role-aware 48-byte form in both endpoint writers, and the
installed Android/Windows endpoints now use it:

```text
CDT1 + role byte + 43-character rendezvous ID
       > phone / Noise initiator
       < desktop / Noise responder
```

The role byte prevents a reconnecting phone from being spliced to its own stale parked
initiator socket. The migration relay keys waiters by `(rendezvous ID, role)` and replaces a
same-role stale waiter. To permit a server-first rollout, it also accepts the 47-byte form:
byte five is unambiguously either an explicit role (`>` / `<`) or the first base64url id byte.
For a legacy peer only, the relay then peeks for up to one second after the id. An old phone
immediately starts Noise message 1 and is therefore an initiator; an old desktop stays silent
until a partner speaks and is therefore a responder. `peek` consumes nothing, so the first
Noise byte remains in the socket.

This makes all four combinations interoperable **on the compatible relay**: old↔old,
old-phone↔new-desktop, new-phone↔old-desktop, and new↔new. The safe rollout order is therefore:

That rollout was completed on 2026-08-26: compatible server first, old↔old verification, Windows
upgrade with a mixed session, then Android upgrade and new↔new verification. The production relay
also demonstrated explicit same-role waiter displacement under an isolated test rendezvous. The
remaining protocol cleanup is simply to remove legacy inference after old clients are retired and
M2 reconnection evidence is sufficient.

## Related documents

- [README](../README.md) — product-level summary and original milestone framing.
- [Decisions](decisions.md) — rationale, platform research, and earlier implementation notes.
- [Development guide](development.md) — build, test, device, and git workflow.
- [Progress record](progress.md) — dated evidence, caveats, and current repository state.
- [Backlog](backlog.md) — explicitly pending work, including the relay migration and endurance
  evidence.
