use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Mutable Conduit state lives beside the executables in `data/`.
///
/// This makes a standalone folder genuinely portable. Scoop packages persist the same `data`
/// directory across upgrades, so callers see one layout regardless of how Conduit was installed.
/// `CONDUIT_DATA_DIR` remains an explicit development/advanced-user override.
pub fn resolve() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CONDUIT_DATA_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let exe = std::env::current_exe().context("finding the Conduit executable")?;
    let parent = exe
        .parent()
        .context("Conduit executable has no parent directory")?;
    let portable = parent.join("data");
    migrate_legacy_if_needed(&portable)?;
    Ok(portable)
}

fn migrate_legacy_if_needed(portable: &Path) -> Result<()> {
    if portable.exists() {
        return Ok(());
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let legacy = PathBuf::from(local).join("Conduit");
    if !legacy.is_dir() || legacy == portable {
        return Ok(());
    }

    std::fs::create_dir_all(portable)
        .with_context(|| format!("creating portable data directory {}", portable.display()))?;
    copy_tree(&legacy, portable).with_context(|| {
        format!(
            "migrating Conduit data from {} to {}",
            legacy.display(),
            portable.display()
        )
    })?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&destination_path)?;
            copy_tree(&source_path, &destination_path)?;
        } else if !destination_path.exists() {
            std::fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copying {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_data_dir_wins() {
        let old = std::env::var_os("CONDUIT_DATA_DIR");
        std::env::set_var("CONDUIT_DATA_DIR", r"C:\test\conduit-data");
        assert_eq!(resolve().unwrap(), PathBuf::from(r"C:\test\conduit-data"));
        match old {
            Some(value) => std::env::set_var("CONDUIT_DATA_DIR", value),
            None => std::env::remove_var("CONDUIT_DATA_DIR"),
        }
    }
}
