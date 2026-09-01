//! Files arriving from the phone.
//!
//! The one rule that shapes this file: a chunk is written to disk the moment it arrives
//! and is never held. A 500 MB share therefore costs this process one 32 KiB frame of
//! memory, not 500 MB — which is the difference between a companion daemon you can leave
//! running for a fortnight and one you cannot.
//!
//! The second rule is that every field here is peer-controlled. `name` in particular is a
//! string from another machine that is about to become a path on this one, so [`sanitise`]
//! is a trust boundary rather than a tidying pass.

use anyhow::{bail, Context, Result};
use prost::Message as _;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::wire::{pb, Session};

/// Matches the phone's `CHUNK`. A larger chunk plus protobuf framing overflows the
/// 65519-byte Noise plaintext ceiling, which would tear the session down.
const CHUNK: u32 = 32 * 1024;

/// ponytail: a flat 512 MiB cap rather than a configurable one. It is generous for
/// "send this to my PC" and it bounds what a peer can make this machine write; raise it
/// when someone actually wants to move a disk image over a phone.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// Checks a local file before it can enter the daemon's bounded outbound queue.
///
/// Canonicalising here means a later UI/Explorer integration can hand the daemon a relative path
/// without making the long-running process depend on whatever its current directory happens to be.
pub fn validate_outbound(path: &Path) -> Result<PathBuf> {
    let path =
        std::fs::canonicalize(path).with_context(|| format!("opening {}", path.display()))?;
    let metadata =
        std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    if !(1..=MAX_FILE).contains(&metadata.len()) {
        bail!("file of {} B is outside 1..{MAX_FILE}", metadata.len());
    }
    Ok(path)
}

/// A local Windows file already opened and ready to send to the phone.
///
/// Opening happens before the `FILE_OFFER` goes on the wire. A missing/locked/oversize local file
/// therefore costs no session. After the offer has been sent, any read failure must end the
/// session so Android's pending MediaStore row is dropped with that session rather than waiting
/// forever for chunks that can never arrive.
pub struct Outbound {
    path: PathBuf,
    file: tokio::fs::File,
    offer: pb::FileOffer,
}

impl Outbound {
    pub async fn open(path: &Path) -> Result<Self> {
        let path = validate_outbound(path)?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        let total = metadata.len();
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "file".to_string());
        let chunks = total.div_ceil(CHUNK as u64);
        let timestamp_ms = now_ms();
        let mut digest = Sha256::new();
        digest.update(path.as_os_str().to_string_lossy().as_bytes());
        digest.update(total.to_be_bytes());
        digest.update(timestamp_ms.to_be_bytes());
        let transfer_id = digest.finalize()[..16].to_vec();
        let offer = pb::FileOffer {
            name,
            mime: mime_for(&path).to_string(),
            total_bytes: total,
            chunk_size: CHUNK,
            chunk_count: chunks,
            transfer_id,
            timestamp_ms,
        };
        Ok(Self { path, file, offer })
    }

    pub fn name(&self) -> &str {
        &self.offer.name
    }

    pub fn transfer_id(&self) -> &[u8] {
        &self.offer.transfer_id
    }

    pub fn total_bytes(&self) -> u64 {
        self.offer.total_bytes
    }

    /// Sends the offer and every chunk, returning the number of encrypted frames written.
    /// Progress is emitted only when the integer percentage changes, so a 512 MiB file still
    /// produces at most 101 local UI updates rather than one IPC event per 32 KiB chunk.
    pub async fn send<F>(
        mut self,
        session: &mut Session,
        stream: &mut TcpStream,
        mut progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64),
    {
        session
            .send(stream, pb::Kind::FileOffer, &self.offer.encode_to_vec())
            .await?;
        let mut frames = 1u64;
        let mut sent = 0u64;
        let mut index = 0u64;
        let mut last_percent = 0u64;
        progress(0, self.offer.total_bytes);
        let mut buffer = vec![0u8; CHUNK as usize];
        while sent < self.offer.total_bytes {
            let want = (self.offer.total_bytes - sent).min(CHUNK as u64) as usize;
            self.file
                .read_exact(&mut buffer[..want])
                .await
                .with_context(|| format!("reading {} at {sent} B", self.path.display()))?;
            let chunk = pb::FileChunk {
                index,
                data: buffer[..want].to_vec(),
                transfer_id: self.offer.transfer_id.clone(),
            };
            session
                .send(stream, pb::Kind::FileChunk, &chunk.encode_to_vec())
                .await?;
            sent += want as u64;
            index += 1;
            frames += 1;
            let percent = sent.saturating_mul(100) / self.offer.total_bytes;
            if percent > last_percent || sent == self.offer.total_bytes {
                last_percent = percent;
                progress(sent, self.offer.total_bytes);
            }
        }
        info!(path = %self.path.display(), bytes = sent, chunks = index, "file sent");
        Ok(frames)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// Names Windows refuses to give a file, with or without an extension: `nul.txt` is still
/// the null device. Prefixed rather than rejected, so a legitimately-named `aux.png` from
/// a phone still lands as `_aux.png` instead of vanishing.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Where received files land. `CONDUIT_DOWNLOADS` overrides the saved preference.
///
/// Asks the shell rather than assuming `%USERPROFILE%\Downloads` only when the user has not
/// chosen a folder. A relocated Downloads folder is common, and writing to the place the user
/// moved *away* from means their file arrives somewhere they will never look.
pub fn downloads(configured: Option<&str>) -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CONDUIT_DOWNLOADS") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    if let Some(dir) = configured.map(str::trim).filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None)
            .context("asking the shell for the Downloads folder")?;
        let path = raw
            .to_string()
            .context("Downloads folder path is not UTF-16")?;
        CoTaskMemFree(Some(raw.0 as *const _));
        Ok(PathBuf::from(path))
    }
}

