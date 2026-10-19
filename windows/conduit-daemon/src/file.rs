//! Files arriving from the phone.
//!
//! Chunks are streamed through a bounded worker queue rather than buffering a whole file.
//!
//! The second rule is that every field here is peer-controlled. `name` in particular is a
//! string from another machine that is about to become a path on this one, so [`sanitise`]
//! is a trust boundary rather than a tidying pass.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use crate::wire::pb;

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
/// therefore costs no session. A later read failure is reported by the scheduler and uses
/// targeted cancellation when the peer supports it.
pub struct Outbound {
    path: PathBuf,
    file: tokio::fs::File,
    offer: pb::FileOffer,
    sent: u64,
    index: u64,
}

impl Outbound {
    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let path = tokio::task::spawn_blocking(move || validate_outbound(&path)).await??;
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            bail!("not a regular file");
        }
        let total = metadata.len();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "file".to_string());
        if !(1..=MAX_FILE).contains(&total) {
            bail!("file of {total} B is outside 1..{MAX_FILE}");
        }
        let chunks = total.div_ceil(CHUNK as u64);
        let timestamp_ms = now_ms();
        let transfer_id = crate::transfer::new_id()?;
        let offer = pb::FileOffer {
            name,
            mime: mime_for(&path).to_string(),
            total_bytes: total,
            chunk_size: CHUNK,
            chunk_count: chunks,
            transfer_id,
            timestamp_ms,
        };
        Ok(Self {
            path,
            file,
            offer,
            sent: 0,
            index: 0,
        })
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

    pub fn offer(&mut self, id: &[u8]) -> pb::FileOffer {
        self.offer.transfer_id = id.to_vec();
        self.offer.clone()
    }

    /// Read on a separate bounded producer; the session never waits for disk I/O.
    pub async fn next_chunk(&mut self) -> Result<Option<pb::FileChunk>> {
        if self.sent == self.offer.total_bytes {
            return Ok(None);
        }
        let want = (self.offer.total_bytes - self.sent).min(CHUNK as u64) as usize;
        let mut data = vec![0; want];
        self.file
            .read_exact(&mut data)
            .await
            .with_context(|| format!("reading {} at {} B", self.path.display(), self.sent))?;
        let chunk = pb::FileChunk {
            index: self.index,
            data,
            transfer_id: self.offer.transfer_id.clone(),
        };
        self.sent += want as u64;
        self.index += 1;
        Ok(Some(chunk))
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
    let device: String = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_lowercase()
        .chars()
        .map(|c| match c {
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            c => c,
        })
        .collect();
    if RESERVED.contains(&device.as_str()) || matches!(device.as_str(), "conin$" | "conout$") {
        stem.insert(0, '_');
    }
    stem = stem
        .chars()
        .scan(0usize, |units, c| {
            *units += c.len_utf16();
            (*units <= 120).then_some(c)
        })
        .collect();
    let ext: String = ext
        .chars()
        .scan(0usize, |units, c| {
            *units += c.len_utf16();
            (*units <= 32).then_some(c)
        })
        .collect();
    format!("{stem}{ext}")
        .trim_end_matches(['.', ' '])
        .to_string()
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
    chunk_size: u64,
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
        if offer.transfer_id.is_empty() || offer.transfer_id.len() > 64 {
            bail!("offer transfer id must contain 1..64 bytes");
        }
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let name = sanitise(&offer.name);
        // A private random name plus exclusive creation cannot truncate a stale partial/symlink.
        let scratch = dir.join(format!("conduit-{}.part", hex(&crate::transfer::new_id()?)));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
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
            chunk_size: offer.chunk_size as u64,
        })
    }

    /// The sanitised name this transfer will land under, for logging before it finishes.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    /// Writes one chunk, returning the finished file's path once the last one lands.
    ///
    /// In-order only. Chunks travel on one TCP stream inside one Noise session, so out of
    /// order is not a network condition — it is a broken or hostile peer, and dropping the
    /// transfer is the right answer.
    pub fn push(&mut self, chunk: &pb::FileChunk) -> Result<Option<PathBuf>> {
        self.push_cancellable(chunk, &std::sync::atomic::AtomicBool::new(false))
    }

    pub fn push_cancellable(
        &mut self,
        chunk: &pb::FileChunk,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Option<PathBuf>> {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            bail!("Cancelled");
        }
        if chunk.transfer_id != self.id {
            bail!("chunk belongs to a different transfer");
        }
        if chunk.index != self.next {
            bail!("chunk {} arrived, expected {}", chunk.index, self.next);
        }
        let len = chunk.data.len() as u64;
        let expected = self.total.saturating_sub(self.written).min(self.chunk_size);
        if self.next >= self.chunks || len != expected {
            bail!("chunk length {len}, expected {expected}");
        }
        if len > self.total.saturating_sub(self.written) {
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
        file.sync_all().context("syncing the received file")?;
        drop(file);

        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            bail!("Cancelled");
        }

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
            if let Err(e) = std::fs::remove_file(&self.scratch) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %self.scratch.display(), error = %e, "could not remove abandoned partial file");
                }
            }
        }
    }
}

