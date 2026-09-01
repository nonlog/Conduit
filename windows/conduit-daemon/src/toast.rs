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

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use windows::core::{Interface, HSTRING};
use windows::ApplicationModel::DataTransfer::SharedStorageAccessManager;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{IPropertyValue, TypedEventHandler};
use windows::Storage::StorageFile;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::UI::Notifications::{
    NotificationData, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};

use crate::clip;
use crate::verification_code;
use crate::wire::pb;

/// Must match the registry key below. Reverse-DNS-ish because that is the convention
/// Windows uses for AUMIDs and it keeps us out of anyone else's namespace.
const AUMID: &str = "Conduit.Desktop";
const APP_IDENTITY_ICON_LIGHT: &[u8] = include_bytes!("../assets/conduit-icon-light.png");
const APP_IDENTITY_ICON_DARK: &[u8] = include_bytes!("../assets/conduit-icon-dark.png");

/// Every toast shares one group, so a single call clears the lot on shutdown and the
/// phone's tag alone identifies a notification within it.
const GROUP: &str = "conduit";

/// ponytail: one phone-capture toast at a time, so the tag is fixed rather than derived. That
/// bounds the staged file and the outstanding broker token at one each, which is the
/// property this project actually cares about; a burst of photos/screenshots costs you the
/// earlier toast. The value stays `photo` for compatibility with an already-visible old toast.
const CAPTURE_TAG: &str = "photo";
/// Explorer/CLI permits one outbound file at a time, so one fixed tag is both sufficient and
/// desirable: every progress event updates the same Action Center row instead of creating a stack.
const TRANSFER_TAG: &str = "file-send";

/// How many contact avatars are kept on disk.
///
/// Only this directory needs a cap. App icons are keyed by package and overwritten in
/// place, so that directory is bounded by the number of apps that have ever notified;
/// avatars are keyed by content, so they grow with the number of distinct faces.
///
/// Eviction is almost free of consequence, which is why the policy is this crude: the
/// phone attaches a face to every notification it has one for, so a dropped file simply
/// reappears the next time that person writes.
const FACES_MAX: usize = 128;

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
    digest(key.as_bytes())
}

/// 64 bits of SHA-256, hex. Long enough to name a cache file by its contents without a
/// collision anyone will ever see, short enough to stay inside the tag limit above.
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8]
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
        /// Names the app-icon cache file; the phone sends the bytes only on first sight.
        package: String,
        app: String,
        title: String,
        body: String,
        /// Newest bounded Android MessagingStyle records, in chronological order.
        messages: Vec<pb::TextMessage>,
        /// Empty after the first notification from a package — the cached file stands in.
        app_icon: Vec<u8>,
        /// The contact photo, when the notification carried one. Empty otherwise.
        avatar: Vec<u8>,
        /// Bounded on Android to at most five descriptors. PendingIntents stay on Android;
        /// Windows gets only labels, indexes and a possible free-form reply key.
        actions: Vec<pb::NotifActionDesc>,
        /// An already-visible Android notification changed structural action metadata. Rebuild the
        /// same Windows tag silently rather than generating a second popup merely to refresh buttons.
        suppress_popup: bool,
    },
    /// Same key, new text. Silent — Windows does not re-alert on an update, which is the
    /// point: a chat thread gaining a message should not pop a second time.
    Update {
        key: String,
        title: String,
        body: String,
        messages: Vec<pb::TextMessage>,
    },
    Hide {
        key: String,
    },
    /// A photo just taken on the phone, already staged at [`path`]. Shown with the
    /// picture in it; clicking hands the file to Snipping Tool.
    Photo {
        path: PathBuf,
    },
    /// A screenshot just taken on the phone. Same bounded capture slot as a camera photo,
    /// but its user-facing semantics must say screenshot rather than photo.
    Screenshot {
        path: PathBuf,
    },
    /// A file the phone shared, already written to Downloads. Clicking opens the folder.
    File {
        path: PathBuf,
    },
    /// Outbound Explorer/CLI transfer feedback is owned by this already-resident COM thread.
    /// The helper process only streams the request/result and never has to initialize WinRT.
    TransferStart {
        name: String,
        total: u64,
    },
    TransferProgress {
        name: String,
        transferred: u64,
        total: u64,
        waiting_for_phone: bool,
    },
    TransferResult {
        name: String,
        success: bool,
    },
    /// A deliberate http/https page share from the phone. The shell handles protocol activation;
    /// no Chrome internals, profile database or extra resident callback is involved.
    SharedUrl {
        url: String,
        title: String,
        source: String,
    },
}

/// The on-disk icon cache.
///
/// A toast image has to be a file — `appLogoOverride` takes a URI, and there is no way to
/// inline bytes — so the phone's PNGs are written out and named in the XML. The directory
/// is the whole of the state: nothing is remembered in memory, so a restart inherits every
/// icon it had before and the two lookups are a `join` and an `exists`.
struct Cache {
    /// App icons, one file per package, overwritten in place. Bounded by the number of
    /// apps that have ever notified, which is why it needs no eviction.
    icons: PathBuf,
    /// Contact photos, named by their own bytes so one face is one file however many
    /// notifications carry it.
    faces: PathBuf,
}

impl Cache {
    fn prepare(root: &Path) -> Result<Self> {
        let cache = Self {
            icons: root.join("icons"),
            faces: root.join("faces"),
        };
        std::fs::create_dir_all(&cache.icons)?;
        std::fs::create_dir_all(&cache.faces)?;
        Ok(cache)
    }

