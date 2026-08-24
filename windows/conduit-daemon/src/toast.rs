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
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use tracing::{debug, info, warn};
use windows::core::HSTRING;
use windows::ApplicationModel::DataTransfer::SharedStorageAccessManager;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Storage::StorageFile;
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

/// ponytail: one photo toast at a time, so the tag is fixed rather than derived. That
/// bounds the staged file and the outstanding broker token at one each, which is the
/// property this project actually cares about; a burst of photos costs you the earlier
/// toasts. Give each photo a hashed tag and its own file if that ever matters.
const PHOTO_TAG: &str = "photo";

/// Snipping Tool's own protocol, read out of its binaries rather than guessed:
/// `ms-screensketch://edit/?source=…&isTemporary=…&sharedAccessToken=…`. `source=Toast`
/// is one of the values it ships with, which is the case this is.
///
/// `isTemporary=true` is the Win+Shift+S posture: the file is scratch, so Snipping Tool
/// opens it for markup and offers Save As rather than editing it in place. That is the
/// behaviour being copied, and it is also true — the staged file is overwritten by the
/// next photo.
fn snip_url(token: &str) -> String {
    format!(
        "ms-screensketch://edit/?source=Toast&isTemporary=true&sharedAccessToken={}",
        percent(token, b"")
    )
}

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

/// Percent-encodes everything outside the unreserved set, plus whatever [`safe`] keeps
/// literal.
///
/// Two jobs at once. A path with a space or a non-ASCII username survives, and nothing
/// is left in the result that an XML parser could read as markup — so an encoded value
/// dropped into an attribute needs no second escaping pass. Bytes, not chars, so
/// non-ASCII is encoded as the UTF-8 a URI is defined over.
fn percent(s: &str, safe: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) || safe.contains(&byte) {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// A `file:///` URL for a toast image's `src`. The separators and the drive colon are
/// the only path characters left literal.
fn file_url(path: &Path) -> String {
    format!(
        "file:///{}",
        percent(&path.to_string_lossy().replace('\\', "/"), b"/:")
    )
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
    /// A photo just taken on the phone, already staged at [`path`]. Shown with the
    /// picture in it; clicking hands the file to Snipping Tool.
    Photo {
        path: PathBuf,
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
    // The broker token for the photo currently on screen. At most one is ever
    // outstanding, because a new photo replaces the toast that named the old one — so
    // this is a slot, not a collection, and it cannot grow over a long run.
    let mut staged: Option<String> = None;
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
            Cmd::Photo { path } => show_photo(notifier, &path, &mut staged),
        };
        if let Err(e) = result {
            warn!(error = %e, "toast failed");
        }
    }
    // Hand the last one back on the way out, so a restarted daemon does not inherit a
    // token nobody will ever redeem.
    if let Some(token) = staged {
        release(&token);
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

/// The photo toast: the picture itself, and a click that opens it in Snipping Tool.
///
/// No bound data and no [`NotificationData`], because nothing ever updates a photo — a
/// newer one replaces it wholesale. `activationType="protocol"` is what keeps this cheap:
/// Windows resolves the URI itself, so there is no COM activator to register and no
/// callback for the daemon to stay alive for.
fn show_photo(notifier: &ToastNotifier, path: &Path, staged: &mut Option<String>) -> Result<()> {
    let token = share(path)?;
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(photo_xml(&token, path)))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(PHOTO_TAG))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    notifier.Show(&toast)?;
    // Only now is the previous one certainly off screen, so only now is its token dead.
    if let Some(old) = staged.replace(token) {
        release(&old);
    }
    info!(path = %path.display(), "photo toast shown");
    Ok(())
}

/// Split out from [`show_photo`] so the markup can be checked without a notifier, a
/// staged file or a broker token.
fn photo_xml(token: &str, path: &Path) -> String {
    format!(
        r#"<toast activationType="protocol" launch="{launch}">
             <visual>
               <binding template="ToastGeneric">
                 <image placement="hero" src="{hero}"/>
                 <text>New photo</text>
                 <text>Select to open it in Snipping Tool.</text>
                 <text placement="attribution">conduit</text>
               </binding>
             </visual>
           </toast>"#,
        // percent() already removed everything the parser could misread, so escape()
        // here is only turning the two literal query separators into entities.
        launch = escape(&snip_url(token)),
        hero = file_url(path),
    )
}

/// Registers the staged photo with the shared-storage broker and returns the token that
/// names it.
///
/// This is the whole reason a plain file path will not do. Snipping Tool is a packaged
/// app in a container; it cannot open an arbitrary path handed to it in a URI, so the
/// documented handoff is a token it redeems for the file. Minting one needs no package
/// identity of our own, which is the part that had to be established rather than assumed.
fn share(path: &Path) -> Result<String> {
    // The toast thread has an MTA of its own, but the tests call this from wherever the
    // harness puts them, and an implicit MTA is idempotent.
    crate::image::ensure_mta();
    let file = crate::image::block_on(StorageFile::GetFileFromPathAsync(&HSTRING::from(
        path.as_os_str(),
    ))?)
    .with_context(|| format!("opening {} as a StorageFile", path.display()))?;
    Ok(SharedStorageAccessManager::AddFile(&file)
        .context("adding the photo to shared storage")?
        .to_string())
}