/// Atomically moves to a free name. No placeholder or replace-existing race.
fn publish(scratch: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
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
        let path = dir.join(candidate);
        match move_new(scratch, &path) {
            Ok(()) => return Ok(path),
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                    || std::fs::symlink_metadata(&path).is_ok() =>
            {
                continue
            }
            Err(e) => return Err(e).context("publishing received file without overwrite"),
        }
    }
    bail!("no free name for received file")
}

fn move_new(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    // canonicalize supplies Windows' extended-length prefix without resolving the final name
    // (which may already be a user file/symlink that must never be followed or replaced).
    let source = std::fs::canonicalize(source)?;
    let parent = destination
        .parent()
        .ok_or_else(|| std::io::Error::other("missing destination directory"))?;
    let name = destination
        .file_name()
        .ok_or_else(|| std::io::Error::other("missing destination name"))?;
    let destination = std::fs::canonicalize(parent)?.join(name);
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MOVEFILE_WRITE_THROUGH, deliberately without MOVEFILE_REPLACE_EXISTING.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
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

        let temp = dir.join("scratch.part");
        std::fs::write(&temp, b"new file")?;
        let second = publish(&temp, &dir, "note.txt")?;
        assert_eq!(second.file_name().unwrap(), "note (2).txt");
        std::fs::write(&temp, b"third file")?;
        let third = publish(&temp, &dir, "note.txt")?;
        assert_eq!(third.file_name().unwrap(), "note (3).txt");
        assert_eq!(std::fs::read(dir.join("note.txt"))?, b"the user's own file");
        // No extension is a case the naive `split('.')` version gets wrong.
        std::fs::write(dir.join("README"), b"x")?;
        std::fs::write(&temp, b"other")?;
        assert_eq!(
            publish(&temp, &dir, "README")?.file_name().unwrap(),
            "README (2)"
        );
        std::fs::create_dir(dir.join("folder"))?;
        std::fs::write(&temp, b"other")?;
        assert_eq!(
            publish(&temp, &dir, "folder")?.file_name().unwrap(),
            "folder (2)"
        );
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

        // Force publication to fail *after* push has closed/taken the file handle. This is the
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

    #[test]
    fn exact_chunk_lengths_and_ids_are_validated_before_mutation() -> Result<()> {
        let dir = scratch("chunk-boundaries");
        let mut rx = Incoming::begin(&offer("valid.bin", 6), &dir)?;
        for data in [vec![], vec![1; 3], vec![1; 5]] {
            assert!(rx.push(&chunk(0, &data)).is_err());
            assert_eq!(rx.written(), 0);
        }
        let mut other = chunk(0, b"abcd");
        other.transfer_id = vec![99];
        assert!(rx.push(&other).is_err());
        assert_eq!(rx.written(), 0);
        assert!(rx.push(&chunk(0, b"abcd"))?.is_none());
        assert!(rx.push(&chunk(1, b"x")).is_err());
        assert_eq!(rx.written(), 4);
        let path = rx.push(&chunk(1, b"ef"))?.unwrap();
        assert_eq!(std::fs::read(&path)?, b"abcdef");
        assert!(rx.push(&chunk(2, b"anything")).is_err());
        drop(rx);
        assert_eq!(std::fs::read(&path)?, b"abcdef");
        Ok(())
    }

    #[test]
    fn refused_ids_and_extreme_declarations_create_no_files() -> Result<()> {
        let dir = scratch("id-bounds");
        for id in [Vec::new(), vec![7; 65]] {
            let mut invalid = offer("bad", 4);
            invalid.transfer_id = id;
            assert!(Incoming::begin(&invalid, &dir).is_err());
        }
        let mut invalid = offer("bad", 4);
        invalid.total_bytes = u64::MAX;
        assert!(Incoming::begin(&invalid, &dir).is_err());
        assert_eq!(std::fs::read_dir(&dir)?.count(), 0);
        Ok(())
    }

    #[test]
    fn cancellation_cleans_partials_and_never_removes_published_files() -> Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = scratch("cancel-race");
        let cancel = AtomicBool::new(false);
        let mut rx = Incoming::begin(&offer("cancel.bin", 8), &dir)?;
        rx.push_cancellable(&chunk(0, b"abcd"), &cancel)?;
        cancel.store(true, Ordering::Release);
        assert!(rx.push_cancellable(&chunk(1, b"efgh"), &cancel).is_err());
        drop(rx);
        assert_eq!(std::fs::read_dir(&dir)?.count(), 0);
        cancel.store(false, Ordering::Release);
        let mut rx = Incoming::begin(&offer("complete.bin", 4), &dir)?;
        let path = rx.push_cancellable(&chunk(0, b"done"), &cancel)?.unwrap();
        cancel.store(true, Ordering::Release);
        drop(rx);
        assert_eq!(std::fs::read(path)?, b"done");
        Ok(())
    }

    #[test]
    fn repeated_peer_ids_cannot_truncate_an_existing_partial() -> Result<()> {
        let dir = scratch("scratch-exclusive");
        let mut first = Incoming::begin(&offer("one", 8), &dir)?;
        first.push(&chunk(0, b"abcd"))?;
        let second = Incoming::begin(&offer("two", 8), &dir)?;
        assert_ne!(first.scratch, second.scratch);
        assert_eq!(std::fs::metadata(&first.scratch)?.len(), 4);
        drop(second);
        drop(first);
        assert_eq!(std::fs::read_dir(&dir)?.count(), 0);
        Ok(())
    }

    #[test]
    fn a_disk_write_error_still_removes_the_partial() -> Result<()> {
        let dir = scratch("write-error-cleanup");
        let mut rx = Incoming::begin(&offer("bad.bin", 4), &dir)?;
        let partial = rx.scratch.clone();
        // A real read-only handle makes WriteFile fail deterministically without a full disk.
        rx.file = Some(File::open(&partial)?);
        assert!(rx.push(&chunk(0, b"data")).is_err());
        drop(rx);
        assert!(!partial.exists());
        assert_eq!(std::fs::read_dir(dir)?.count(), 0);
        Ok(())
    }

    #[test]
    fn names_fit_windows_utf16_limits_even_with_hostile_extensions() {
        for name in [
            format!("{}.{}", "😀".repeat(200), "😀".repeat(200)),
            format!("x.{}", "z".repeat(400)),
        ] {
            assert!(sanitise(&name).encode_utf16().count() <= 152);
        }
        assert_eq!(sanitise("nul.tar.gz"), "_nul.tar.gz");
        assert_eq!(sanitise("COM¹.txt"), "_COM¹.txt");
        assert_eq!(sanitise("CONOUT$.txt"), "_CONOUT$.txt");
    }

    #[test]
    fn concurrent_publications_never_replace_each_other() -> Result<()> {
        let dir = scratch("publish-race");
        let first = dir.join("one.part");
        let second = dir.join("two.part");
        std::fs::write(&first, b"one")?;
        std::fs::write(&second, b"two")?;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let other_barrier = barrier.clone();
        let other_dir = dir.clone();
        let other = std::thread::spawn(move || {
            other_barrier.wait();
            publish(&second, &other_dir, "same.bin")
        });
        barrier.wait();
        let one = publish(&first, &dir, "same.bin")?;
        let two = other.join().unwrap()?;
        assert_ne!(one, two);
        assert_eq!(std::fs::read(one)?, b"one");
        assert_eq!(std::fs::read(two)?, b"two");
        assert_eq!(std::fs::read_dir(&dir)?.count(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn outbound_reads_one_declared_chunk_per_turn() -> Result<()> {
        let dir = scratch("outbound-chunks");
        let path = dir.join("payload.bin");
        let bytes = vec![42u8; CHUNK as usize + 3];
        std::fs::write(&path, &bytes)?;
        let mut outbound = Outbound::open(&path).await?;
        let offer = outbound.offer(&[8; 16]);
        assert_eq!(offer.chunk_count, 2);
        let first = outbound.next_chunk().await?.unwrap();
        assert_eq!(first.index, 0);
        assert_eq!(first.data.len(), CHUNK as usize);
        let last = outbound.next_chunk().await?.unwrap();
        assert_eq!(last.index, 1);
        assert_eq!(last.data, vec![42; 3]);
        assert_eq!(last.transfer_id, vec![8; 16]);
        assert!(outbound.next_chunk().await?.is_none());
        Ok(())
    }
}