    /// Stores whatever art arrived and returns the file the toast should show.
    ///
    /// The avatar wins when there is one: for a chat the person who wrote is the point,
    /// and the app is still named in a readable source-app line either way. The app icon is
    /// what everything without a person attached falls back to.
    fn logo(&self, package: &str, app_icon: &[u8], avatar: &[u8]) -> Result<Option<PathBuf>> {
        let icon = self
            .icons
            .join(format!("{}.png", digest(package.as_bytes())));
        // An empty `app_icon` on a later notification means "you already have it", not
        // "there is none" — the phone sends it once per package precisely so a day of chat
        // does not carry the same 8 kB PNG over a cellular relay a thousand times.
        if !app_icon.is_empty() {
            std::fs::write(&icon, app_icon)
                .with_context(|| format!("caching {}", icon.display()))?;
        }

        let face = if avatar.is_empty() {
            None
        } else {
            let path = self.faces.join(format!("{}.png", digest(avatar)));
            // Content-addressed, so a thread's hundredth message finds the file already
            // there. Skipping the rewrite is not just an optimisation: truncating a file
            // the shell may be rendering an on-screen toast from is how you get a blank
            // avatar in Action Center.
            if !path.exists() {
                self.evict()?;
                std::fs::write(&path, avatar)
                    .with_context(|| format!("caching {}", path.display()))?;
            }
            Some(path)
        };

        Ok(icon.is_file().then_some(icon).or(face))
    }

    /// Makes room for one more face.
    ///
    /// ponytail: a `read_dir` and a sort, but only when a face that has never been seen
    /// arrives — a chat with someone you already talk to costs an `exists` and nothing
    /// else. Keep an in-memory index if this ever needs to hold thousands.
    ///
    /// Losing a face is close to consequence-free, which is what lets the policy be this
    /// crude: the phone attaches one to every notification that has one, so an evicted
    /// file is written again the next time that person writes.
    fn evict(&self) -> Result<()> {
        let mut faces: Vec<_> = std::fs::read_dir(&self.faces)?.flatten().collect();
        if faces.len() < FACES_MAX {
            return Ok(());
        }
        faces.sort_by_key(|face| face.metadata().and_then(|m| m.modified()).ok());
        for face in &faces[..faces.len() - FACES_MAX + 1] {
            let _ = std::fs::remove_file(face.path());
        }
        debug!(kept = FACES_MAX - 1, "trimmed the face cache");
        Ok(())
    }
}

fn system_uses_light_theme() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .ok()
        .and_then(|key| key.get_u32("SystemUsesLightTheme").ok())
        .unwrap_or(1)
        != 0
}

