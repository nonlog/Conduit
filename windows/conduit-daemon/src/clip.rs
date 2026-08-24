//! Windows clipboard bridge.
//!
//! `AddClipboardFormatListener` on a message-only window is why there is no timer and
//! no polling here. Exactly one thread is created at startup and joined at shutdown;
//! clipboard traffic never adds a second one, and neither does an image — transcoding
//! happens inline on this same thread, because it is already the thread that must not
//! be a tokio worker.

use anyhow::{anyhow, Result};
use clipboard_win::monitor::{Monitor, Shutdown};
use clipboard_win::{formats, Getter as _, Setter as _};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::image;

/// Comfortably under the 65519-byte plaintext ceiling once `ClipText` and `Envelope`
/// framing are added. A longer clip is skipped with a log rather than truncated, and
/// rather than handed to `Session::send`, whose refusal would tear down the session.
const MAX_TEXT: usize = 64_000;

/// Matches `MAX_MESSAGE` in the proto. Chunking means the frame ceiling does not apply,
/// but something must bound it, and a clipboard is not a file transfer: 10 MiB is a
/// generous phone screenshot and a cheap thing to refuse before allocating.
pub const MAX_IMAGE: usize = 10 * 1024 * 1024;

/// Depth 4: clipboard changes are human-paced, so a full ring means the session is
/// wedged and the newest clip is the only one worth keeping anyway. Broadcast rather
/// than mpsc because the bridge outlives sessions — each session subscribes, and a
/// clip copied while nothing is connected simply ages out.
const QUEUE: usize = 4;

/// What one clipboard change carries.
///
/// `Arc` on the image because every session subscriber gets a clone of this value, and
/// a broadcast ring of depth 4 would otherwise hold four copies of a 10 MiB screenshot.
#[derive(Clone, Debug)]
pub enum Clip {
    Text(String),
    /// Always PNG. The wire format is PNG in both directions, so a `CF_DIB` is
    /// transcoded on the way out and back on the way in, never sent raw.
    Image(Arc<Vec<u8>>),
}

/// Windows puts CRLF on the clipboard; the wire and Android both use LF.
fn to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn to_crlf(s: &str) -> String {
    to_lf(s).replace('\n', "\r\n")
}

/// Identifies clipboard content for echo suppression.
///
/// Text is compared by value, images by length. A digest would be more precise, but the
/// two images this has to tell apart are "the one we just wrote" and "a different one a
/// human copied", and those differ in length essentially always — the failure mode is a
/// re-copy of an image that is byte-identical to the last one, which is a no-op anyway.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
enum Seen {
    #[default]
    Nothing,
    Text(String),
    Image(usize),
}

pub struct Bridge {
    /// Last content seen in either direction. An update equal to this is our own write
    /// coming back, and dropping it is the whole of ping-pong prevention.
    last: Arc<Mutex<Seen>>,
    tx: broadcast::Sender<Clip>,
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
        let last = Arc::new(Mutex::new(Seen::Nothing));

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

