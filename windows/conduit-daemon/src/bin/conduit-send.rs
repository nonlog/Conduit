#![windows_subsystem = "windows"]

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const AUMID: &str = "Conduit.Desktop";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            show_failure(&message);
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
    let mut error = invoke_send(&daemon, &file).err();

    // Explorer can invoke this helper before the sign-in daemon has finished creating its
    // control pipe. Only that pre-request failure is safe to retry: once the resident daemon has
    // accepted a file path, retrying could create a duplicate on the phone.
    if error.as_deref().is_some_and(is_control_plane_unavailable) {
        let _ = Command::new(&daemon)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        for _ in 0..4 {
            thread::sleep(Duration::from_millis(250));
            match invoke_send(&daemon, &file) {
                Ok(()) => return Ok(()),
                Err(next) => {
                    if !is_control_plane_unavailable(&next) {
                        return Err(send_error(&file, &next));
                    }
                    error = Some(next);
                }
            }
        }
    }

    match error {
        None => Ok(()),
        Some(reason) => Err(send_error(&file, &reason)),
    }
}

fn invoke_send(daemon: &Path, file: &Path) -> Result<(), String> {
    let output = Command::new(daemon)
        .arg("send")
        .arg(file)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        // Never inherit Explorer's nonexistent standard handles. The old helper inherited them,
        // and the GUI-subsystem daemon could successfully publish the file then fail while
        // printing its success line, producing a false "could not send" dialog.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Could not start {}: {e}", daemon.display()))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let raw = if !stderr.is_empty() { stderr } else { stdout };
    if raw.is_empty() {
        return Err(format!("The send helper exited with {}.", output.status));
    }
    Err(clean_error(&raw))
}

fn clean_error(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if let Some(rest) = value.strip_prefix("Error: ") {
        value = rest.trim().to_string();
    }
    value
}

fn is_control_plane_unavailable(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("opening the conduit local control pipe")
        || error.contains("conduit daemon is not accepting local requests")
}

fn send_error(file: &Path, reason: &str) -> String {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string());
    format!("Couldn’t send {name}.\n\n{reason}")
}

fn show_failure(message: &str) {
    if show_failure_toast(message).is_ok() {
        return;
    }
    show_in_control_ui(message);
}

fn show_in_control_ui(message: &str) {
    let Ok(ui) = std::env::current_exe().map(|path| path.with_file_name("Conduit.exe")) else {
        return;
    };
    let _ = Command::new(ui)
        .arg("--send-error")
        .arg(message)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn show_failure_toast(message: &str) -> windows::core::Result<()> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let result = (|| {
        let xml = XmlDocument::new()?;
        let markup = format!(
            r#"<toast><visual><binding template="ToastGeneric"><text>Couldn’t send file</text><text>{}</text></binding></visual></toast>"#,
            escape_xml(message)
        );
        xml.LoadXml(&HSTRING::from(markup))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
        notifier.Show(&toast)
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_real_daemon_error_instead_of_a_generic_link_hint() {
        assert_eq!(
            clean_error("Error: phone did not publish the file: Downloads provider refused x\r\n"),
            "phone did not publish the file: Downloads provider refused x"
        );
    }

    #[test]
    fn only_pre_request_control_failures_are_retryable() {
        assert!(is_control_plane_unavailable(
            "opening the Conduit local control pipe: The system cannot find the file specified. (os error 2)"
        ));
        assert!(!is_control_plane_unavailable(
            "the Conduit session ended before the phone confirmed the file"
        ));
        assert!(!is_control_plane_unavailable(
            "phone did not publish the file: no storage"
        ));
    }
}
