//! Explicit user connection intent, separate from pairing trust and routing configuration.
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;

const FILE: &str = "link-intent.txt";

pub fn load(dir: &Path) -> Result<bool> {
    match std::fs::read_to_string(dir.join(FILE)) {
        Ok(text) => match text.trim() {
            "enabled" => Ok(true),
            "paused" => Ok(false),
            _ => bail!("invalid link intent; refusing to reconnect without an explicit choice"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e).context("reading saved connection intent"),
    }
}

pub fn save(dir: &Path, enabled: bool) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(FILE);
    let temp = dir.join("link-intent.tmp");
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(if enabled { b"enabled\n" } else { b"paused\n" })?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, &path).context("committing saved connection intent")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_pause_survives_restart_without_changing_trust() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("conduit-link-intent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        assert!(load(&dir)?);
        std::fs::write(dir.join("peer.txt"), "trusted peer")?;
        save(&dir, false)?;
        assert!(!load(&dir)?);
        save(&dir, true)?;
        assert!(load(&dir)?);
        assert_eq!(
            std::fs::read_to_string(dir.join("peer.txt"))?,
            "trusted peer"
        );
        std::fs::write(dir.join(FILE), "broken")?;
        assert!(load(&dir).is_err());
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
