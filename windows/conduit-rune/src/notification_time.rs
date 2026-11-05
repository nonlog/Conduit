//! Local, absolute notification dates. Nothing here runs a clock or polls the OS.
//! Windows performs historical timezone/DST conversion when a record is displayed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub day_key: String,
    pub day: String,
    pub time: String,
    pub full: String,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Calendar {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millis: u16,
}

fn label(value: Calendar) -> Option<Label> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if !(1..=12).contains(&value.month)
        || !(1..=31).contains(&value.day)
        || !(1601..=9999).contains(&value.year)
        || value.hour > 23
        || value.minute > 59
    {
        return None;
    }
    let day_key = format!("{:04}-{:02}-{:02}", value.year, value.month, value.day);
    let day = format!(
        "{:02} {} {}",
        value.day,
        MONTHS[usize::from(value.month - 1)],
        value.year
    );
    let time = format!("{:02}:{:02}", value.hour, value.minute);
    let full = format!("{day} at {time}");
    Some(Label {
        day_key,
        day,
        time,
        full,
    })
}

#[cfg(windows)]
fn local_calendar(timestamp_ms: u64) -> Option<Calendar> {
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(file: *const FileTime, system: *mut Calendar) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const core::ffi::c_void,
            universal: *const Calendar,
            local: *mut Calendar,
        ) -> i32;
    }
    let ticks = timestamp_ms
        .checked_mul(10_000)?
        .checked_add(116_444_736_000_000_000)?;
    let file = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let mut utc = Calendar::default();
    let mut local = Calendar::default();
    // These functions only write the fixed-sized structures above; null zone
    // selects the current Windows timezone rather than treating UTC as local.
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
    }
    Some(local)
}

#[cfg(not(windows))]
fn local_calendar(_: u64) -> Option<Calendar> {
    None
}

pub fn format(timestamp_ms: u64) -> Label {
    (timestamp_ms != 0)
        .then(|| local_calendar(timestamp_ms))
        .flatten()
        .and_then(label)
        .unwrap_or_else(|| Label {
            day_key: "unknown".into(),
            day: "Date unavailable".into(),
            time: "Unknown time".into(),
            full: "Notification time unavailable".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_time_are_visible_and_have_a_stable_group_key() {
        let stamp = label(Calendar {
            year: 2026,
            month: 11,
            day: 5,
            hour: 9,
            minute: 7,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(stamp.day_key, "2026-11-05");
        assert_eq!(stamp.day, "05 Nov 2026");
        assert_eq!(stamp.time, "09:07");
        assert_eq!(stamp.full, "05 Nov 2026 at 09:07");
    }

    #[test]
    fn invalid_or_overflowing_timestamps_never_break_notification_layout() {
        assert_eq!(format(0).day_key, "unknown");
        assert_eq!(format(u64::MAX).day_key, "unknown");
        assert!(label(Calendar::default()).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn real_unix_timestamp_is_converted_with_windows_local_timezone() {
        let stamp = format(1_793_836_800_000);
        assert_ne!(stamp.day_key, "unknown");
        assert_eq!(stamp.time.len(), 5);
        assert!(stamp.full.contains("2026"));
    }
}
