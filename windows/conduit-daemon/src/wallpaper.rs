//! Phone-wallpaper preview cache for the desktop device frame.
//!
//! The wire payload is intentionally tiny and content-addressed. The daemon never polls this file:
//! the phone sends a new preview only after an authenticated connection or a real wallpaper-change
//! callback, and the WinUI process already watches the data directory for changes.

use anyhow::{anyhow, Context as _, Result};
use sha2::Digest as _;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "wallpaper.jpg";
const MAX_PREVIEW_BYTES: usize = 56 * 1024;
const SHA256_BYTES: usize = 32;

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub fn cached_hash(data_dir: &Path) -> Vec<u8> {
    let file = path(data_dir);
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    if !valid_jpeg(&bytes) || bytes.len() > MAX_PREVIEW_BYTES {
        return Vec::new();
    }
    sha2::Sha256::digest(&bytes).to_vec()
}

/// Validates and stores a received preview. `Ok(false)` means the cached bytes were already current.
pub fn store(data_dir: &Path, preview: &crate::wire::pb::WallpaperPreview) -> Result<bool> {
    if preview.jpeg.is_empty() || preview.jpeg.len() > MAX_PREVIEW_BYTES {
        return Err(anyhow!(
            "wallpaper preview is {} B, expected 1..={MAX_PREVIEW_BYTES}",
            preview.jpeg.len()
        ));
    }
    if !valid_jpeg(&preview.jpeg) {
        return Err(anyhow!("wallpaper preview is not a JPEG"));
    }
    if preview.sha256.len() != SHA256_BYTES {
        return Err(anyhow!("wallpaper preview hash is not SHA-256"));
    }
    let actual = sha2::Sha256::digest(&preview.jpeg);
    if actual.as_slice() != preview.sha256.as_slice() {
        return Err(anyhow!("wallpaper preview hash mismatch"));
    }
    if cached_hash(data_dir).as_slice() == actual.as_slice() {
        return Ok(false);
    }

    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    let target = path(data_dir);
    let temporary = data_dir.join("wallpaper.jpg.tmp");
    std::fs::write(&temporary, &preview.jpeg)
        .with_context(|| format!("writing {}", temporary.display()))?;
    // BitmapImage does not keep a sharing-denying file handle after decode in this UI. Replacing a
    // complete temporary file still prevents the watcher from ever observing a partial JPEG.
    if target.exists() {
        std::fs::remove_file(&target).with_context(|| format!("replacing {}", target.display()))?;
    }
    std::fs::rename(&temporary, &target)
        .with_context(|| format!("publishing {}", target.display()))?;
    Ok(true)
}

fn valid_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_jpeg_preview() {
        let preview = crate::wire::pb::WallpaperPreview {
            jpeg: b"not-a-jpeg".to_vec(),
            sha256: sha2::Sha256::digest(b"not-a-jpeg").to_vec(),
        };
        let dir = std::env::temp_dir().join(format!("conduit-wallpaper-{}", std::process::id()));
        let result = store(&dir, &preview);
        assert!(result.is_err());
    }
}