/// Turns a peer-supplied name into one basename that is safe to create in a directory of
/// our choosing.
///
/// Everything here is defence against a name that means something other than it looks:
///  - only the last path component survives, so `..\..\Startup\x.lnk` is `x.lnk`
///  - the characters Windows forbids become `_`, so `C:evil` cannot be drive-relative
///  - trailing dots and spaces go, because Windows silently ignores them — `evil.exe.`
///    and `evil.exe` are the same file, and only one of them is what the log said
///  - device names are prefixed rather than dropped
///  - the stem is capped so the whole component stays inside the 255-unit filesystem limit
///    with room for a ` (2)` suffix
fn sanitise(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);

    // Split on the last dot so `archive.tar.gz` keeps `.gz` and `.gitignore` stays whole.
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (cleaned, String::new()),
    };
    let mut stem = if stem.is_empty() {
        "file".to_string()
    } else {
        stem.to_string()
    };
    if RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
        stem.insert(0, '_');
    }
    if stem.chars().count() > 120 {
        stem = stem.chars().take(120).collect();
    }
    format!("{stem}{ext}")
}

/// Reserves a free name in `dir`, creating it empty.
///
/// `create_new` rather than an `exists` check, because the two are not the same thing: the
/// check-then-create version can hand the same name to two transfers, and clobbering a
/// file the user already had is the one outcome that is not recoverable.
fn reserve(dir: &Path, name: &str) -> Result<PathBuf> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    for n in 1..=999 {
        let candidate = if n == 1 {
            name.to_string()
        } else {
            format!("{stem} ({n}){ext}")
        };
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    bail!("999 files in {} are already called {name}", dir.display())
}

/// One file being received.
///
/// Writes to a scratch name and only takes the real one at the end, so a transfer in
/// flight can never be mistaken for a finished file. The [`Drop`] is the other half of
/// that: a session that dies mid-file takes the scratch file with it, whether it ended by
/// return, error or task abort. Same invariant as `SessionGuard`, same reason.
pub struct Incoming {
    dir: PathBuf,
    scratch: PathBuf,
    file: Option<File>,
    /// True only after the scratch file has become the final destination. `file == None`
    /// is not enough: the handle is deliberately closed before publication, and any error
    /// between close and rename must still make Drop remove the partial.
    published: bool,
    name: String,
    id: Vec<u8>,
    next: u64,
    written: u64,
    total: u64,
    chunks: u64,
}

