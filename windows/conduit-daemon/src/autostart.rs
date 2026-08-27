//! Per-user Windows sign-in startup for the headless daemon.
//!
//! This deliberately uses HKCU\...\Run rather than a Windows Service. Clipboard and toast work
//! belong to the interactive user session, and a service would add privilege/session plumbing for
//! no benefit. The daemon itself is a Windows-GUI-subsystem executable, so the Run entry can launch
//! it directly without PowerShell, cmd.exe, or any console flash.

use anyhow::{Context, Result};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Conduit";

pub fn install() -> Result<String> {
    let command = command()?;
    let key = windows_registry::CURRENT_USER
        .create(RUN_KEY)
        .context("opening the current-user Run registry key")?;
    key.set_string(VALUE, &command)
        .context("registering Conduit at sign-in")?;
    Ok(command)
}

pub fn remove() -> Result<bool> {
    let key = windows_registry::CURRENT_USER
        .create(RUN_KEY)
        .context("opening the current-user Run registry key")?;
    if key.get_string(VALUE).is_err() {
        return Ok(false);
    }
    key.remove_value(VALUE)
        .context("removing Conduit from sign-in startup")?;
    Ok(true)
}

pub fn status() -> Option<String> {
    windows_registry::CURRENT_USER
        .open(RUN_KEY)
        .ok()?
        .get_string(VALUE)
        .ok()
}

fn command() -> Result<String> {
    let exe = std::env::current_exe().context("finding the Conduit executable")?;
    Ok(command_for(&exe.to_string_lossy()))
}

fn command_for(exe: &str) -> String {
    format!("\"{exe}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_is_a_direct_quoted_gui_subsystem_executable() {
        let command = command_for(r"C:\Users\O'Brien\My Apps\conduit-daemon.exe");
        assert_eq!(command, r#""C:\Users\O'Brien\My Apps\conduit-daemon.exe""#);
        assert!(!command.to_ascii_lowercase().contains("powershell"));
        assert!(!command.to_ascii_lowercase().contains("cmd.exe"));
    }
}
