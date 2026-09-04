use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Mutable Conduit state lives beside the executables in `data/` for portable installs.
///
/// Scoop installs are a special case: the persisted directory is authoritative even if a bad
/// development overlay accidentally replaces Scoop's `current\\data` junction with a plain
/// directory. Resolving the persist path directly prevents identity rotation and settings loss.
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

    if let Some(persisted) = scoop_persist_from_install(parent) {
        migrate_portable_into_persist_if_needed(&portable, &persisted)?;
        std::fs::create_dir_all(&persisted).with_context(|| {
            format!(
                "creating Scoop Conduit data directory {}",
                persisted.display()
            )
        })?;
        return Ok(persisted);
    }

    migrate_legacy_if_needed(&portable)?;
    Ok(portable)
}

/// Recognise Scoop's `<root>\\apps\\conduit\\<version-or-current>` layout without depending on
/// Scoop being on PATH. This also works while Scoop is running an installer from a version folder.
fn scoop_persist_from_install(install: &Path) -> Option<PathBuf> {
    let conduit = install.parent()?;
    if !component_eq(conduit, "conduit") {
        return None;
    }
    let apps = conduit.parent()?;
    if !component_eq(apps, "apps") {
        return None;
    }
    let root = apps.parent()?;
    Some(root.join("persist").join("conduit").join("data"))
}

fn component_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

/// If a broken overlay already created a plain `current\\data`, salvage files that do not yet
/// exist in the real Scoop persist directory. Existing persisted identity/config always win.
fn migrate_portable_into_persist_if_needed(portable: &Path, persisted: &Path) -> Result<()> {
    if !portable.is_dir() {
        return Ok(());
    }

    std::fs::create_dir_all(persisted)
        .with_context(|| format!("creating {}", persisted.display()))?;

    let same_target = match (
        std::fs::canonicalize(portable),
        std::fs::canonicalize(persisted),
    ) {
        (Ok(source), Ok(destination)) => source == destination,
        _ => false,
    };
    if same_target {
        return Ok(());
    }

    copy_tree(portable, persisted).with_context(|| {
        format!(
            "recovering Conduit data from {} to {}",
            portable.display(),
            persisted.display()
        )
    })
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

    #[test]
    fn scoop_layout_resolves_persist_directory() {
        assert_eq!(
            scoop_persist_from_install(Path::new(r"D:\Programs\Scoop\apps\conduit\current")),
            Some(PathBuf::from(r"D:\Programs\Scoop\persist\conduit\data"))
        );
        assert_eq!(
            scoop_persist_from_install(Path::new(r"D:\Programs\Scoop\apps\conduit\0.1.1")),
            Some(PathBuf::from(r"D:\Programs\Scoop\persist\conduit\data"))
        );
    }

    #[test]
    fn portable_layout_does_not_look_like_scoop() {
        assert_eq!(
            scoop_persist_from_install(Path::new(r"D:\Portable\Conduit")),
            None
        );
    }
}
