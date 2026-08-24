//! Windows clipboard bridge.
//!
//! `AddClipboardFormatListener` on a message-only window is why there is no timer and
//! no polling here. Exactly one thread is created at startup and joined at shutdown;
//! clipboard traffic never adds a second one.

use anyhow::{anyhow, Result};
use clipboard_win::monitor::{Monitor, Shutdown};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Comfortably under the 65519-byte plaintext ceiling once `ClipText` and `Envelope`
/// framing are added. A longer clip is skipped with a log rather than truncated, and
/// rather than handed to `Session::send`, whose refusal would tear down the session.
const MAX_TEXT: usize = 64_000;

/// Depth 4: clipboard changes are human-paced, so a full ring means the session is
/// wedged and the newest clip is the only one worth keeping anyway. Broadcast rather
/// than mpsc because the bridge outlives sessions — each session subscribes, and a
/// clip copied while nothing is connected simply ages out.
const QUEUE: usize = 4;

/// Windows puts CRLF on the clipboard; the wire and Android both use LF.
fn to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn to_crlf(s: &str) -> String {
    to_lf(s).replace('\n', "\r\n")
}

pub struct Bridge {
    /// Last text seen in either direction, LF-normalised. An update equal to this is
    /// our own write coming back, and dropping it is the whole of ping-pong prevention.
    last: Arc<Mutex<String>>,
    tx: broadcast::Sender<String>,
    /// Dropping this posts the monitor's close message, which unblocks the thread.
    /// `Mutex` purely for the `Sync` bound `Arc<Bridge>` needs — the raw `HWND` inside
    /// makes `Shutdown` `Send` but not `Sync`, and it is only ever touched in [`Drop`].
    shutdown: Mutex<Option<Shutdown>>,
    thread: Option<JoinHandle<()>>,
}

impl Bridge {
    /// Starts the listener thread.
    pub fn start() -> Result<Self> {
        let (tx, _) = broadcast::channel(QUEUE);
        let last = Arc::new(Mutex::new(String::new()));

        // `Monitor` is not Send, so the thread builds it and hands back only the
        // shutdown handle. This channel also carries the construction error out.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_last = last.clone();
        let thread_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                let mut monitor = match Monitor::new() {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if ready_tx.send(Ok(monitor.shutdown_channel())).is_err() {
                    return; // caller gave up; drop the monitor and unwind
                }
                pump(&mut monitor, &thread_last, &thread_tx);
            })?;

        let shutdown = match ready_rx.recv() {
            Ok(Ok(shutdown)) => shutdown,
            Ok(Err(e)) => return Err(anyhow!("AddClipboardFormatListener failed: {e}")),
            Err(_) => {
                let _ = thread.join();
                return Err(anyhow!("clipboard thread died before it was ready"));
            }
        };
        info!("clipboard listener up");
        Ok(Self {
            last,
            tx,
            shutdown: Mutex::new(Some(shutdown)),
            thread: Some(thread),
        })
    }

    /// Local clipboard text, LF-normalised and already de-duplicated against our own
    /// writes. One subscription per session; the bridge outlives all of them.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Puts remote text on the local clipboard. Cheap enough to call from async: the
    /// only blocking part is `OpenClipboard`, which retries with `Sleep(0)` yields.
    pub fn apply(&self, text: &str) -> Result<()> {
        let normalised = to_lf(text);
        {
            // Recorded before the write, because the listener fires on another thread
            // and may observe the change before `set_clipboard_string` even returns.
            let mut last = self.last.lock().expect("clipboard mutex poisoned");
            if *last == normalised {
                debug!("remote clip already on the clipboard");
                return Ok(());
            }
            *last = normalised.clone();
        }
        clipboard_win::set_clipboard_string(&to_crlf(&normalised))
            .map_err(|e| anyhow!("SetClipboardData failed: {e}"))
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        // Order matters: the posted close message is what lets the join return.
        drop(self.shutdown.get_mut().expect("clipboard mutex poisoned").take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        info!("clipboard listener down");
    }
}

/// Blocks in `GetMessage` until shutdown. Returning drops the `Monitor`, which
/// removes the format listener and destroys the window.
fn pump(monitor: &mut Monitor, last: &Mutex<String>, tx: &broadcast::Sender<String>) {
    loop {
        match monitor.recv() {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                warn!(error = %e, "clipboard message loop failed");
                return;
            }
        }
        // No CF_UNICODETEXT is the normal case for a copied image, not an error.
        let Ok(raw) = clipboard_win::get_clipboard_string() else {
            debug!("clipboard update carried no text");
            continue;
        };
        let text = to_lf(&raw);
        if text.is_empty() {
            continue;
        }
        if text.len() > MAX_TEXT {
            warn!(bytes = text.len(), "clip too large for one frame, skipped");
            continue;
        }
        {
            let mut last = last.lock().expect("clipboard mutex poisoned");
            if *last == text {
                debug!("echo of our own write, dropped");
                continue;
            }
            *last = text.clone();
        }
        // No subscriber means no session is up, which is not an error: the clip is
        // simply not going anywhere.
        let _ = tx.send(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_round_trip_without_growing() {
        assert_eq!(to_lf("a\r\nb"), "a\nb");
        assert_eq!(to_crlf("a\nb"), "a\r\nb");
        // The idempotence that keeps `last` comparable in both directions: text that
        // has been through a write and back must normalise to what we stored.
        assert_eq!(to_lf(&to_crlf("a\nb")), "a\nb");
        assert_eq!(to_crlf("a\r\nb"), "a\r\nb", "must not double up existing CRLF");
    }
}
