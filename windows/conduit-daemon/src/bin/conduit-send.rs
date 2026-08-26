#![windows_subsystem = "windows"]

//! On-demand Explorer bridge. This process has no window and no resident state.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            show_error(&message);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let file = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| "Explorer did not provide a file to send.".to_string())?;
    if std::env::args_os().nth(2).is_some() {
        return Err("Conduit currently sends one selected file at a time.".into());
    }

    let daemon = std::env::current_exe()
        .map_err(|e| format!("Could not locate Conduit: {e}"))?
        .with_file_name("conduit-daemon.exe");
    let status = Command::new(&daemon)
        .arg("send")
        .arg(&file)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Could not start {}: {e}", daemon.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Conduit could not send {}. Make sure the desktop daemon is linked to the phone.",
            file.display()
        ))
    }
}

fn show_error(message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let title = HSTRING::from("Conduit");
    let message = HSTRING::from(message);
    unsafe {
        let _ = MessageBoxW(None, &message, &title, MB_OK | MB_ICONERROR);
    }
}