fn ensure_app_identity_icon(root: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let light = system_uses_light_theme();
    let bytes = if light {
        APP_IDENTITY_ICON_LIGHT
    } else {
        APP_IDENTITY_ICON_DARK
    };
    // A theme-specific filename makes the AUMID IconUri itself change, which is more reliable than
    // asking Action Center to notice different bytes behind the same cached URI. The AUMID remains
    // one stable Conduit.Desktop identity.
    let path = root.join(if light {
        "conduit-icon-light.png"
    } else {
        "conduit-icon-dark.png"
    });
    let matches = std::fs::read(&path)
        .map(|current| current.as_slice() == bytes)
        .unwrap_or(false);
    if !matches {
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(path)
}

/// Rebinds the existing AUMID to the light/dark identity icon selected by Windows. Theme-change
/// notification is borrowed from the tray's existing message loop, so this adds no background poll.
pub fn refresh_app_identity(root: &Path) -> Result<()> {
    let identity_icon = ensure_app_identity_icon(root)?;
    register_aumid(&identity_icon)
}

pub struct Notifier {
    /// `Option` so [`Drop`] can close the channel before joining; the closed channel is
    /// what makes the thread's `recv` return and the join finish.
    tx: Option<mpsc::Sender<Cmd>>,
    thread: Option<JoinHandle<()>>,
}

impl Notifier {
    /// `cache` is where the phone's icons are written; a toast image can only be named by
    /// URI, so they have to land somewhere the shell can read them.
    pub fn start(
        cache: &Path,
        action_tx: broadcast::Sender<pb::NotifAction>,
        clipboard: Arc<clip::Bridge>,
    ) -> Result<Self> {
        let identity_icon = ensure_app_identity_icon(cache)?;
        register_aumid(&identity_icon).context("registering the toast AppUserModelID")?;
        let cache = Cache::prepare(cache).context("preparing the toast icon cache")?;

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
                            pump(&notifier, &cache, rx, action_tx, clipboard);
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

#[derive(Clone)]
struct ToastShape {
    app: String,
    logo: Option<PathBuf>,
    actions: Vec<pb::NotifActionDesc>,
    code: Option<String>,
}

fn pump(
    notifier: &ToastNotifier,
    cache: &Cache,
    rx: mpsc::Receiver<Cmd>,
    action_tx: broadcast::Sender<pb::NotifAction>,
    clipboard: Arc<clip::Bridge>,
) {
    // The broker token for the phone capture currently on screen. At most one is ever
    // outstanding, because a new capture replaces the toast that named the old one — so
    // this is a slot, not a collection, and it cannot grow over a long run.
    let mut staged: Option<String> = None;
    // A ToastNotification owns its Activated delegate. Keep only currently mirrored toasts alive,
    // with the same hard ceiling Android already uses for remembered notification keys.
    let mut actionable: HashMap<String, ToastNotification> = HashMap::new();
    // Shape data is needed only when an update adds/removes a verification code, because toast
    // actions are XML and cannot be changed through NotificationData. Bounded like actionable.
    let mut shapes: HashMap<String, ToastShape> = HashMap::new();
    while let Ok(cmd) = rx.recv() {
        let result = match cmd {
            Cmd::Show {
                key,
                package,
                app,
                title,
                body,
                messages,
                app_icon,
                avatar,
                actions,
                suppress_popup,
            } => {
                // A cache write that fails costs the toast its picture, nothing more, so
                // the logo is resolved leniently and the toast still shows.
                let logo = cache
                    .logo(&package, &app_icon, &avatar)
                    .unwrap_or_else(|e| {
                        warn!(error = %e, "could not cache the notification icon");
                        None
                    });
                let code = verification_code::extract(&title, &body, &messages);
                match show(
                    notifier,
                    &key,
                    &app,
                    &title,
                    &body,
                    &messages,
                    logo.as_deref(),
                    &actions,
                    code.as_deref(),
                    suppress_popup,
                    &action_tx,
                    &clipboard,
                ) {
                    Ok(toast) => {
                        if shapes.len() >= 256 && !shapes.contains_key(&key) {
                            if let Some(evict) = shapes.keys().next().cloned() {
                                shapes.remove(&evict);
                                actionable.remove(&evict);
                            }
                        }
                        shapes.insert(
                            key.clone(),
                            ToastShape {
                                app,
                                logo,
                                actions,
                                code,
                            },
                        );
                        if let Some(toast) = toast {
                            actionable.insert(key, toast);
                        } else {
                            actionable.remove(&key);
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Cmd::Update {
                key,
                title,
                body,
                messages,
            } => {
                let code = verification_code::extract(&title, &body, &messages);
                let shape = shapes.get(&key).cloned();
                if let Some(shape) = shape {
                    if shape.code != code {
                        match show(
                            notifier,
                            &key,
                            &shape.app,
                            &title,
                            &body,
                            &messages,
                            shape.logo.as_deref(),
                            &shape.actions,
                            code.as_deref(),
                            true,
                            &action_tx,
                            &clipboard,
                        ) {
                            Ok(toast) => {
                                if let Some(shape) = shapes.get_mut(&key) {
                                    shape.code = code;
                                }
                                if let Some(toast) = toast {
                                    actionable.insert(key, toast);
                                } else {
                                    actionable.remove(&key);
                                }
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    } else {
                        update(notifier, &key, &title, &body, &messages)
                    }
                } else {
                    update(notifier, &key, &title, &body, &messages)
                }
            }
            Cmd::Hide { key } => {
                actionable.remove(&key);
                shapes.remove(&key);
                hide(&key)
            }
            Cmd::Photo { path } => show_capture(notifier, &path, &mut staged, false),
            Cmd::Screenshot { path } => show_capture(notifier, &path, &mut staged, true),
            Cmd::File { path } => show_file(notifier, &path),
            Cmd::TransferStart { name, total } => show_transfer(notifier, &name, total),
            Cmd::TransferProgress {
                name,
                transferred,
                total,
                waiting_for_phone,
            } => update_transfer(notifier, &name, transferred, total, waiting_for_phone),
            Cmd::TransferResult { name, success } => {
                update_transfer_result(notifier, &name, success)
            }
            Cmd::SharedUrl { url, title, source } => {
                show_shared_url(notifier, &url, &title, &source)
            }
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
fn show(
    notifier: &ToastNotifier,
    key: &str,
    app: &str,
    title: &str,
    body: &str,
    messages: &[pb::TextMessage],
    logo: Option<&Path>,
    actions: &[pb::NotifActionDesc],
    copy_code: Option<&str>,
    suppress_popup: bool,
    action_tx: &broadcast::Sender<pb::NotifAction>,
    clipboard: &Arc<clip::Bridge>,
) -> Result<Option<ToastNotification>> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(show_xml(app, logo, actions, copy_code)))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let tag = tag_for(key);
    toast.SetTag(&HSTRING::from(&tag))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    toast.SetData(&data(title, body, messages)?)?;
    toast.SetSuppressPopup(suppress_popup)?;
    let has_copy_code = copy_code.is_some();
    if !actions.is_empty() || has_copy_code {
        let key = key.to_owned();
        let actions = actions.to_vec();
        let expected_copy_code = copy_code.map(str::to_owned);
        let action_tx = action_tx.clone();
        let clipboard = clipboard.clone();
        let handler = TypedEventHandler::<ToastNotification, windows::core::IInspectable>::new(
            move |_sender, args| {
                let Some(args) = &*args else { return Ok(()) };
                let args: ToastActivatedEventArgs = args.cast()?;
                let arguments = args.Arguments()?.to_string();
                if let Some(code) = parse_copy_code(&arguments) {
                    // Only the exact code embedded in this toast is accepted. Never log the value.
                    if expected_copy_code.as_deref() == Some(code) {
                        if let Err(e) = clipboard.apply(code) {
                            warn!(error = %e, "could not copy verification code");
                        }
                    }
                    return Ok(());
                }
                let Some(index) = parse_action_index(&arguments) else {
                    return Ok(());
                };
                let Some(desc) = actions.iter().find(|action| action.index == index) else {
                    return Ok(());
                };
                let reply = if desc.has_remote_input {
                    let input = args.UserInput()?;
                    if input.HasKey(&HSTRING::from("reply"))? {
                        input
                            .Lookup(&HSTRING::from("reply"))?
                            .cast::<IPropertyValue>()?
                            .GetString()?
                            .to_string()
                            .trim_end_matches(['\r', '\n'])
                            .to_owned()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let action = pb::NotifAction {
                    key: key.clone(),
                    action_index: index,
                    reply_text: reply,
                    action_label: desc.label.clone(),
                    result_key: desc.result_key.clone(),
                };
                // No active Noise session means no broadcast receiver; send then fails and the
                // click is discarded instead of being queued for a later, potentially stale session.
                let _ = action_tx.send(action);
                Ok(())
            },
        );
        toast.Activated(&handler)?;
    }
    notifier.Show(&toast)?;
    debug!(%tag, logo = logo.is_some(), suppress_popup, "toast shown");
    Ok((!actions.is_empty() || has_copy_code).then_some(toast))
}

/// Split out so the markup can be checked without a notifier, the same as [`photo_xml`].
///
/// The image is inlined rather than bound, because only text and progress values can be
/// bound — which also settles what [`update`] can do: a later message in the same thread
/// rewrites the text and keeps the picture the first one arrived with. Re-showing to change
/// a face would pop and re-alert, which is the thing update exists to avoid.
fn show_xml(
    app: &str,
    logo: Option<&Path>,
    actions: &[pb::NotifActionDesc],
    copy_code: Option<&str>,
) -> String {
    let image = match logo {
        // Circle-cropped because that is what the slot is for, and because an Android
        // adaptive icon has already been drawn through the platform's own round mask.
        Some(path) => format!(
            r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
            file_url(path)
        ),
        None => String::new(),
    };
    let actions = action_xml(actions, copy_code);
    format!(
        r#"<toast>
             <visual>
               <binding template="ToastGeneric">
                 {image}
                 <text hint-style="captionSubtle">{}</text>
                 <text hint-style="title">{{title}}</text>
                 <text>{{body}}</text>
               </binding>
             </visual>
             {actions}
           </toast>"#,
        escape(app)
    )
}

fn action_xml(actions: &[pb::NotifActionDesc], copy_code: Option<&str>) -> String {
    if actions.is_empty() && copy_code.is_none() {
        return String::new();
    }
    let remote_position = actions.iter().position(|action| action.has_remote_input);
    let mut xml = String::from("<actions>");
    if remote_position.is_some() {
        xml.push_str(r#"<input id="reply" type="text" placeHolderContent="Reply"/>"#);
    }
    // ToastGeneric supports at most five action buttons. Reserve one for Copy when a code exists.
    let action_budget = if copy_code.is_some() { 4 } else { 5 };
    let mut added = 0usize;
    for (position, action) in actions.iter().enumerate() {
        if action.label.is_empty() || added >= action_budget {
            continue;
        }
        if action.has_remote_input && Some(position) != remote_position {
            continue;
        }
        let input = if action.has_remote_input {
            r#" hint-inputId="reply""#
        } else {
            ""
        };
        xml.push_str(&format!(
            r#"<action content="{}" arguments="action={}" activationType="foreground"{input}/>"#,
            escape(&action.label),
            action.index,
        ));
        added += 1;
    }
    if let Some(code) = copy_code {
        xml.push_str(&format!(
            r#"<action content="Copy" arguments="copy={}" activationType="foreground"/>"#,
            escape(code),
        ));
    }
    xml.push_str("</actions>");
    xml
}

fn parse_action_index(arguments: &str) -> Option<u32> {
    arguments.strip_prefix("action=")?.parse().ok()
}

fn parse_copy_code(arguments: &str) -> Option<&str> {
    let code = arguments.strip_prefix("copy=")?;
    (code.len() >= 4 && code.len() <= 8 && code.bytes().all(|b| b.is_ascii_digit())).then_some(code)
}

/// A phone-capture toast: the picture itself, and a click that opens it in Snipping Tool.
///
/// No bound data and no [`NotificationData`], because nothing ever updates a capture — a
/// newer one replaces it wholesale. `activationType="protocol"` is what keeps this cheap:
/// Windows resolves the URI itself, so there is no COM activator to register and no
/// callback for the daemon to stay alive for.
fn show_capture(
    notifier: &ToastNotifier,
    path: &Path,
    staged: &mut Option<String>,
    screenshot: bool,
) -> Result<()> {
    let token = share(path)?;
    let xml = XmlDocument::new()?;
    let markup = if screenshot {
        screenshot_xml(&token, path)
    } else {
        photo_xml(&token, path)
    };
    xml.LoadXml(&HSTRING::from(markup))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(CAPTURE_TAG))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    notifier.Show(&toast)?;
    // Only now is the previous one certainly off screen, so only now is its token dead.
    if let Some(old) = staged.replace(token) {
        release(&old);
    }
    info!(path = %path.display(), screenshot, "capture toast shown");
    Ok(())
}

/// Split out from [`show_capture`] so the markup can be checked without a notifier, a
/// staged file or a broker token.
fn photo_xml(token: &str, path: &Path) -> String {
    capture_xml(
        token,
        path,
        "New photo",
        "Select to open it in Snipping Tool.",
    )
}

fn screenshot_xml(token: &str, path: &Path) -> String {
    capture_xml(
        token,
        path,
        "New screenshot",
        "Select to open it in Snipping Tool.",
    )
}

fn capture_xml(token: &str, path: &Path, title: &str, body: &str) -> String {
    format!(
        r#"<toast activationType="protocol" launch="{launch}">
             <visual>
               <binding template="ToastGeneric">
                 <image placement="hero" src="{hero}"/>
                 <text>{title}</text>
                 <text>{body}</text>
                 <text placement="attribution">Conduit</text>
               </binding>
             </visual>
           </toast>"#,
        // percent() already removed everything the parser could misread, so escape()
        // here is only turning the two literal query separators into entities.
        launch = escape(&snip_url(token)),
        hero = file_url(path),
        title = escape(title),
        body = escape(body),
    )
}

/// Registers the staged capture with the shared-storage broker and returns the token that
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
        .context("adding the capture to shared storage")?
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

/// A deliberate web-page share. This intentionally belongs to Conduit, not Chrome's private
/// Send Tab to Self sync component. Protocol activation hands the http/https URL to the user's
/// configured browser without another resident callback or polling path.
fn safe_web_url(url: &str) -> bool {
    if url.len() > 4096 || url.chars().any(char::is_control) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn shared_url_xml(url: &str, title: &str, source: &str) -> Result<String> {
    if !safe_web_url(url) {
        bail!("shared URL is not a bounded http/https URL");
    }
    let url = escape(url);
    let source = escape(if source.trim().is_empty() {
        "phone"
    } else {
        source.trim()
    });
    let title = escape(title.trim());
    let detail = if title.is_empty() {
        format!(r#"<text>{url}</text>"#)
    } else {
        format!(r#"<text>{title}</text><text>{url}</text>"#)
    };
    Ok(format!(
        r#"<toast launch="{url}" activationType="protocol">
             <visual><binding template="ToastGeneric">
               <text>Page shared from {source}</text>
               {detail}
             </binding></visual>
             <actions>
               <action content="Open in New Tab" arguments="{url}" activationType="protocol"/>
             </actions>
           </toast>"#
    ))
}

fn show_shared_url(notifier: &ToastNotifier, url: &str, title: &str, source: &str) -> Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(shared_url_xml(url, title, source)?))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(tag_for(url)))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    notifier.Show(&toast)?;
    info!(%url, source, "shared URL toast shown");
    Ok(())
}

/// The file toast: what arrived, and a click that opens the folder it landed in.
///
/// Tagged by path rather than sharing one slot like the photo toast, because several files
/// can arrive in a row and each is a separate thing the user may want to go and find.
fn show_file(notifier: &ToastNotifier, path: &Path) -> Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(file_xml(path)))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let tag = digest(path.to_string_lossy().as_bytes());
    toast.SetTag(&HSTRING::from(&tag))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    notifier.Show(&toast)?;
    info!(path = %path.display(), "file toast shown");
    Ok(())
}

/// Split out from [`show_file`] so the markup can be checked without a notifier.
///
/// Opens the *folder*, not the file. A received file's name and extension are chosen by
/// the peer, and a toast whose click runs whatever arrived is a worse idea than one that
/// shows the user where it is and lets them decide.
fn file_xml(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = path.parent().unwrap_or(Path::new("."));
    format!(
        r#"<toast activationType="protocol" launch="{launch}">
             <visual>
               <binding template="ToastGeneric">
                 <text>File received</text>
                 <text>{name}</text>
                 <text placement="attribution">Conduit</text>
               </binding>
             </visual>
           </toast>"#,
        launch = escape(&file_url(folder)),
        name = escape(&name),
    )
}

fn transfer_xml() -> &'static str {
    r#"<toast duration="long">
         <visual>
           <binding template="ToastGeneric">
             <text>Send with Conduit</text>
             <text>{transferName}</text>
             <progress title="{transferTitle}"
                       value="{transferValue}"
                       valueStringOverride="{transferPercent}"
                       status="{transferStatus}"/>
           </binding>
         </visual>
       </toast>"#
}

fn transfer_data(
    name: &str,
    transferred: u64,
    total: u64,
    title: &str,
    status: &str,
) -> Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    let total = total.max(1);
    let transferred = transferred.min(total);
    let percent = transferred.saturating_mul(100) / total;
    values.Insert(&HSTRING::from("transferName"), &HSTRING::from(name))?;
    values.Insert(&HSTRING::from("transferTitle"), &HSTRING::from(title))?;
    values.Insert(
        &HSTRING::from("transferValue"),
        &HSTRING::from(format!("{:.4}", transferred as f64 / total as f64)),
    )?;
    values.Insert(
        &HSTRING::from("transferPercent"),
        &HSTRING::from(format!("{percent}%")),
    )?;
    values.Insert(&HSTRING::from("transferStatus"), &HSTRING::from(status))?;
    data.SetSequenceNumber(0)?;
    Ok(data)
}

fn transfer_result_data(name: &str, success: bool) -> Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    values.Insert(&HSTRING::from("transferName"), &HSTRING::from(name))?;
    values.Insert(
        &HSTRING::from("transferTitle"),
        &HSTRING::from(if success {
            "Sent to phone"
        } else {
            "Couldn’t send to phone"
        }),
    )?;
    values.Insert(&HSTRING::from("transferValue"), &HSTRING::from("1"))?;
    values.Insert(
        &HSTRING::from("transferPercent"),
        &HSTRING::from(if success { "100%" } else { "Failed" }),
    )?;
    values.Insert(
        &HSTRING::from("transferStatus"),
        &HSTRING::from(if success { "Complete" } else { "Failed" }),
    )?;
    data.SetSequenceNumber(0)?;
    Ok(data)
}

fn show_transfer(notifier: &ToastNotifier, name: &str, total: u64) -> Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(transfer_xml()))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(TRANSFER_TAG))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    toast.SetData(&transfer_data(
        name,
        0,
        total,
        "Sending to phone",
        "Preparing",
    )?)?;
    toast.SetSuppressPopup(false)?;
    notifier.Show(&toast)?;
    debug!(name, total, "outbound transfer toast shown");
    Ok(())
}

fn update_transfer(
    notifier: &ToastNotifier,
    name: &str,
    transferred: u64,
    total: u64,
    waiting_for_phone: bool,
) -> Result<()> {
    let status = if waiting_for_phone {
        "Waiting for phone"
    } else {
        "Sending"
    };
    let outcome = notifier.UpdateWithTagAndGroup(
        &transfer_data(name, transferred, total, "Sending to phone", status)?,
        &HSTRING::from(TRANSFER_TAG),
        &HSTRING::from(GROUP),
    )?;
    debug!(
        name,
        transferred,
        total,
        ?outcome,
        "outbound transfer toast updated"
    );
    Ok(())
}

fn update_transfer_result(notifier: &ToastNotifier, name: &str, success: bool) -> Result<()> {
    let outcome = notifier.UpdateWithTagAndGroup(
        &transfer_result_data(name, success)?,
        &HSTRING::from(TRANSFER_TAG),
        &HSTRING::from(GROUP),
    )?;
    debug!(name, success, ?outcome, "outbound transfer toast completed");
    Ok(())
}

fn update(
    notifier: &ToastNotifier,
    key: &str,
    title: &str,
    body: &str,
    messages: &[pb::TextMessage],
) -> Result<()> {
    let tag = tag_for(key);
    // NotificationUpdateResult::Failed comes back when the toast has already aged out of
    // Action Center. Nothing to do about it and nothing wrong, so it is not an error.
    let outcome = notifier.UpdateWithTagAndGroup(
        &data(title, body, messages)?,
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
fn data(title: &str, body: &str, messages: &[pb::TextMessage]) -> Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    values.Insert(&HSTRING::from("title"), &HSTRING::from(title))?;
    values.Insert(
        &HSTRING::from("body"),
        &HSTRING::from(conversation_body(body, messages)),
    )?;
    data.SetSequenceNumber(0)?;
    Ok(data)
}

/// Uses Android's bounded MessagingStyle history when it adds context; otherwise preserves the
/// ordinary notification body exactly. Android already caps the list and each field before it
/// reaches the wire, so this is formatting only and allocates at most a few short lines per event.
fn conversation_body(body: &str, messages: &[pb::TextMessage]) -> String {
    if messages.is_empty() || (messages.len() == 1 && !body.is_empty()) {
        return body.to_owned();
    }
    let rendered = messages
        .iter()
        .filter(|message| !message.text.is_empty())
        .map(|message| {
            if message.sender.is_empty() {
                message.text.clone()
            } else {
                format!("{}: {}", message.sender, message.text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if rendered.is_empty() {
        body.to_owned()
    } else {
        rendered
    }
}

/// Idempotent. Without this the notifier is created but every `Show` fails, because
/// Windows cannot resolve the AUMID to anything it is willing to attribute a toast to.
fn register_aumid(identity_icon: &Path) -> Result<()> {
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{AUMID}"))?;
    key.set_string("DisplayName", "Conduit")?;
    // Unpackaged Win32 notifications get their Action Center identity from this AUMID
    // registry entry. Without IconUri, Windows falls back to the generic window glyph.
    key.set_string("IconUri", identity_icon.to_string_lossy().as_ref())?;
    key.set_string("IconBackgroundColor", "00000000")?;
    // On, so Conduit gets its own entry in Settings -> Notifications and the toasts can
    // be muted there like any other app's rather than only by killing the daemon.
    key.set_u32("ShowInActionCenter", 1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_transfer_markup_parses_and_uses_one_stable_tag() -> Result<()> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(transfer_xml()))?;
        let markup = xml.GetXml()?.to_string();
        assert!(markup.contains("Send with Conduit"));
        assert!(markup.contains("{transferValue}"));
        assert_eq!(TRANSFER_TAG, "file-send");
        Ok(())
    }

    #[test]
    fn outbound_transfer_data_bounds_percent_at_one_hundred() -> Result<()> {
        let data = transfer_data("x.bin", 2048, 1024, "Sending to phone", "Sending")?;
        let values = data.Values()?;
        assert_eq!(
            values
                .Lookup(&HSTRING::from("transferPercent"))?
                .to_string(),
            "100%"
        );
        assert_eq!(
            values.Lookup(&HSTRING::from("transferValue"))?.to_string(),
            "1.0000"
        );
        Ok(())
    }

    #[test]
    fn tags_fit_the_platform_limit_and_stay_stable() {
        let key = "0|com.tencent.mm|1234567|null|10123";
        let tag = tag_for(key);
        assert_eq!(tag.len(), 16, "16 hex chars, far inside the 64-char cap");
        assert_eq!(
            tag,
            tag_for(key),
            "derived, so update and removal find it again"
        );
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
        assert_eq!(
            percent("a+b/c=d&e?f#g\"h'i<j>k", b""),
            "a%2Bb%2Fc%3Dd%26e%3Ff%23g%22h%27i%3Cj%3Ek"
        );
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

    /// A file name is peer-chosen text going straight into the markup, so the two things
    /// worth checking are that it cannot break out of it and that the click still points at
    /// the folder rather than at the file.
    #[test]
    fn file_markup_parses_and_points_at_the_folder() -> Result<()> {
        crate::image::ensure_mta();
        let path = Path::new(r"C:\Users\a b\Downloads\quarterly & final <report>.pdf");

        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(file_xml(path)))?;
        let root = xml.DocumentElement()?;

        assert_eq!(
            root.GetAttribute(&HSTRING::from("launch"))?.to_string(),
            file_url(Path::new(r"C:\Users\a b\Downloads")),
            "the click would not open the folder the file landed in"
        );
        let texts = xml.GetElementsByTagName(&HSTRING::from("text"))?;
        assert_eq!(
            texts.GetAt(1)?.InnerText()?.to_string(),
            "quarterly & final <report>.pdf",
            "the parser did not give back the name we escaped"
        );
        Ok(())
    }

    /// A directory of this test's own, wiped first so a previous run cannot be mistaken
    /// for cached state — which is exactly what these tests are about.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("conduit-cache-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn count(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    }

    /// The contract the whole icon feature rests on: the phone sends an app icon *once*
    /// per package, so a later notification arriving with no bytes must still find the
    /// picture. Get this wrong and every notification but the first is bare.
    #[test]
    fn an_app_icon_sent_once_is_found_again_with_no_bytes() -> Result<()> {
        let cache = Cache::prepare(&scratch("once"))?;

        let first = cache.logo("com.tencent.mm", b"icon-bytes", b"")?;
        assert!(first.is_some(), "the first notification cached nothing");
        // The next hundred notifications from this app carry no icon at all.
        let later = cache.logo("com.tencent.mm", b"", b"")?;
        assert_eq!(later, first, "the cached icon was not reused");
        assert_eq!(std::fs::read(later.unwrap())?, b"icon-bytes");

        // A package that has never sent one gets no picture rather than someone else's.
        assert_eq!(cache.logo("org.other.app", b"", b"")?, None);
        Ok(())
    }

    #[test]
    fn app_icon_stays_the_source_identity_while_faces_are_cached_as_fallback() -> Result<()> {
        let dir = scratch("faces");
        let cache = Cache::prepare(&dir)?;
        let app = cache.logo("im.app", b"app-icon", b"")?.unwrap();

        let with_face = cache.logo("im.app", b"", b"alice-photo")?.unwrap();
        assert_eq!(
            with_face, app,
            "a contact face replaced the source app icon"
        );
        assert_eq!(std::fs::read(&with_face)?, b"app-icon");
        // Faces remain content-addressed and bounded for the case where an app icon is unavailable.
        cache.logo("face.only", b"", b"alice-photo")?;
        cache.logo("face.only", b"", b"alice-photo")?;
        cache.logo("face.only", b"", b"bob-photo")?;
        assert_eq!(count(&dir.join("faces")), 2);
        assert_eq!(
            std::fs::read(cache.logo("face.only", b"", b"alice-photo")?.unwrap())?,
            b"alice-photo"
        );
        Ok(())
    }

    #[test]
    fn the_face_cache_stops_growing() -> Result<()> {
        let dir = scratch("evict");
        let cache = Cache::prepare(&dir)?;
        // Well past the cap, every face distinct so none of them dedupe.
        for i in 0..(FACES_MAX * 2) {
            cache.logo("im.app", b"", format!("face-{i}").as_bytes())?;
        }
        assert!(
            count(&dir.join("faces")) <= FACES_MAX,
            "the cache grew past its cap"
        );
        Ok(())
    }

    /// The image is inlined into the markup rather than bound, so a broken path or a
    /// stray character here is a toast that renders without its icon and says nothing
    /// about why. Both shapes have to parse.
    #[test]
    fn the_logo_is_named_in_markup_that_parses() -> Result<()> {
        crate::image::ensure_mta();
        let logo = Path::new(r"C:\Users\a b\Conduit\icons\deadbeef.png");

        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(show_xml("WeChat", Some(logo), &[], None)))?;
        let images = xml.GetElementsByTagName(&HSTRING::from("image"))?;
        assert_eq!(images.Length()?, 1, "the toast has no logo element");
        let image = images.GetAt(0)?;
        assert_eq!(
            image
                .Attributes()?
                .GetNamedItem(&HSTRING::from("src"))?
                .InnerText()?
                .to_string(),
            file_url(logo),
            "the parser did not give back the path we escaped"
        );
        assert_eq!(
            image
                .Attributes()?
                .GetNamedItem(&HSTRING::from("placement"))?
                .InnerText()?
                .to_string(),
            "appLogoOverride"
        );

        // No icon yet: still a valid toast, just a bare one.
        let plain = XmlDocument::new()?;
        plain.LoadXml(&HSTRING::from(show_xml("WeChat", None, &[], None)))?;
        assert_eq!(
            plain
                .GetElementsByTagName(&HSTRING::from("image"))?
                .Length()?,
            0
        );
        Ok(())
    }

    #[test]
    fn source_app_name_is_the_first_readable_line_above_the_notification_title() -> Result<()> {
        crate::image::ensure_mta();
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(show_xml("ChatGPT", None, &[], None)))?;
        let texts = xml.GetElementsByTagName(&HSTRING::from("text"))?;
        assert_eq!(texts.Length()?, 3);
        let source = texts.GetAt(0)?;
        assert_eq!(source.InnerText()?.to_string(), "ChatGPT");
        let attributes = source.Attributes()?;
        assert_eq!(
            attributes
                .GetNamedItem(&HSTRING::from("hint-style"))?
                .InnerText()?
                .to_string(),
            "captionSubtle"
        );
        assert!(
            attributes
                .GetNamedItem(&HSTRING::from("placement"))
                .is_err(),
            "source app fell back to Windows' tiny attribution line"
        );
        assert_eq!(texts.GetAt(1)?.InnerText()?.to_string(), "{title}");
        Ok(())
    }

    #[test]
    fn notification_actions_parse_escape_and_expose_only_one_reply_box() -> Result<()> {
        crate::image::ensure_mta();
        let actions = vec![
            pb::NotifActionDesc {
                label: "Reply & send".into(),
                index: 3,
                has_remote_input: true,
                result_key: "message".into(),
            },
            pb::NotifActionDesc {
                label: "Second reply".into(),
                index: 4,
                has_remote_input: true,
                result_key: "other".into(),
            },
            pb::NotifActionDesc {
                label: "Mark <read>".into(),
                index: 7,
                has_remote_input: false,
                result_key: String::new(),
            },
        ];
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(show_xml("Chat", None, &actions, None)))?;
        assert_eq!(
            xml.GetElementsByTagName(&HSTRING::from("input"))?
                .Length()?,
            1
        );
        let buttons = xml.GetElementsByTagName(&HSTRING::from("action"))?;
        assert_eq!(
            buttons.Length()?,
            2,
            "later RemoteInput action must be omitted"
        );
        assert!(show_xml("Chat", None, &actions, None).contains("Reply &amp; send"));
        assert!(show_xml("Chat", None, &actions, None).contains("Mark &lt;read&gt;"));
        assert_eq!(parse_action_index("action=7"), Some(7));
        assert_eq!(parse_action_index("action=nope"), None);
        assert_eq!(parse_action_index("launch=7"), None);
        Ok(())
    }

    #[test]
    fn verification_code_adds_a_local_copy_button_without_breaking_android_actions() -> Result<()> {
        crate::image::ensure_mta();
        let actions = vec![pb::NotifActionDesc {
            label: "Reply".into(),
            index: 3,
            has_remote_input: true,
            result_key: "message".into(),
        }];
        let markup = show_xml("Messages", None, &actions, Some("482731"));
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(&markup))?;
        assert_eq!(
            xml.GetElementsByTagName(&HSTRING::from("action"))?
                .Length()?,
            2
        );
        assert!(markup.contains(r#"content="Copy" arguments="copy=482731""#));
        assert_eq!(parse_copy_code("copy=482731"), Some("482731"));
        assert_eq!(parse_copy_code("copy=123"), None);
        assert_eq!(parse_copy_code("copy=123456789"), None);
        assert_eq!(parse_copy_code("copy=12ab56"), None);
        Ok(())
    }

    #[test]
    fn verification_code_reserves_one_of_five_toast_buttons_for_copy() -> Result<()> {
        crate::image::ensure_mta();
        let actions = (0..5)
            .map(|index| pb::NotifActionDesc {
                label: format!("A{index}"),
                index,
                has_remote_input: false,
                result_key: String::new(),
            })
            .collect::<Vec<_>>();
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(show_xml(
            "Messages",
            None,
            &actions,
            Some("123456"),
        )))?;
        assert_eq!(
            xml.GetElementsByTagName(&HSTRING::from("action"))?
                .Length()?,
            5
        );
        Ok(())
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
            root.GetAttribute(&HSTRING::from("activationType"))?
                .to_string(),
            "protocol",
            "without this Windows looks for a COM activator we do not have"
        );
        // The token survived as an opaque, re-decodable value rather than as separators.
        assert!(snip_url(token).ends_with("sharedAccessToken=a%2Bb%2Fc%3Dd%26e"));
        Ok(())
    }

    #[test]
    fn screenshot_markup_keeps_capture_semantics_and_the_same_snipping_handoff() -> Result<()> {
        crate::image::ensure_mta();
        let token = "capture-token";
        let path = Path::new(r"C:\Users\a b\Conduit\capture.png");
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(screenshot_xml(token, path)))?;
        let root = xml.DocumentElement()?;

        assert_eq!(
            root.GetAttribute(&HSTRING::from("launch"))?.to_string(),
            snip_url(token)
        );
        let texts = xml.GetElementsByTagName(&HSTRING::from("text"))?;
        assert_eq!(texts.GetAt(0)?.InnerText()?.to_string(), "New screenshot");
        assert_eq!(texts.GetAt(2)?.InnerText()?.to_string(), "Conduit");
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

        let (action_tx, _) = broadcast::channel(4);
        let clipboard = Arc::new(clip::Bridge::start()?);
        let notifier = Notifier::start(&scratch("photo"), action_tx, clipboard)?;
        notifier.post(Cmd::Photo { path });
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert!(
            in_history(CAPTURE_TAG)?,
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
        let (action_tx, _) = broadcast::channel(4);
        let clipboard = Arc::new(clip::Bridge::start()?);
        let notifier = Notifier::start(&scratch("live"), action_tx, clipboard)?;
        // A real 96 px square, the size the phone sends, so the logo slot is exercised by
        // something the shell will actually decode rather than a path to nothing.
        crate::image::ensure_mta();
        let icon = crate::image::dib_to_png(&solid_dib(96, 96, [64, 64, 220, 255]))?;

        notifier.post(Cmd::Show {
            key: key.into(),
            package: "com.conduit.test".into(),
            app: "conduit self-test".into(),
            title: "First title".into(),
            body: "If this reads First title, data binding renders.".into(),
            messages: Vec::new(),
            app_icon: icon,
            avatar: Vec::new(),
            actions: Vec::new(),
            suppress_popup: false,
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(in_history(&tag)?, "Show did not reach Action Center");

        notifier.post(Cmd::Update {
            key: key.into(),
            title: "Second title".into(),
            body: "Updated in place, and it must not have popped again.".into(),
            messages: Vec::new(),
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

    #[test]
    fn messaging_history_replaces_only_the_body_and_keeps_sender_context() {
        let messages = vec![
            pb::TextMessage {
                sender: "Alice".into(),
                text: "first".into(),
            },
            pb::TextMessage {
                sender: "Bob".into(),
                text: "second".into(),
            },
            pb::TextMessage {
                sender: String::new(),
                text: "third".into(),
            },
        ];
        assert_eq!(
            conversation_body("latest fallback", &messages),
            "Alice: first\nBob: second\nthird"
        );
        assert_eq!(conversation_body("ordinary body", &[]), "ordinary body");
        assert_eq!(
            conversation_body(
                "single latest",
                &[pb::TextMessage {
                    sender: "Alice".into(),
                    text: "single latest".into(),
                }],
            ),
            "single latest",
            "a one-message conversation should not duplicate the ordinary body"
        );
    }

    /// Answers the architecture question for mirrored notification actions without registering a
    /// COM activator: while the resident daemon is alive, does a foreground toast action invoke the
    /// ToastNotification::Activated event and carry the text input? If this passes on the target
    /// Windows build, Conduit can keep activation inside its already-resident toast thread rather
    /// than adding another local-server process.
    #[test]
    #[ignore = "shows an interactive toast; enter text and select Send"]
    fn a_live_toast_activation_returns_action_and_user_input() -> Result<()> {
        crate::image::ensure_mta();
        let identity_icon = ensure_app_identity_icon(&scratch("aumid"))?;
        register_aumid(&identity_icon)?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(
            r#"<toast>
                 <visual><binding template="ToastGeneric">
                   <text>Conduit action self-test</text>
                   <text>Type a reply, then select Send.</text>
                 </binding></visual>
                 <actions>
                   <input id="reply" type="text" placeHolderContent="Reply text"/>
                   <action content="Send" arguments="action=7" activationType="foreground" hint-inputId="reply"/>
                 </actions>
               </toast>"#,
        ))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        toast.SetTag(&HSTRING::from("activation-test"))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;

        let (tx, rx) = std::sync::mpsc::channel::<(String, String)>();
        let handler = TypedEventHandler::<ToastNotification, windows::core::IInspectable>::new(
            move |_sender, args| {
                let Some(args) = &*args else { return Ok(()) };
                let args: ToastActivatedEventArgs = args.cast()?;
                let arguments = args.Arguments()?.to_string();
                let input = args.UserInput()?;
                let reply = if input.HasKey(&HSTRING::from("reply"))? {
                    input
                        .Lookup(&HSTRING::from("reply"))?
                        .cast::<IPropertyValue>()?
                        .GetString()?
                        .to_string()
                } else {
                    String::new()
                };
                let _ = tx.send((arguments, reply));
                Ok(())
            },
        );
        let token = toast.Activated(&handler)?;
        notifier.Show(&toast)?;
        println!("toast is up — enter any text and select Send within 60 seconds");

        let (arguments, reply) = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .context("the running process never received the toast activation")?;
        toast.RemoveActivated(token)?;
        hide("activation-test")?;
        println!("arguments={arguments:?} reply={reply:?}");
        assert_eq!(arguments, "action=7");
        assert!(!reply.is_empty(), "Windows returned no inline-reply text");
        Ok(())
    }

    #[cfg(test)]
    fn in_history(tag: &str) -> Result<bool> {
        Ok(from_history(tag)?.is_some())
    }

    #[cfg(test)]
    fn from_history(tag: &str) -> Result<Option<ToastNotification>> {
        let history =
            ToastNotificationManager::History()?.GetHistoryWithId(&HSTRING::from(AUMID))?;
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

    #[test]
    fn shared_url_markup_is_bounded_escaped_and_protocol_activated() -> Result<()> {
        let markup = shared_url_xml(
            "https://example.com/a?x=1&y=2",
            "A <page>",
            "OnePlus & phone",
        )?;
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(&markup))?;
        assert!(markup.contains("Page shared from OnePlus &amp; phone"));
        assert!(markup.contains("A &lt;page&gt;"));
        assert!(markup.contains("Open in New Tab"));
        assert!(markup.contains("activationType=\"protocol\""));
        assert!(!safe_web_url("file:///C:/secret.txt"));
        assert!(!safe_web_url("javascript:alert(1)"));
        assert!(safe_web_url("HTTPS://example.com/Path"));
        Ok(())
    }
}
