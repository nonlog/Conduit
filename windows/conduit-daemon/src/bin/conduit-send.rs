#![windows_subsystem = "windows"]

use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            show_in_control_ui(&message);
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
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string());
    let daemon = std::env::current_exe()
        .map_err(|e| format!("Could not locate Conduit: {e}"))?
        .with_file_name("conduit-daemon.exe");
    let mut progress = |_: u64, _: u64| {};
    let mut error = invoke_send(&daemon, &file, &mut progress).err();

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
            match invoke_send(&daemon, &file, &mut progress) {
                Ok(()) => {
                    error = None;
                    break;
                }
                Err(next) => {
                    if !is_control_plane_unavailable(&next) {
                        error = Some(next);
                        break;
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

fn invoke_send(
    daemon: &Path,
    file: &Path,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    let mut child = Command::new(daemon)
        .arg("send")
        .arg(file)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", daemon.display()))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read Conduit send progress.".to_string())?;
    let mut normal_stdout = String::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("Could not read Conduit send progress: {e}"))?;
        if let Some((transferred, total)) = parse_progress_line(&line) {
            on_progress(transferred, total);
        } else {
            if !normal_stdout.is_empty() {
                normal_stdout.push('\n');
            }
            normal_stdout.push_str(&line);
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Could not wait for {}: {e}", daemon.display()))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = normal_stdout.trim().to_string();
    let raw = if !stderr.is_empty() { stderr } else { stdout };
    if raw.is_empty() {
        return Err(format!("The send helper exited with {}.", output.status));
    }
    Err(clean_error(&raw))
}

fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("PROGRESS ")?;
    let mut fields = rest.split_whitespace();
    let transferred = fields.next()?.parse().ok()?;
    let total = fields.next()?.parse().ok()?;
    (fields.next().is_none()).then_some((transferred, total))
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

    #[test]
    fn progress_lines_are_strict() {
        assert_eq!(parse_progress_line("PROGRESS 512 1024"), Some((512, 1024)));
        assert_eq!(parse_progress_line("PROGRESS 1"), None);
        assert_eq!(parse_progress_line("sent 1 2"), None);
    }
}
