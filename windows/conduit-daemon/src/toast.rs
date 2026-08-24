//! Native Windows toasts.
//!
//! Genuine `ToastNotification`s, not a balloon and not a custom window: the phone's
//! shade lands in Action Center, updates in place, and disappears when the phone's
//! notification does.
//!
//! Two things make this less obvious than it looks.
//!
//! First, `ToastNotifier` is COM apartment-bound, so it cannot be created on one tokio
//! worker and used from another. One dedicated thread owns it for the life of the
//! process and takes commands over a channel — the same shape as [`crate::clip`], and
//! for the same reason: notification traffic must not add a thread per message.
//!
//! Second, an unpackaged Win32 process has no package identity, so
//! `CreateToastNotifierWithId` needs an AppUserModelID that Windows can resolve to a
//! display name. Registering one under `HKCU\Software\Classes\AppUserModelId` is the
//! lightest way to get that; the alternative is a Start Menu shortcut carrying the ID
//! as a shell property, which means COM `IShellLink` plumbing for the same result.

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::sync::mpsc;
use std::thread::JoinHandle;
use tracing::{debug, info, warn};
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{
    NotificationData, ToastNotification, ToastNotificationManager, ToastNotifier,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Must match the registry key below. Reverse-DNS-ish because that is the convention
/// Windows uses for AUMIDs and it keeps us out of anyone else's namespace.
const AUMID: &str = "Conduit.Desktop";

/// Every toast shares one group, so a single call clears the lot on shutdown and the
/// phone's tag alone identifies a notification within it.
const GROUP: &str = "conduit";

/// Windows caps a toast tag at 64 characters. Android keys — `0|com.foo|1234|null|10123`
/// — routinely exceed that, so the tag is a digest of the key rather than the key. It is
/// derived, not remembered, which is why update and removal need no state on this side.
fn tag_for(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..8]
        .iter()
        .fold(String::with_capacity(16), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// The app name is the one value inlined into the XML, so it is the one that needs
/// escaping. Title and body travel as bound data and never touch the parser.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub enum Cmd {
    Show {
        key: String,
        app: String,
        title: String,
        body: String,
    },
    /// Same key, new text. Silent — Windows does not re-alert on an update, which is the
    /// point: a chat thread gaining a message should not pop a second time.
    Update {
        key: String,
        title: String,
        body: String,
    },
    Hide {
        key: String,
    },
}

pub struct Notifier {
    /// `Option` so [`Drop`] can close the channel before joining; the closed channel is
    /// what makes the thread's `recv` return and the join finish.
    tx: Option<mpsc::Sender<Cmd>>,
    thread: Option<JoinHandle<()>>,
}

impl Notifier {
    pub fn start() -> Result<Self> {
        register_aumid().context("registering the toast AppUserModelID")?;

        let (tx, rx) = mpsc::channel();
        // The notifier is built on the thread that will use it, and only the outcome
        // comes back — a COM object cannot cross apartments.
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("toast".into())
            .spawn(move || {
                // MTA: this thread never pumps messages, so an STA would deadlock the
                // moment WinRT needed to marshal a call into it.
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if com.is_err() {
                    let _ = ready_tx.send(Err(anyhow!("CoInitializeEx failed: {com:?}")));
                    return;
                }
                match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) {
                    Ok(notifier) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            pump(&notifier, rx);
                        }
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow!("CreateToastNotifierWithId: {e}")));
                    }
                }
                unsafe { CoUninitialize() };
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                let _ = thread.join();
                return Err(anyhow!("toast thread died before it was ready"));
            }
        }
        info!(aumid = AUMID, "toast notifier up");
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    /// Never fails the caller: a toast that cannot be shown is not worth ending a
    /// session over, and the thread logs its own errors.
    pub fn post(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        info!("toast notifier down");
    }
}

fn pump(notifier: &ToastNotifier, rx: mpsc::Receiver<Cmd>) {
    while let Ok(cmd) = rx.recv() {
        let result = match cmd {
            Cmd::Show {
                key,
                app,
                title,
                body,
            } => show(notifier, &key, &app, &title, &body),
            Cmd::Update { key, title, body } => update(notifier, &key, &title, &body),
            Cmd::Hide { key } => hide(&key),
        };
        if let Err(e) = result {
            warn!(error = %e, "toast failed");
        }
    }
}

/// `{title}` and `{body}` are data-bound placeholders, which is what makes
/// [`update`] possible at all: `ToastNotifier::Update` can only touch bound values.
fn show(notifier: &ToastNotifier, key: &str, app: &str, title: &str, body: &str) -> Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(format!(
        r#"<toast>
             <visual>
               <binding template="ToastGeneric">
                 <text>{{title}}</text>
                 <text>{{body}}</text>
                 <text placement="attribution">{}</text>
               </binding>
             </visual>
           </toast>"#,
        escape(app)
    )))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let tag = tag_for(key);
    toast.SetTag(&HSTRING::from(&tag))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    toast.SetData(&data(title, body)?)?;
    notifier.Show(&toast)?;
    debug!(%tag, "toast shown");
    Ok(())
}

