//! PNG <-> `CF_DIB` transcoding, via the codecs Windows already ships.
//!
//! Windows and Android disagree about what an image on a clipboard is. Android hands over
//! a `content://` URI that is almost always PNG or JPEG; Windows hands over a `CF_DIB`, a
//! raw pixel buffer with a `BITMAPINFOHEADER` and no compression. Sending a DIB over the
//! wire would be absurd — a phone screenshot is ~12 MB as a DIB and a few hundred KB as a
//! PNG — so PNG is the wire format and this module is the adapter at the Windows end.
//!
//! `Windows.Graphics.Imaging` rather than an image crate: the codecs are already on the
//! machine and the `windows` crate was already a dependency for toasts, so this costs two
//! cargo features instead of a decoder, its colour management and its CVE stream.

use anyhow::{anyhow, Context as _, Result};
use sha2::Digest as _;
use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use windows::core::GUID;
use windows::Graphics::Imaging::{BitmapDecoder, BitmapEncoder};
use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};
use windows::Win32::System::Com::CoIncrementMTAUsage;

/// A `CF_DIB` is a `BITMAPINFOHEADER` plus pixels, with no `BITMAPFILEHEADER` in front.
/// `BitmapDecoder` wants a whole .bmp file, so those 14 bytes are synthesised here.
const FILE_HEADER: usize = 14;

/// Unparks the thread that is blocked in [`block_on`].
struct Unparker(std::thread::Thread);

impl Wake for Unparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives one WinRT operation to completion on the current thread.
///
/// The async types only expose `IntoFuture`, and the blocking `join` on their internal
/// trait is not re-exported, so this is the supported way to wait synchronously. Park and
/// unpark rather than a spin: the completion handler fires on an OS thread and wakes this
/// one, so a conversion costs no CPU while the codec works. A spurious unpark only causes
/// an extra harmless poll.
///
/// Deliberately sync all the way down. Every caller is already off the reactor — the
/// clipboard thread, the toast thread, a `spawn_blocking` — because this is COM work.
pub(crate) fn block_on<F: IntoFuture>(op: F) -> F::Output {
    let mut future = std::pin::pin!(op.into_future());
    let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}

/// WinRT will not activate a class on a thread with no apartment, and neither the
/// clipboard thread nor a `spawn_blocking` worker has one.
///
/// Rather than initialise each of them — and have to un-initialise them, on exactly the
/// same thread, or leak an apartment — this keeps a process-wide implicit MTA alive that
/// uninitialised threads join automatically. The cookie is a plain `Copy` handle with no
/// destructor, so simply not calling `CoDecrementMTAUsage` is what keeps the MTA up.
pub(crate) fn ensure_mta() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = unsafe { CoIncrementMTAUsage() };
    });
}

/// Reads a WinRT stream out to a `Vec`, from position 0.
fn drain(stream: &InMemoryRandomAccessStream) -> Result<Vec<u8>> {
    let size = stream.Size()?;
    if size == 0 {
        return Err(anyhow!("codec produced an empty stream"));
    }
    let size = u32::try_from(size).map_err(|_| anyhow!("image of {size} bytes is implausible"))?;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    let loaded = block_on(reader.LoadAsync(size)?)?;
    let mut out = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut out)?;
    Ok(out)
}

/// Fills a fresh WinRT stream with `bytes` and rewinds it.
fn stream_of(bytes: &[u8]) -> Result<InMemoryRandomAccessStream> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0)?)?;
    writer.WriteBytes(bytes)?;
    block_on(writer.StoreAsync()?)?;
    block_on(writer.FlushAsync()?)?;
    // Detached, so dropping the writer does not close the stream under the decoder.
    let _ = writer.DetachStream();
    stream.Seek(0)?;
    Ok(stream)
}

