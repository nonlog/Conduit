//! Per-user Explorer context-menu integration.
//!
//! This is deliberately registry-only and non-resident. Explorer starts the tiny GUI-subsystem
//! `conduit-send.exe` helper only when the user invokes the verb; the helper launches the existing
//! `send` CLI without a console, waits for the phone's publication result, then exits.

use anyhow::{Context, Result};
use std::ptr::null;
use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const VERB_KEY: &str = r"Software\Classes\*\shell\Conduit.SendToPhone";
const COMMAND_KEY: &str = r"Software\Classes\*\shell\Conduit.SendToPhone\command";
const VERB_LABEL: &str = "Send with Conduit";

pub fn install() -> Result<String> {
    let exe = std::env::current_exe().context("finding the Conduit executable")?;
    let helper = exe.with_file_name("conduit-send.exe");
    if !helper.is_file() {
        anyhow::bail!(
            "{} is missing; build/install conduit-send.exe beside the daemon first",
            helper.display()
        );
    }
    let command = command_for(&helper.to_string_lossy());
    let icon = exe.with_file_name("conduit-icon.ico");
    let icon_spec = if icon.is_file() {
        format!("\"{}\",0", icon.to_string_lossy())
    } else {
        format!("\"{}\",0", exe.to_string_lossy())
    };

    let verb = windows_registry::CURRENT_USER
        .create(VERB_KEY)
        .context("creating the Conduit Explorer verb")?;
    verb.set_string("", VERB_LABEL)?;
    verb.set_string("Icon", &icon_spec)?;
    // The transport currently serialises explicit file sends; do not imply multi-select support
    // in Explorer until the shell command can report a batch result coherently.
    verb.set_string("MultiSelectModel", "Single")?;

    windows_registry::CURRENT_USER
        .create(COMMAND_KEY)?
        .set_string("", &command)?;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
    Ok(command)
}

pub fn remove() -> Result<bool> {
    if windows_registry::CURRENT_USER.open(VERB_KEY).is_err() {
        return Ok(false);
    }
    windows_registry::CURRENT_USER
        .remove_tree(VERB_KEY)
        .context("removing the Conduit Explorer verb")?;
    Ok(true)
}

pub fn status() -> Option<String> {
    windows_registry::CURRENT_USER
        .open(COMMAND_KEY)
        .ok()?
        .get_string("")
        .ok()
}

fn command_for(helper: &str) -> String {
    // Windows filenames cannot contain a double quote, so the classic shell `%1` substitution is
    // safe when both executable and file are quoted. Apostrophes are ordinary argv characters and
    // never pass through a scripting language.
    format!("\"{helper}\" \"%1\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_command_is_plain_argv_not_a_script() {
        let command = command_for(r"C:\Apps\Conduit's tools\conduit-send.exe");
        assert!(command.starts_with(r#""C:\Apps\Conduit's tools\conduit-send.exe""#));
        assert!(command.ends_with("\"%1\""));
        assert!(!command.contains("powershell"));
    }

    #[test]
    fn explorer_verb_uses_the_short_product_label() {
        assert_eq!(VERB_LABEL, "Send with Conduit");
    }
}
