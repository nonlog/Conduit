//! Windows Explorer's send helper starts the UI with `--send-error` if a file
//! cannot be sent. Keep the message until React has subscribed, including when
//! another launch is forwarded to an already-running Tauri window.

use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct LaunchNotices(Mutex<VecDeque<String>>);

impl LaunchNotices {
    pub fn push(&self, message: String) {
        if let Ok(mut queue) = self.0.lock() {
            if queue.len() == 8 {
                queue.pop_front();
            }
            queue.push_back(message);
        }
    }

    pub fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default()
    }
}

pub fn send_error_from_args(args: &[String]) -> Option<String> {
    if args.len() != 3 || args[1] != "--send-error" {
        return None;
    }
    let message: String = args[2]
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(1024)
        .collect();
    let message = message.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_messages_are_allowlisted_and_bounded() {
        let args = vec![
            "Conduit.exe".into(),
            "--send-error".into(),
            "Failed: x\0y".into(),
        ];
        assert_eq!(send_error_from_args(&args).as_deref(), Some("Failed: xy"));
        assert!(
            send_error_from_args(&["Conduit.exe".into(), "--unknown".into(), "x".into()]).is_none()
        );
        assert!(send_error_from_args(&["Conduit.exe".into(), "--send-error".into()]).is_none());
        let long = vec![
            "Conduit.exe".into(),
            "--send-error".into(),
            "x".repeat(5000),
        ];
        assert_eq!(send_error_from_args(&long).unwrap().chars().count(), 1024);
    }

    #[test]
    fn launch_notices_are_drained_only_once() {
        let queue = LaunchNotices::default();
        for index in 0..11 {
            queue.push(format!("error-{index}"));
        }
        let collected = queue.take();
        assert_eq!(collected.len(), 8);
        assert_eq!(collected[0], "error-3");
        assert_eq!(collected[7], "error-10");
        assert!(queue.take().is_empty());
    }
}
