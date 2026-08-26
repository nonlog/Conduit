# Conduit architecture

> **Status:** implementation-oriented map as of 2026-08-26. It describes the current
> source tree and the role-aware relay protocol now deployed on the test/production path.
> Decision rationale and research evidence live in [decisions.md](decisions.md) and
> [research-synthesis.md](research-synthesis.md); this document avoids restating them.

## Purpose and boundaries

Conduit is a small Android ↔ Windows companion designed around one operational property:
a long-running link must remain boring.  It synchronises text and image clipboard contents,
mirrors Android notifications as native Windows toasts, and currently accepts explicit
**phone → PC** file shares.  New camera photos and phone screenshots are separately surfaced
as Windows toasts that can open the captured image in Snipping Tool.

It intentionally does **not** implement telephony, SMS, screen mirroring, remote control,
media control, filesystem mounting, remote input, or a general file browser.  Desktop →
phone file sending is deliberately deferred; it is not implied by the one-way share feature.

The design constraints are:

- no polling or recurring network scans on Android;
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
| Android UI | `MainActivity`, `History`, `Settings` | Compose status, peer identity, history, connect/disconnect, and user-owned notification-content privacy choice. |
| Android service | `SyncService` | Owns the link for the app process; receives clipboard and default-network events; starts/stops discovery, camera/screenshot observation, and reconnect scheduling. |
| Android transport | `Link`, `WireSession`, `Noise` | One socket/session at a time; one reader thread and one single-thread sender executor; Noise XX framing and dispatch. |
| Android integration | `ClipboardHook`, `NotificationRelay`, `Discovery`, `Photos`, `Screenshots`, `ShareActivity` | LSPosed clipboard permission escape, system notification callbacks, bounded mDNS discovery, edge-triggered camera/screenshot observation, and URI-grant-safe sharing. |
| Wire contract | [`../proto/conduit.proto`](../proto/conduit.proto) | Single protobuf schema consumed by Android and Rust. |
| Windows daemon | `main.rs`, `wire.rs`, `clip.rs`, `image.rs`, `file.rs` | mDNS advertising, LAN listener, relay parking, Noise session, native clipboard bridge, bounded image/file receive paths. |
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

The reader and writer never share Noise counters concurrently.  A PONG requested by the
reader is posted to the sender executor rather than written directly.  On Android, closing
the socket interrupts the blocked reader; `Socket().use {}` then performs the common cleanup.
On Windows, `SessionGuard` makes a normal return, error, panic, or task abort count as a
closed session.

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
| Camera photo | `MediaStore` `ContentObserver` | Image header/chunks with `photo=true` | Staged hero image; toast activation uses a shared-storage token to open Snipping Tool. |
| Phone screenshot | Dedicated `MediaStore` `ContentObserver` filtered to `Pictures/Screenshots/%` + `Screenshot_` | Image header/chunks with `photo=true,screenshot=true` | Screenshot-specific native toast; click hands the staged image to Snipping Tool through the same shared-storage-token protocol. |

Notification mirroring intentionally has no offline queue.  A notification that did not have
a live desktop session is stale by the time a future session arrives.  The phone filters its
own, ongoing, group-summary, media, and Android-silent notifications before any bytes are
sent.  App icons are sent once per package; large icons supply contact/avatar art when
available, subject to a frame-safe byte cap.

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
| Files | One transfer per session, 32 KiB chunks, 512 MiB maximum; partial files are deleted by `Drop`, including failures after the receive handle has already been closed for final publication. |
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
  LSPosed hook on the rooted target device.  The planned non-root fallback is separate.
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
