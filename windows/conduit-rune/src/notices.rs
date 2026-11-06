//! One finite, dismissible message. Deadlines use Rune's frame clock and one wake,
//! never a polling thread or a timer that keeps an idle window rendering.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Progress,
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub message: String,
    pub kind: Kind,
    pub pairing: bool,
    deadline: Option<f64>,
    lifetime: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick {
    pub expired: bool,
    pub wake_after: Option<Duration>,
}

impl Notice {
    pub fn new(message: impl Into<String>, kind: Kind) -> Self {
        Self {
            message: message.into(),
            kind,
            pairing: false,
            deadline: None,
            lifetime: (kind == Kind::Success).then_some(5.0),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, Kind::Error)
    }

    pub fn tick(&mut self, now: f64) -> Tick {
        if !now.is_finite() {
            return Tick::default();
        }
        if let Some(deadline) = self.deadline {
            return Tick {
                expired: now >= deadline,
                wake_after: None,
            };
        }
        if let Some(seconds) = self.lifetime {
            self.deadline = Some(now + seconds);
            return Tick {
                expired: false,
                wake_after: Some(Duration::from_secs_f64(seconds)),
            };
        }
        Tick::default()
    }
}

pub fn completion_message(operation: &str) -> &str {
    match operation {
        "Starting pairing" => "Pairing code is ready. Scan it on your phone.",
        "Cancelling pairing" => "Pairing cancelled",
        "Forgetting device" => "Device unpaired",
        "Opening file picker" | "Queueing files" => "File selection handled",
        "Clearing notifications" => "Notification history cleared",
        "Clearing links" => "Shared link history cleared",
        "Opening downloads" | "Opening data folder" => "Folder opened",
        "Opening link" | "Opening GitHub" => "Link opened",
        "Updating startup" => "Startup preference updated",
        "Updating Explorer" => "Explorer integration updated",
        _ => "Done",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_schedules_exactly_one_expiry_and_then_is_removed() {
        let mut message = Notice::new("Settings saved", Kind::Success);
        assert_eq!(message.tick(10.0).wake_after, Some(Duration::from_secs(5)));
        for now in [10.1, 11.0, 14.9] {
            assert_eq!(message.tick(now), Tick::default());
        }
        assert!(message.tick(15.0).expired);
    }

    #[test]
    fn old_wake_cannot_expire_a_new_message() {
        let mut first = Notice::new("First", Kind::Success);
        assert!(first.tick(0.0).wake_after.is_some());
        let mut replacement = Notice::new("Second", Kind::Success);
        assert!(replacement.tick(4.0).wake_after.is_some());
        assert!(!replacement.tick(5.0).expired);
        assert!(replacement.tick(9.0).expired);
    }

    #[test]
    fn errors_and_in_progress_operations_do_not_disappear_without_a_result() {
        for kind in [Kind::Progress, Kind::Error] {
            let mut message = Notice::new("Needs attention", kind);
            assert_eq!(message.tick(0.0), Tick::default());
            assert_eq!(message.tick(9999.0), Tick::default());
        }
        assert_eq!(
            completion_message("Starting pairing"),
            "Pairing code is ready. Scan it on your phone."
        );
    }
}