impl Incoming {
    /// Validates the offer and opens the scratch file, or fails without creating anything.
    pub fn begin(offer: &pb::FileOffer, dir: &Path) -> Result<Self> {
        if !(1..=MAX_FILE).contains(&offer.total_bytes) {
            bail!("file of {} B is outside 1..{MAX_FILE}", offer.total_bytes);
        }
        if !(1..=CHUNK).contains(&offer.chunk_size) {
            bail!("implausible chunk size {}", offer.chunk_size);
        }
        let expect = offer.total_bytes.div_ceil(offer.chunk_size as u64);
        if offer.chunk_count != expect {
            bail!(
                "offer claims {} chunks, {} B in {} B needs {expect}",
                offer.chunk_count,
                offer.total_bytes,
                offer.chunk_size
            );
        }
        if offer.transfer_id.is_empty() {
            bail!("offer has no transfer id");
        }
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let name = sanitise(&offer.name);
        // Named by the transfer id, so two transfers cannot collide on it and the file is
        // obviously scratch to anyone watching the folder.
        let scratch = dir.join(format!("conduit-{}.part", hex(&offer.transfer_id)));
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&scratch)
            .with_context(|| format!("creating {}", scratch.display()))?;

        Ok(Self {
            dir: dir.to_path_buf(),
            scratch,
            file: Some(file),
            published: false,
            name,
            id: offer.transfer_id.clone(),
            next: 0,
            written: 0,
            total: offer.total_bytes,
            chunks: offer.chunk_count,
        })
    }

    /// The sanitised name this transfer will land under, for logging before it finishes.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Writes one chunk, returning the finished file's path once the last one lands.
    ///
    /// In-order only. Chunks travel on one TCP stream inside one Noise session, so out of
    /// order is not a network condition — it is a broken or hostile peer, and dropping the
    /// transfer is the right answer.
    pub fn push(&mut self, chunk: &pb::FileChunk) -> Result<Option<PathBuf>> {
        if chunk.transfer_id != self.id {
            bail!("chunk belongs to a different transfer");
        }
        if chunk.index != self.next {
            bail!("chunk {} arrived, expected {}", chunk.index, self.next);
        }
        let len = chunk.data.len() as u64;
        if self.written + len > self.total {
            bail!(
                "chunk {} would take the file past the {} B it declared",
                chunk.index,
                self.total
            );
        }
        let file = self.file.as_mut().context("transfer already finished")?;
        file.write_all(&chunk.data)
            .with_context(|| format!("writing {}", self.scratch.display()))?;
        self.next += 1;
        self.written += len;

        if self.next < self.chunks {
            return Ok(None);
        }
        if self.written != self.total {
            bail!(
                "file ended at {} B, offer said {}",
                self.written,
                self.total
            );
        }
        // Flushed and closed before the rename: a handle still open is a rename that fails
        // on Windows.
        let mut file = self.file.take().expect("checked above");
        file.flush().context("flushing the received file")?;
        drop(file);

        let path = publish(&self.scratch, &self.dir, &self.name)?;
        self.published = true;
        info!(path = %path.display(), bytes = self.total, "file received");
        Ok(Some(path))
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        // Closing the handle is necessary before Windows can publish/remove the scratch,
        // but closing it is *not* success. A reserve/rename failure happens after file.take(),
        // so publication has its own explicit bit rather than being inferred from the handle.
        let _ = self.file.take();
        if !self.published {
            warn!(
                name = %self.name,
                got = self.written,
                of = self.total,
                "file transfer abandoned, dropping the partial"
            );
            let _ = std::fs::remove_file(&self.scratch);
        }
    }
}