    /// Local clipboard content, already de-duplicated against our own writes. One
    /// subscription per session; the bridge outlives all of them.
    pub fn subscribe(&self) -> broadcast::Receiver<Clip> {
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
            if *last == Seen::Text(normalised.clone()) {
                debug!("remote clip already on the clipboard");
                return Ok(());
            }
            *last = Seen::Text(normalised.clone());
        }
        clipboard_win::set_clipboard_string(&to_crlf(&normalised))
            .map_err(|e| anyhow!("SetClipboardData failed: {e}"))
    }

    /// Puts a remote PNG on the local clipboard, in both formats Windows apps expect.
    ///
    /// Blocking and slow enough to matter — a 10 MiB screenshot is a decode plus an
    /// encode — so callers hand this to `spawn_blocking` rather than run it on a worker.
    pub fn apply_image(&self, png: &[u8]) -> Result<()> {
        {
            let mut last = self.last.lock().expect("clipboard mutex poisoned");
            if *last == Seen::Image(png.len()) {
                debug!("remote image already on the clipboard");
                return Ok(());
            }
            *last = Seen::Image(png.len());
        }
        let dib = image::png_to_dib(png)?;
        let png_format = png_format().ok_or_else(|| anyhow!("could not register the PNG format"))?;

        // One open session for both formats. Two `set_clipboard` calls would each empty
        // the clipboard first, so the second would delete the first — and an app that
        // reads only `CF_DIB`, or only PNG, would find nothing.
        let mut wrote: Result<()> = Ok(());
        clipboard_win::with_clipboard(|| {
            wrote = (|| {
                clipboard_win::raw::empty().map_err(|e| anyhow!("EmptyClipboard failed: {e}"))?;
                // PNG first: it is the lossless original, and paste targets that
                // understand it should not have to fall back.
                formats::RawData(png_format)
                    .write_clipboard(&png)
                    .map_err(|e| anyhow!("writing PNG to the clipboard failed: {e}"))?;
                formats::Bitmap
                    .write_clipboard(&dib)
                    .map_err(|e| anyhow!("writing CF_DIB to the clipboard failed: {e}"))
            })();
        })
        .map_err(|e| anyhow!("OpenClipboard failed: {e}"))?;
        wrote
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

/// The registered `"PNG"` clipboard format, which is what browsers, Chromium apps and
/// modern screenshot tools put alongside their `CF_DIB`. Registering an existing name
/// returns the existing id, so this is a lookup after the first call.
fn png_format() -> Option<u32> {
    clipboard_win::raw::register_format("PNG").map(|id| id.get())
}

/// Reads whatever the clipboard now holds, or `None` if it is nothing we carry.
///
/// PNG is preferred over `CF_DIB` because it needs no conversion and, unlike a DIB,
/// still has an alpha channel. `CF_DIB` is the fallback for the many apps that only
/// offer that — Paint, Office, most of Win32.
fn read_clipboard() -> Option<Clip> {
    if let Some(format) = png_format() {
        let mut png = Vec::new();
        if formats::RawData(format).read_clipboard(&mut png).is_ok() && !png.is_empty() {
            debug!(bytes = png.len(), "clipboard carried a PNG");
            return Some(Clip::Image(Arc::new(png)));
        }
    }
    let mut dib = Vec::new();
    if formats::Bitmap.read_clipboard(&mut dib).is_ok() && !dib.is_empty() {
        debug!(bytes = dib.len(), "clipboard carried a DIB");
        return match image::dib_to_png(&dib) {
            Ok(png) => Some(Clip::Image(Arc::new(png))),
            Err(e) => {
                warn!(error = %e, "could not transcode the clipboard DIB");
                None
            }
        };
    }
    // No text either is the normal case for a copied file or a custom format.
    let raw = clipboard_win::get_clipboard_string().ok()?;
    let text = to_lf(&raw);
    (!text.is_empty()).then_some(Clip::Text(text))
}

/// Blocks in `GetMessage` until shutdown. Returning drops the `Monitor`, which
/// removes the format listener and destroys the window.
fn pump(monitor: &mut Monitor, last: &Mutex<Seen>, tx: &broadcast::Sender<Clip>) {
    loop {
        match monitor.recv() {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                warn!(error = %e, "clipboard message loop failed");
                return;
            }
        }
        let Some(clip) = read_clipboard() else {
            debug!("clipboard update carried nothing we sync");
            continue;
        };

        let seen = match &clip {
            Clip::Text(text) => {
                if text.len() > MAX_TEXT {
                    warn!(bytes = text.len(), "clip too large for one frame, skipped");
                    continue;
                }
                Seen::Text(text.clone())
            }
            Clip::Image(png) => {
                if png.len() > MAX_IMAGE {
                    warn!(bytes = png.len(), "image too large to sync, skipped");
                    continue;
                }
                Seen::Image(png.len())
            }
        };
        {
            let mut last = last.lock().expect("clipboard mutex poisoned");
            if *last == seen {
                debug!("echo of our own write, dropped");
                continue;
            }
            *last = seen;
        }
        // No subscriber means no session is up, which is not an error: the clip is
        // simply not going anywhere.
        let _ = tx.send(clip);
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

    #[test]
    fn text_and_an_image_are_never_mistaken_for_each_other() {
        // Both arms of `Seen` are compared with the same `==`, so a bridge that
        // stored the wrong arm would suppress a real clip. Length collision between
        // the two variants must not read as equal.
        assert_ne!(Seen::Text("abcd".into()), Seen::Image(4));
        assert_eq!(Seen::Image(4), Seen::Image(4));
        assert_ne!(Seen::Image(4), Seen::Image(5));
        assert_ne!(Seen::Nothing, Seen::Text(String::new()));
    }

    #[test]
    fn the_png_clipboard_format_is_stable_across_calls() {
        // Registration must be idempotent, or `apply_image` would write under one id
        // and `read_clipboard` would look under another — a silent no-paste.
        let first = png_format().expect("PNG format should register");
        assert_eq!(Some(first), png_format());
        // Registered formats live above 0xC000; a smaller value would mean we had
        // collided with a predefined format such as CF_DIB.
        assert!(first >= 0xC000, "unexpected id {first:#x}");
    }
}