/// Best effort. A token Snipping Tool already redeemed is gone, and being told so is not
/// a problem worth a warning.
fn release(token: &str) {
    match SharedStorageAccessManager::RemoveFile(&HSTRING::from(token)) {
        Ok(()) => debug!("returned a shared-file token"),
        Err(e) => debug!(error = %e, "shared-file token was already gone"),
    }
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

    #[test]
    fn percent_encoding_leaves_nothing_for_the_parser() {
        // Everything a URI or an XML attribute could choke on, encoded.
        assert_eq!(percent("a+b/c=d&e?f#g\"h'i<j>k", b""), "a%2Bb%2Fc%3Dd%26e%3Ff%23g%22h%27i%3Cj%3Ek");
        // Unreserved characters are left alone, so a token of them is untouched.
        assert_eq!(percent("Az09-._~", b""), "Az09-._~");
        // Bytes, not chars: non-ASCII becomes the UTF-8 a URI is defined over.
        assert_eq!(percent("é", b""), "%C3%A9");
    }

    #[test]
    fn file_urls_survive_spaces_and_non_ascii_paths() {
        assert_eq!(
            file_url(Path::new(r"C:\Users\a b\Conduit\photo.jpg")),
            "file:///C:/Users/a%20b/Conduit/photo.jpg"
        );
        // A non-ASCII user profile is the case that breaks a naive `format!`.
        assert_eq!(
            file_url(Path::new(r"C:\Users\用户\photo.png")),
            "file:///C:/Users/%E7%94%A8%E6%88%B7/photo.png"
        );
    }

    /// The load-bearing check on the photo path that needs no toast: the markup parses,
    /// and the launch URI the parser hands back is byte-for-byte the one we built. That
    /// is what proves percent-encoding and XML escaping compose instead of colliding —
    /// get it wrong and the click either does nothing or opens the wrong file.
    #[test]
    fn photo_markup_parses_and_returns_the_launch_uri_intact() -> Result<()> {
        crate::image::ensure_mta();
        // A token full of exactly the characters that would otherwise end the attribute.
        let token = "a+b/c=d&e";
        let path = Path::new(r"C:\Users\a b\Conduit\photo.jpg");

        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(photo_xml(token, path)))?;
        let root = xml.DocumentElement()?;

        assert_eq!(
            root.GetAttribute(&HSTRING::from("launch"))?.to_string(),
            snip_url(token),
            "the parser did not give back the URI we escaped"
        );
        assert_eq!(
            root.GetAttribute(&HSTRING::from("activationType"))?.to_string(),
            "protocol",
            "without this Windows looks for a COM activator we do not have"
        );
        // The token survived as an opaque, re-decodable value rather than as separators.
        assert!(snip_url(token).ends_with("sharedAccessToken=a%2Bb%2Fc%3Dd%26e"));
        Ok(())
    }

    /// A 32bpp `BI_RGB` DIB of one colour, so the ignored test below has a real image to
    /// stage without a fixture file. 16:9, because a hero image is letterboxed otherwise.
    #[cfg(test)]
    fn solid_dib(width: i32, height: i32, bgra: [u8; 4]) -> Vec<u8> {
        let mut dib = Vec::with_capacity(40 + (width * height * 4) as usize);
        for field in [40u32, width as u32, height as u32] {
            dib.extend_from_slice(&field.to_le_bytes());
        }
        dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        dib.extend_from_slice(&((width * height * 4) as u32).to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]); // resolution, biClrUsed, biClrImportant
        for _ in 0..(width * height) {
            dib.extend_from_slice(&bgra);
        }
        dib
    }

    /// Pops a real photo toast and leaves it on screen: `cargo test -p conduit-daemon
    /// -- --ignored photo_toast`.
    ///
    /// The assertions cover what code can check — that an unpackaged process really can
    /// mint a shared-storage token, and that the toast reached Action Center. The part
    /// only a human can confirm is printed: click it, and Snipping Tool should open the
    /// blue rectangle exactly as if it had been snipped with Win+Shift+S.
    #[test]
    #[ignore = "shows a real photo toast and waits for a human to click it"]
    fn a_photo_toast_opens_in_snipping_tool() -> Result<()> {
        crate::image::ensure_mta();
        let png = crate::image::dib_to_png(&solid_dib(640, 360, [224, 111, 46, 255]))?;
        let path = std::env::temp_dir().join("conduit-photo-test.png");
        std::fs::write(&path, &png)?;
        println!("staged {} ({} bytes)", path.display(), png.len());

        // The question this test exists to answer.
        let token = share(&path)?;
        assert!(!token.is_empty(), "an unpackaged process got no token");
        println!("token: {token}");
        println!("launch: {}", snip_url(&token));

        let notifier = Notifier::start()?;
        notifier.post(Cmd::Photo { path });
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert!(
            in_history(PHOTO_TAG)?,
            "the photo toast never reached Action Center"
        );
        println!("toast is up — click it now");
        std::thread::sleep(std::time::Duration::from_secs(20));
        Ok(())
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