/// Atomically reserves a collision-free final name and moves the completed scratch into it.
///
/// `reserve` creates a zero-byte placeholder so an external process cannot win the filename
/// between an `exists` check and our rename. If the rename itself fails, that placeholder is
/// ours and must be removed before returning the error; otherwise a failed transfer leaves a
/// convincing-looking empty destination behind.
fn publish(scratch: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    let path = reserve(dir, name)?;
    match std::fs::rename(scratch, &path) {
        Ok(()) => Ok(path),
        Err(e) => {
            if let Err(cleanup) = std::fs::remove_file(&path) {
                warn!(
                    path = %path.display(),
                    error = %cleanup,
                    "could not remove a failed transfer's reserved placeholder"
                );
            }
            Err(e).with_context(|| format!("moving the transfer into {}", path.display()))
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("conduit-file-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The trust boundary. Every case here is a name that would land somewhere other than
    /// the directory we chose, or as a file that is not the one the log named.
    #[test]
    fn a_peer_cannot_choose_where_its_file_lands() {
        assert_eq!(sanitise(r"..\..\..\Windows\System32\evil.dll"), "evil.dll");
        assert_eq!(sanitise("../../../etc/passwd"), "passwd");
        assert_eq!(sanitise(".."), "file");
        assert_eq!(sanitise("."), "file");
        assert_eq!(sanitise(""), "file");
        assert_eq!(sanitise("   "), "file");
        // Drive-relative: `C:evil.exe` means "evil.exe in the current directory of C:",
        // which is not this directory.
        assert_eq!(sanitise("C:evil.exe"), "C_evil.exe");
        assert_eq!(sanitise(r"\\server\share\x.txt"), "x.txt");
        // Windows ignores a trailing dot, so this would be `evil.exe` on disk while every
        // log line called it something else.
        assert_eq!(sanitise("evil.exe."), "evil.exe");
        assert_eq!(sanitise("evil.exe "), "evil.exe");
        // Device names, with and without an extension.
        assert_eq!(sanitise("nul"), "_nul");
        assert_eq!(sanitise("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitise("COM1.png"), "_COM1.png");
        // Not reserved, and must not be mangled.
        assert_eq!(sanitise("console.log"), "console.log");
        // A newline in a filename is a log-injection trick and not a legal path character.
        assert_eq!(sanitise("a\nb.txt"), "a_b.txt");
        // Ordinary names survive untouched, including non-ASCII ones.
        assert_eq!(
            sanitise("Screenshot 2026-08-25.png"),
            "Screenshot 2026-08-25.png"
        );
        assert_eq!(sanitise("照片.jpg"), "照片.jpg");
        assert_eq!(sanitise("archive.tar.gz"), "archive.tar.gz");
        assert_eq!(sanitise(".gitignore"), ".gitignore");
        // Long enough to blow the component limit, extension kept.
        let long = sanitise(&format!("{}.txt", "n".repeat(400)));
        assert!(
            long.chars().count() <= 124,
            "{} chars",
            long.chars().count()
        );
        assert!(long.ends_with(".txt"));
    }

    #[test]
    fn an_existing_file_is_never_overwritten() -> Result<()> {
        let dir = scratch("reserve");
        std::fs::write(dir.join("note.txt"), b"the user's own file")?;

        let second = reserve(&dir, "note.txt")?;
        assert_eq!(second.file_name().unwrap(), "note (2).txt");
        let third = reserve(&dir, "note.txt")?;
        assert_eq!(third.file_name().unwrap(), "note (3).txt");
        assert_eq!(std::fs::read(dir.join("note.txt"))?, b"the user's own file");
        // No extension is a case the naive `split('.')` version gets wrong.
        std::fs::write(dir.join("README"), b"x")?;
        assert_eq!(reserve(&dir, "README")?.file_name().unwrap(), "README (2)");
        Ok(())
    }

    fn offer(name: &str, total: u64) -> pb::FileOffer {
        pb::FileOffer {
            name: name.into(),
            mime: "application/octet-stream".into(),
            total_bytes: total,
            chunk_size: 4,
            chunk_count: total.div_ceil(4),
            transfer_id: vec![7, 7, 7, 7],
            timestamp_ms: 0,
        }
    }

    fn chunk(index: u64, data: &[u8]) -> pb::FileChunk {
        pb::FileChunk {
            index,
            data: data.to_vec(),
            transfer_id: vec![7, 7, 7, 7],
        }
    }

    #[test]
    fn a_whole_file_arrives_under_a_safe_name() -> Result<()> {
        let dir = scratch("whole");
        let mut rx = Incoming::begin(&offer(r"..\report.txt", 10), &dir)?;
        assert_eq!(rx.push(&chunk(0, b"abcd"))?, None);
        assert_eq!(rx.push(&chunk(1, b"efgh"))?, None);
        let path = rx
            .push(&chunk(2, b"ij"))?
            .expect("the last chunk did not finish it");

        assert_eq!(path.file_name().unwrap(), "report.txt");
        assert_eq!(std::fs::read(&path)?, b"abcdefghij");
        // Nothing left behind: one file in the directory, and it is not a `.part`.
        assert_eq!(std::fs::read_dir(&dir)?.count(), 1);
        Ok(())
    }

    #[test]
    fn a_publication_failure_does_not_leave_its_placeholder() -> Result<()> {
        let dir = scratch("publish-fail");
        let missing = dir.join("does-not-exist.part");
        assert!(publish(&missing, &dir, "result.bin").is_err());
        assert!(
            !dir.join("result.bin").exists(),
            "failed rename left its zero-byte reserved destination behind"
        );
        Ok(())
    }

    #[test]
    fn a_finalisation_error_still_deletes_the_partial() -> Result<()> {
        let dir = scratch("finish-cleanup");
        let mut rx = Incoming::begin(&offer("finish.bin", 4), &dir)?;
        let part = rx.scratch.clone();
        assert!(part.is_file(), "begin did not create the scratch file");

        // Force reserve() to fail *after* push has closed/taken the file handle. This is the
        // exact window the old Drop logic missed because it equated `file == None` with success.
        rx.dir = dir.join("missing-parent");
        assert!(rx.push(&chunk(0, b"abcd")).is_err());
        drop(rx);
        assert!(
            !part.exists(),
            "finalisation error leaked the completed .part file"
        );
        Ok(())
    }

    #[test]
    fn an_abandoned_transfer_leaves_nothing_behind() -> Result<()> {
        let dir = scratch("abandon");
        {
            let mut rx = Incoming::begin(&offer("half.bin", 10), &dir)?;
            rx.push(&chunk(0, b"abcd"))?;
            assert_eq!(
                std::fs::read_dir(&dir)?.count(),
                1,
                "nothing was being written"
            );
        } // dropped here, as it would be when a session ends
        assert_eq!(
            std::fs::read_dir(&dir)?.count(),
            0,
            "a partial file survived the session that was receiving it"
        );
        Ok(())
    }

    #[test]
    fn a_dishonest_offer_is_refused_before_anything_is_created() -> Result<()> {
        let dir = scratch("refuse");
        // Nothing at all.
        assert!(Incoming::begin(&offer("x", 0), &dir).is_err());
        // Past the cap, which must cost no allocation and no file.
        assert!(Incoming::begin(&offer("x", MAX_FILE + 1), &dir).is_err());
        // A chunk count that does not follow from the size: the receiver would either
        // finish early or wait forever.
        let mut lying = offer("x", 10);
        lying.chunk_count = 1;
        assert!(Incoming::begin(&lying, &dir).is_err());
        // A chunk larger than one frame can carry.
        let mut fat = offer("x", 10);
        fat.chunk_size = CHUNK + 1;
        assert!(Incoming::begin(&fat, &dir).is_err());

        assert_eq!(
            std::fs::read_dir(&dir)?.count(),
            0,
            "a refused offer created a file"
        );
        Ok(())
    }

    #[test]
    fn a_broken_stream_drops_the_transfer_rather_than_the_file_it_would_write() -> Result<()> {
        let dir = scratch("broken");
        let mut rx = Incoming::begin(&offer("x.bin", 10), &dir)?;
        rx.push(&chunk(0, b"abcd"))?;
        // Out of order.
        assert!(rx.push(&chunk(2, b"efgh")).is_err());
        // Someone else's transfer.
        let mut stray = chunk(1, b"efgh");
        stray.transfer_id = vec![9, 9];
        assert!(rx.push(&stray).is_err());
        // Past the declared total.
        assert!(rx.push(&chunk(1, b"efghijklmn")).is_err());
        Ok(())
    }

    /// A real path from the shell, so the folder files land in is the one the user sees
    /// rather than a guess at where it usually is.
    #[test]
    fn the_downloads_folder_is_an_absolute_path_that_exists() -> Result<()> {
        crate::image::ensure_mta();
        let dir = downloads(None)?;
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(dir.is_dir(), "{} is not a directory", dir.display());
        Ok(())
    }

    #[tokio::test]
    async fn an_outbound_file_is_opened_before_it_is_offered() -> Result<()> {
        let dir = scratch("outbound");
        let path = dir.join("phone photo.png");
        std::fs::write(&path, b"abcdefghij")?;

        let outbound = Outbound::open(&path).await?;
        assert_eq!(outbound.offer.name, "phone photo.png");
        assert_eq!(outbound.offer.mime, "image/png");
        assert_eq!(outbound.offer.total_bytes, 10);
        assert_eq!(outbound.offer.chunk_size, CHUNK);
        assert_eq!(outbound.offer.chunk_count, 1);
        assert_eq!(outbound.offer.transfer_id.len(), 16);

        assert!(
            Outbound::open(&dir).await.is_err(),
            "a directory became a file offer"
        );
        let empty = dir.join("empty.bin");
        std::fs::write(&empty, [])?;
        assert!(
            Outbound::open(&empty).await.is_err(),
            "an empty file became a file offer"
        );
        Ok(())
    }
}
