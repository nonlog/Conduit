//! Native message loops can be stuck inside a third-party COM or shell callback.
//! Request shutdown normally first; never let a destructor wait forever on that callback.

use std::os::windows::io::AsRawHandle;
use std::thread::JoinHandle;
use std::time::Duration;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

pub fn join_native(thread: JoinHandle<()>, name: &str) {
    join_with_deadline(thread, name, Duration::from_secs(1));
}

fn join_with_deadline(thread: JoinHandle<()>, name: &str, limit: Duration) {
    let milliseconds = limit.as_millis().min(u128::from(u32::MAX - 1)) as u32;
    let result = unsafe { WaitForSingleObject(thread.as_raw_handle(), milliseconds) };
    if result == WAIT_OBJECT_0 {
        if thread.join().is_err() {
            tracing::warn!(worker = name, "native worker panicked during shutdown");
        }
    } else {
        // Dropping JoinHandle detaches only this already-stopping thread. The
        // process can then complete normally; no terminate-process or kill loop.
        tracing::warn!(
            worker = name,
            result,
            "native worker did not finish within shutdown deadline"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_finished_workers_and_does_not_wait_forever_for_blocked_ones() {
        let finished = std::thread::spawn(|| {});
        join_with_deadline(finished, "test-finished", Duration::from_secs(1));
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let blocked = std::thread::spawn(move || {
            let _ = wait.recv();
        });
        let start = std::time::Instant::now();
        join_with_deadline(blocked, "test-blocked", Duration::from_millis(10));
        assert!(start.elapsed() < Duration::from_secs(1));
        let _ = release.send(());
    }
}
