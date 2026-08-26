//! Per-user Windows sign-in startup for the headless daemon.
//!
//! This deliberately uses HKCU\...\Run rather than a Windows Service. Clipboard and toast work
//! belong to the interactive user session, and a service would add privilege/session plumbing for
//! no benefit. The Run entry starts this console-subsystem binary through one short-lived hidden
//! PowerShell process so sign-in never flashes a terminal window; PowerShell exits immediately
//! after creating the daemon and is not part of Conduit's steady-state cost.

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
    // PowerShell single-quoted literals escape a literal quote by doubling it. Windows paths may
    // legally contain apostrophes, so do not assume an installation directory never will.
    let exe = exe.replace('\'', "''");
    format!(
        "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -Command \"Start-Process -FilePath '{exe}' -WindowStyle Hidden\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_spaces_and_apostrophes_without_a_resident_shell() {
        let command = command_for(r"C:\Users\O'Brien\My Apps\conduit-daemon.exe");
        assert!(command.contains("-WindowStyle Hidden"));
        assert!(command.contains(r"'C:\Users\O''Brien\My Apps\conduit-daemon.exe'"));
        assert!(command.contains("Start-Process"));
        // No `-Wait`: PowerShell launches the hidden daemon and exits instead of becoming a
        // second long-running companion process.
        assert!(!command.contains("-Wait"));
    }
}
