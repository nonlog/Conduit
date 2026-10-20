# Transfer and LAN reliability contract

This work continues the uncommitted `codex/transfer-lan-reliability` worktree from
the released v0.1.6 master. It does not revert to the older local development
checkout or reintroduce the rejected sensitive-notification setting.

## Transport ownership

- A TCP connection or port probe is only a candidate. Windows completes bounded
  Noise/identity admission and rechecks the pairing policy before replacing the
  trusted session. Failed, stale or unauthenticated candidates cannot cancel its
  transfers. Pending candidates are capped at four.
- Each file has an unpredictable ID and belongs to one authenticated session.
  A stale result, wrong-ID chunk or cancellation cannot mutate another file or
  a replacement session.
- One file per direction streams at a time, with a bounded FIFO of pending work.
  A new user-selected photo is a queued file, not a reason to reconnect. Clipboard,
  notification, heartbeat and cancellation traffic runs between complete encrypted
  frames. Never abort an individual write and reuse that partially written stream.
- The FIFO head retains its place until the receiver reports publication. Reaching
  100% locally means **waiting for confirmation**, not success. A failed source,
  refusal, cancellation, disconnect or confirmation timeout is not completion.
- Cancellation is per-ID. Publication wins a late cancellation race; cancelling
  an unsent queued item does not open its source or affect the active file.
- PairRequest capability version 1 enables FILE_CANCEL (33) and acknowledged
  results. FileResult adds a cancelled flag. Old peers remain readable; unsupported
  active cancellation is not emulated by killing unrelated traffic.
- Both sides bound queues/history, stream 32 KiB chunks and validate size/count/ID
  arithmetic. Failed partial files are not published. Windows does not overwrite
  an existing destination name.

## Discovery and UI

- LAN NSD uses a bounded event-triggered burst on physical Wi-Fi or Ethernet,
  including a hotspot without validated Internet. Stored peer IDs are filtered
  before address selection and authenticated again by Noise. Direct sockets bind
  the same Network as discovery. When a VPN DNS proxy injects 198.18/15 fake
  `.local` addresses into NSD's resolution, a single bounded mDNS A/QU query
  over the physical discovery Network recovers the advertised on-link IPv4;
  there is no continuous scanner, hard-coded desktop address or bypass of Noise.
- Stale callbacks and default-VPN changes must not relabel a live route or tear
  down a valid physical socket. Optional Relay-to-LAN promotion waits for active
  file transfers and pending publication results.
- Windows managed firewall rules allow only this daemon's LAN TCP and mDNS UDP on
  LocalSubnet, including the hotspot's Public profile. They never disable the
  firewall or expose management ports to the Internet.
- Android send controls have equal width/height and bounded one-line labels.
  Both endpoints show per-file progress, queued/waiting/terminal states and cancel
  controls. Desktop transfer state remains visible from all four pages.
- Preserve the native WinUI/Mica/system-accent UI and bidirectional arrow assets.
- Windows upgrades compare the sparse Share Target MSIX manifest version against
  the registered version. An older identity must be updated in place to prevent
  the new WinUI binary loading old XAML PRI resources. Registration uses temporary
  signing-certificate trust with administrator approval when needed, and removes
  only trust it added; existing user data and pairing are not replaced.

## Validation gate

All compilation, unit tests and installable artifacts use **GitHub Actions**,
not the PC or VPS. Static formatting/XML/TOML checks alone are not acceptance.
Before release test a large file followed by a photo, simultaneous opposite-
direction transfers, queued/active cancellation from both devices, disconnect
cleanup, matching final hashes, duplicate names, unauthenticated TCP probes, LAN
discovery, and notification/clipboard behavior on the actual paired devices.

This patch has no resume database. A disconnected transfer is reported failed,
its partial output is discarded and the user can explicitly retry. Do not claim
it was saved or silently retransmit it to a different paired device.

Device smoke checks completed before release: 3 MiB transfer in both directions
with identical SHA-256, 24 MiB audio outbound with a 3 MiB photo incoming while
audio was active (both completed and passed SHA-256), and per-ID cancellation
from both endpoints leaving no partial destination files. The updated physical
mDNS query passed parser/selection tests against captured response packets; a
fresh phone/hotspot LAN run is still required to verify it end to end.