fn update(notifier: &ToastNotifier, key: &str, title: &str, body: &str) -> Result<()> {
    let tag = tag_for(key);
    // NotificationUpdateResult::Failed comes back when the toast has already aged out of
    // Action Center. Nothing to do about it and nothing wrong, so it is not an error.
    let outcome = notifier.UpdateWithTagAndGroup(
        &data(title, body)?,
        &HSTRING::from(&tag),
        &HSTRING::from(GROUP),
    )?;
    debug!(%tag, ?outcome, "toast updated");
    Ok(())
}

fn hide(key: &str) -> Result<()> {
    let tag = tag_for(key);
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(
        &HSTRING::from(&tag),
        &HSTRING::from(GROUP),
        &HSTRING::from(AUMID),
    )?;
    debug!(%tag, "toast removed");
    Ok(())
}

/// Sequence number 0 means "apply unconditionally". A non-zero sequence would make
/// Windows reject data older than what it has, which needs a counter per toast; the
/// single sender thread already guarantees order, so the counter would be dead weight.
fn data(title: &str, body: &str) -> Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    values.Insert(&HSTRING::from("title"), &HSTRING::from(title))?;
    values.Insert(&HSTRING::from("body"), &HSTRING::from(body))?;
    data.SetSequenceNumber(0)?;
    Ok(data)
}

/// Idempotent. Without this the notifier is created but every `Show` fails, because
/// Windows cannot resolve the AUMID to anything it is willing to attribute a toast to.
fn register_aumid() -> Result<()> {
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{AUMID}"))?;
    key.set_string("DisplayName", "conduit")?;
    // On, so conduit gets its own entry in Settings -> Notifications and the toasts can
    // be muted there like any other app's rather than only by killing the daemon.
    key.set_u32("ShowInActionCenter", 1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_fit_the_platform_limit_and_stay_stable() {
        let key = "0|com.tencent.mm|1234567|null|10123";
        let tag = tag_for(key);
        assert_eq!(tag.len(), 16, "16 hex chars, far inside the 64-char cap");
        assert_eq!(tag, tag_for(key), "derived, so update and removal find it again");
        assert_ne!(tag, tag_for("0|com.tencent.mm|1234568|null|10123"));
        // A key long past the tag limit still produces a legal tag, which is the whole
        // reason the key is not used directly.
        assert_eq!(tag_for(&"x".repeat(4096)).len(), 16);
    }

    #[test]
    fn app_names_cannot_break_out_of_the_xml() {
        assert_eq!(
            escape(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&apos;"
        );
        // Ampersand first, or the escapes escape each other into nonsense.
        assert_eq!(escape("&lt;"), "&amp;lt;");
    }

    /// Pops a real toast, so it is opt-in: `cargo test -p conduit-daemon -- --ignored`.
    ///
    /// Worth keeping despite that, because it is the only thing that answers the
    /// question the unit tests cannot: whether an unpackaged process with nothing but a
    /// registry AUMID is allowed to show, update and withdraw a toast on this machine.
    /// `Update` returning `Succeeded` is the load-bearing assertion — it proves Windows
    /// found our toast by tag *and* that the XML really had bindable placeholders to
    /// write into, which is what the update path depends on.
    #[test]
    #[ignore = "shows a real toast on the desktop"]
    fn a_toast_shows_updates_and_withdraws() -> Result<()> {
        let key = "0|com.conduit.test|1|null|10000";
        let tag = tag_for(key);
        let notifier = Notifier::start()?;

        notifier.post(Cmd::Show {
            key: key.into(),
            app: "conduit self-test".into(),
            title: "First title".into(),
            body: "If this reads First title, data binding renders.".into(),
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(in_history(&tag)?, "Show did not reach Action Center");

        notifier.post(Cmd::Update {
            key: key.into(),
            title: "Second title".into(),
            body: "Updated in place, and it must not have popped again.".into(),
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(in_history(&tag)?, "Update removed the toast instead");
        // The binding proof: the new title is readable off the live toast, so the
        // placeholders in the XML really are bound keys and Update wrote into them.
        // Without this the test would pass just as happily on a toast rendering the
        // literal text "{title}".
        assert_eq!(
            bound_value(&tag, "title")?.as_deref(),
            Some("Second title"),
            "the toast in Action Center did not take the updated bound value"
        );

        notifier.post(Cmd::Hide { key: key.into() });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!in_history(&tag)?, "Hide left the toast behind");
        Ok(())
    }

    #[cfg(test)]
    fn in_history(tag: &str) -> Result<bool> {
        Ok(from_history(tag)?.is_some())
    }

    #[cfg(test)]
    fn from_history(tag: &str) -> Result<Option<ToastNotification>> {
        let history = ToastNotificationManager::History()?.GetHistoryWithId(&HSTRING::from(AUMID))?;
        for toast in history {
            if toast.Tag()? == HSTRING::from(tag) {
                return Ok(Some(toast));
            }
        }
        Ok(None)
    }

    #[cfg(test)]
    fn bound_value(tag: &str, name: &str) -> Result<Option<String>> {
        let Some(toast) = from_history(tag)? else {
            return Ok(None);
        };
        let values = toast.Data()?.Values()?;
        let key = HSTRING::from(name);
        Ok(values
            .HasKey(&key)?
            .then(|| values.Lookup(&key))
            .transpose()?
            .map(|v| v.to_string()))
    }
}