/// Decodes `bytes` with whichever codec Windows recognises, re-encodes as `encoder`.
fn recode(bytes: &[u8], encoder: GUID) -> Result<Vec<u8>> {
    ensure_mta();
    let source = stream_of(bytes)?;
    let decoder = block_on(BitmapDecoder::CreateAsync(&source)?)?;
    let frame = block_on(decoder.GetSoftwareBitmapAsync()?)?;

    let target = InMemoryRandomAccessStream::new()?;
    let sink = block_on(BitmapEncoder::CreateAsync(encoder, &target)?)?;
    sink.SetSoftwareBitmap(&frame)?;
    block_on(sink.FlushAsync()?)?;
    drain(&target)
}

/// `CF_DIB` bytes -> PNG, for sending a Windows copy to the phone.
///
/// The DIB becomes a .bmp by prepending the file header the clipboard format omits. Its
/// `bfOffBits` has to account for the colour table and, for a `BI_BITFIELDS` DIB, the
/// masks that follow the header — get it wrong and every pixel shifts.
pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>> {
    // Enough of a `BITMAPINFOHEADER` to find where the pixels start.
    if dib.len() < 40 {
        return Err(anyhow!("DIB of {} bytes is too short to have a header", dib.len()));
    }
    let read = |at: usize| u32::from_le_bytes(dib[at..at + 4].try_into().expect("4 bytes"));
    let info_size = read(0) as usize;
    if info_size < 40 || info_size > dib.len() {
        return Err(anyhow!("implausible BITMAPINFOHEADER size {info_size}"));
    }
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().expect("2 bytes")) as usize;
    let compression = read(16);
    let mut palette = read(32) as usize;
    // biClrUsed 0 means "all of them" for an indexed image and nothing for true colour.
    if palette == 0 && bit_count <= 8 {
        palette = 1 << bit_count;
    }
    // BI_BITFIELDS stores three DWORD masks between header and pixels, and
    // BI_ALPHABITFIELDS four. Only for a v3 header: v4 and v5 carry them inside.
    let masks = match compression {
        3 if info_size == 40 => 12,
        6 if info_size == 40 => 16,
        _ => 0,
    };
    let offset = FILE_HEADER + info_size + masks + palette * 4;

    let mut bmp = Vec::with_capacity(FILE_HEADER + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((FILE_HEADER + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes()); // bfReserved1 and bfReserved2
    bmp.extend_from_slice(&(offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);

    recode(&bmp, BitmapEncoder::PngEncoderId()?).context("re-encoding a clipboard DIB as PNG")
}

/// PNG — or anything else Windows decodes — to `CF_DIB` bytes, for pasting a phone image.
///
/// The inverse trim. `bfOffBits` is not consulted on the way back because `CF_DIB` starts
/// at the info header, which is always immediately after the 14-byte file header.
pub fn png_to_dib(png: &[u8]) -> Result<Vec<u8>> {
    let bmp = recode(png, BitmapEncoder::BmpEncoderId()?).context("re-encoding an image as BMP")?;
    if bmp.len() <= FILE_HEADER || bmp[..2] != *b"BM" {
        return Err(anyhow!("BMP encoder produced {} bytes with no file header", bmp.len()));
    }
    Ok(bmp[FILE_HEADER..].to_vec())
}

/// Makes sure `bytes` really are a PNG, re-encoding if they are not.
///
/// The phone sends camera photos as the JPEG it already had rather than re-encoding them:
/// that saves a decode and an encode on a battery, and avoids a 4 MB photo arriving as
/// 20 MB of PNG. The conversion lands here instead, because the registered `"PNG"`
/// clipboard format has to actually contain a PNG — an app reading JPEG bytes from it
/// shows nothing, and gives no hint why.
///
/// Detection is by signature, not by the header's MIME string: the sender's `mime` is a
/// hint, and a peer that mislabels its own bytes should still end up with a working paste.
pub fn to_png(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>> {
    const MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(MAGIC) {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    recode(bytes, BitmapEncoder::PngEncoderId()?)
        .map(std::borrow::Cow::Owned)
        .context("re-encoding a received image as PNG")
}

/// 32 KiB. A 64 KiB chunk plus its protobuf framing overflows the 65519-byte Noise
/// plaintext ceiling, and `Session::send` refuses an oversized frame by returning an
/// error — which would tear down the session over a pasted screenshot.
const CHUNK: usize = 32 * 1024;

/// Sends one PNG as a header followed by chunks, returning the number of frames written.
///
/// Sequential and blocking on the session's write half by design. Sends are already
/// single-threaded — `Session::send` is explicitly not cancel-safe — so an image simply
/// occupies the writer for the few hundred syscalls it takes. `stream_id` exists in the
/// wire format for interleaving, but nothing here interleaves: one sender, one image at
/// a time, so the receiver never has to demultiplex.
pub async fn send<W>(
    session: &mut crate::wire::Session,
    w: &mut W,
    png: &[u8],
    photo: bool,
    screenshot: bool,
) -> Result<u64>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use crate::wire::pb;
    use prost::Message as _;

    let total = u32::try_from(png.len()).context("image does not fit a u32 length")?;
    let count = png.len().div_ceil(CHUNK) as u32;
    // Content-addressed rather than random: it needs no state to generate, it is stable
    // if the same image is copied twice, and it gives the receiver a cheap correlator.
    let id = sha2::Sha256::digest(png)[..16].to_vec();

    let header = pb::ClipImageHeader {
        mime: "image/png".into(),
        total_bytes: total,
        chunk_size: CHUNK as u32,
        chunk_count: count,
        file_name: String::new(),
        timestamp_ms: crate::now_ms(),
        header_id: id.clone(),
        photo,
        screenshot,
    };
    session
        .send(w, pb::Kind::ClipImageHeader, &header.encode_to_vec())
        .await?;

    for (index, chunk) in png.chunks(CHUNK).enumerate() {
        let frame = pb::ClipImageChunk {
            index: index as u32,
            data: chunk.to_vec(),
            header_id: id.clone(),
            stream_id: 1,
        };
        session
            .send(w, pb::Kind::ClipImageChunk, &frame.encode_to_vec())
            .await?;
    }
    Ok(count as u64 + 1)
}

/// One image being reassembled from chunks.
///
/// Bounded twice over: the header is refused outright above [`crate::clip::MAX_IMAGE`],
/// so nothing is allocated for an absurd claim, and every chunk is checked against the
/// declared total, so a peer cannot grow the buffer past what it announced.
///
/// Chunks are required to arrive in order. They travel on one TCP stream inside one Noise
/// session, so out-of-order is not a network condition here — it is a broken or hostile
/// peer, and dropping the transfer is the right response.
pub struct Assembly {
    id: Vec<u8>,
    photo: bool,
    screenshot: bool,
    expect: u32,
    next: u32,
    total: usize,
    bytes: Vec<u8>,
}

impl Assembly {
    /// Starts a transfer, or rejects the header. Replaces any transfer already in
    /// progress: a new header means the peer abandoned the last one.
    pub fn begin(header: &crate::wire::pb::ClipImageHeader) -> Result<Self> {
        let total = header.total_bytes as usize;
        if total == 0 {
            return Err(anyhow!("image header declares zero bytes"));
        }
        if total > crate::clip::MAX_IMAGE {
            return Err(anyhow!(
                "image of {total} B is over the {} B ceiling, refused before allocating",
                crate::clip::MAX_IMAGE
            ));
        }
        let chunk = header.chunk_size as usize;
        if chunk == 0 || chunk > CHUNK {
            return Err(anyhow!("implausible chunk size {chunk}"));
        }
        let expect = total.div_ceil(chunk) as u32;
        if header.chunk_count != expect {
            return Err(anyhow!(
                "header claims {} chunks, {total} B in {chunk} B needs {expect}",
                header.chunk_count
            ));
        }
        Ok(Self {
            id: header.header_id.clone(),
            photo: header.photo,
            screenshot: header.screenshot,
            expect,
            next: 0,
            total,
            bytes: Vec::with_capacity(total),
        })
    }

    /// Adds a chunk. `Ok(Some(png))` means that was the last one.
    pub fn push(&mut self, chunk: &crate::wire::pb::ClipImageChunk) -> Result<Option<Vec<u8>>> {
        if chunk.header_id != self.id {
            return Err(anyhow!("chunk belongs to a different image"));
        }
        if chunk.index != self.next {
            return Err(anyhow!("chunk {} arrived, expected {}", chunk.index, self.next));
        }
        if self.bytes.len() + chunk.data.len() > self.total {
            return Err(anyhow!(
                "chunk {} would take the image past the {} B it declared",
                chunk.index,
                self.total
            ));
        }
        self.bytes.extend_from_slice(&chunk.data);
        self.next += 1;

        if self.next < self.expect {
            return Ok(None);
        }
        if self.bytes.len() != self.total {
            return Err(anyhow!(
                "image ended at {} B, header said {}",
                self.bytes.len(),
                self.total
            ));
        }
        Ok(Some(std::mem::take(&mut self.bytes)))
    }

    /// Whether this was announced as a camera photo rather than a clipboard copy.
    pub fn is_photo(&self) -> bool {
        self.photo
    }

    /// Explicit screenshot semantics. New phone clients also set `photo=true` on a
    /// screenshot so old desktops keep it out of the clipboard; this field is what lets
    /// a new desktop label and handle the capture correctly.
    pub fn is_screenshot(&self) -> bool {
        self.screenshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 32-bit DIB: bottom-up, BGRA, no colour table.
    fn dib_2x2() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&2i32.to_le_bytes()); // biWidth
        dib.extend_from_slice(&2i32.to_le_bytes()); // biHeight
        dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
        dib.extend_from_slice(&16u32.to_le_bytes()); // biSizeImage
        dib.extend_from_slice(&[0u8; 16]); // ppm x and y, biClrUsed, biClrImportant
        for pixel in [
            [0u8, 0, 255, 255],   // red, as BGRA
            [0, 255, 0, 255],     // green
            [255, 0, 0, 255],     // blue
            [255, 255, 255, 255], // white
        ] {
            dib.extend_from_slice(&pixel);
        }
        dib
    }

    #[test]
    fn a_dib_survives_the_round_trip_as_a_png() {
        let png = dib_to_png(&dib_2x2()).expect("a DIB should encode as PNG");
        // The magic is the point: a DIB that had merely been copied would not start
        // with it, and neither would a stream the encoder failed to write.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

        let dib = png_to_dib(&png).expect("a PNG should encode back to a DIB");
        let width = i32::from_le_bytes(dib[4..8].try_into().unwrap());
        let height = i32::from_le_bytes(dib[8..12].try_into().unwrap());
        assert_eq!((width, height.abs()), (2, 2), "geometry changed in transit");
        // No "BM". `SetClipboardData(CF_DIB, ...)` with a file header still attached is
        // the one mistake that produces a plausible-looking blank paste.
        assert_ne!(&dib[..2], b"BM", "the file header must be trimmed for CF_DIB");
    }

    #[test]
    fn a_short_or_lying_dib_is_refused_rather_than_read_past_its_end() {
        assert!(dib_to_png(&[]).is_err());
        assert!(dib_to_png(&dib_2x2()[..20]).is_err());
        // Plausible length, absurd biSize: without the check, `offset` runs off the end.
        let mut lying = dib_2x2();
        lying[..4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(dib_to_png(&lying).is_err());
    }

    #[test]
    fn junk_is_an_error_not_a_panic() {
        assert!(png_to_dib(b"this is not an image at all").is_err());
        assert!(to_png(b"this is not an image at all").is_err());
    }

    #[test]
    fn to_png_passes_a_png_through_and_converts_anything_else() {
        let png = dib_to_png(&dib_2x2()).expect("a DIB should encode as PNG");
        // Borrowed, not re-encoded: a needless decode/encode of every received image
        // would be invisible except as latency on the common path.
        let same = to_png(&png).expect("a PNG should pass through");
        assert!(matches!(same, std::borrow::Cow::Borrowed(_)), "PNG was re-encoded");
        assert_eq!(&*same, &png[..]);

        // A JPEG is what the phone actually sends for a camera photo. Whatever comes
        // back must carry the PNG signature, or the clipboard format would be a lie.
        let jpeg = recode(&png, BitmapEncoder::JpegEncoderId().unwrap()).expect("encode JPEG");
        assert_ne!(&jpeg[..2], b"\x89P", "fixture is not actually a JPEG");
        let converted = to_png(&jpeg).expect("a JPEG should convert");
        assert_eq!(&converted[..8], b"\x89PNG\r\n\x1a\n", "not converted to PNG");
    }

    fn header(total: usize, chunk: usize, count: u32) -> crate::wire::pb::ClipImageHeader {
        crate::wire::pb::ClipImageHeader {
            mime: "image/png".into(),
            total_bytes: total as u32,
            chunk_size: chunk as u32,
            chunk_count: count,
            file_name: String::new(),
            timestamp_ms: 0,
            header_id: vec![7; 16],
            photo: false,
            screenshot: false,
        }
    }

    fn chunk(index: u32, data: Vec<u8>) -> crate::wire::pb::ClipImageChunk {
        crate::wire::pb::ClipImageChunk {
            index,
            data,
            header_id: vec![7; 16],
            stream_id: 1,
        }
    }

    #[test]
    fn chunks_reassemble_to_exactly_what_was_sent() {
        let payload: Vec<u8> = (0..CHUNK + 100).map(|i| i as u8).collect();
        let count = payload.len().div_ceil(CHUNK) as u32;
        let mut rx = Assembly::begin(&header(payload.len(), CHUNK, count)).expect("valid header");

        let mut out = None;
        for (index, part) in payload.chunks(CHUNK).enumerate() {
            out = rx.push(&chunk(index as u32, part.to_vec())).expect("valid chunk");
        }
        assert_eq!(out.as_deref(), Some(&payload[..]), "reassembly changed the bytes");
    }

    #[test]
    fn capture_flags_survive_header_validation() {
        let mut photo = header(4, CHUNK, 1);
        photo.photo = true;
        assert!(Assembly::begin(&photo).expect("photo header").is_photo());

        let mut screenshot = header(4, CHUNK, 1);
        // Compatibility marker and explicit semantics travel together.
        screenshot.photo = true;
        screenshot.screenshot = true;
        let rx = Assembly::begin(&screenshot).expect("screenshot header");
        assert!(rx.is_photo());
        assert!(rx.is_screenshot());
    }

    #[test]
    fn a_header_that_would_allocate_too_much_is_refused() {
        // The whole point of validating before `Vec::with_capacity`: a peer claiming
        // 4 GiB must not be able to make this process ask for it.
        assert!(Assembly::begin(&header(crate::clip::MAX_IMAGE + 1, CHUNK, 999)).is_err());
        assert!(Assembly::begin(&header(0, CHUNK, 0)).is_err());
        assert!(Assembly::begin(&header(100, 0, 1)).is_err());
        // A chunk size above the ceiling could not have been sent in one frame.
        assert!(Assembly::begin(&header(100, CHUNK * 2, 1)).is_err());
        // chunk_count must agree with the arithmetic, or the last chunk never arrives
        // and the buffer is held for the life of the session.
        assert!(Assembly::begin(&header(CHUNK * 2, CHUNK, 1)).is_err());
    }

    #[test]
    fn a_peer_cannot_exceed_or_reorder_what_it_declared() {
        let mut rx = Assembly::begin(&header(10, CHUNK, 1)).expect("valid header");
        assert!(rx.push(&chunk(0, vec![0; 11])).is_err(), "overshoot must be refused");

        let mut rx = Assembly::begin(&header(10, CHUNK, 1)).expect("valid header");
        assert!(rx.push(&chunk(1, vec![0; 10])).is_err(), "wrong index must be refused");

        let mut rx = Assembly::begin(&header(10, CHUNK, 1)).expect("valid header");
        let mut stranger = chunk(0, vec![0; 10]);
        stranger.header_id = vec![9; 16];
        assert!(rx.push(&stranger).is_err(), "another image's chunk must be refused");

        // Short of the declared total on the final chunk: the count is satisfied but
        // the image is truncated, so it must not reach the clipboard as a valid PNG.
        let mut rx = Assembly::begin(&header(10, CHUNK, 1)).expect("valid header");
        assert!(rx.push(&chunk(0, vec![0; 9])).is_err(), "short image must be refused");
    }
}
