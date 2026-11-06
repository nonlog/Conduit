#![cfg_attr(windows, windows_subsystem = "windows")]

mod bridge;
mod icons;
mod notices;
mod notification_time;
mod pairing_qr;
mod smooth_scroll;
mod window_chrome;
#[cfg(windows)]
mod window_icon;
mod window_material;

use notices::{Kind as NoticeKind, Notice};
use notify::{RecursiveMode, Watcher};
use rune::tok::{self, Theme, type_};
use rune::{
    App, AppHandle, AppSpec, Color, Cx, El, Env, Icon, Image, ImageCache, Key, KeyEvent, Runtime,
    ScrollSpec, SurfaceFit, TextEdit, Tx, Waker, col, el, icon, row, text, text_input,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::OnceLock;

const PREVIEW_TITLE: &str = "Conduit Rune Preview";
const STABLE_TITLE: &str = "Conduit — Rune";
const LAUNCH_NOTICE_PREFIX: &str = "rune-launch-error-";
static HANDLE: OnceLock<AppHandle<Ui>> = OnceLock::new();

fn stable_executable() -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(|path| path.file_name())
        .is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("conduit-rune.exe")
        })
}

fn window_title() -> &'static str {
    if stable_executable() {
        STABLE_TITLE
    } else {
        PREVIEW_TITLE
    }
}

fn send_error_from_args(args: &[String]) -> Option<String> {
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

fn launch_error() -> Option<String> {
    let args = std::env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    send_error_from_args(&args)
}

fn write_launch_notice(message: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let dir = bridge::data_dir();
    std::fs::create_dir_all(&dir)?;
    let unique = format!(
        "{}{}-{}",
        LAUNCH_NOTICE_PREFIX,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let part = dir.join(format!("{unique}.part"));
    let destination = dir.join(format!("{unique}.txt"));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part)?;
        file.write_all(message.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&part, &destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(part);
    }
    result
}

fn take_launch_notices() -> Vec<String> {
    let mut notices = Vec::new();
    let Ok(files) = std::fs::read_dir(bridge::data_dir()) else {
        return notices;
    };
    for entry in files.flatten().take(64) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(LAUNCH_NOTICE_PREFIX) || !name.ends_with(".txt") {
            continue;
        }
        if entry
            .metadata()
            .is_ok_and(|metadata| metadata.len() <= 4096)
        {
            if let Ok(message) = std::fs::read_to_string(entry.path()) {
                if let Some(safe) =
                    send_error_from_args(&["Conduit".into(), "--send-error".into(), message])
                {
                    notices.push(safe);
                }
            }
        }
        let _ = std::fs::remove_file(entry.path());
        if notices.len() >= 8 {
            break;
        }
    }
    notices
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Overview,
    Transfers,
    Notifications,
    Links,
    Pairing,
    Settings,
    About,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::Overview,
        Page::Transfers,
        Page::Notifications,
        Page::Links,
        Page::Pairing,
        Page::Settings,
        Page::About,
    ];

    fn name(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Transfers => "Transfers",
            Page::Notifications => "Notifications",
            Page::Links => "Shared links",
            Page::Pairing => "Pairing",
            Page::Settings => "Settings",
            Page::About => "About",
        }
    }

    fn glyph(self) -> &'static Icon {
        match self {
            Page::Overview => &icons::HOME,
            Page::Transfers => &icons::TRANSFER,
            Page::Notifications => &icons::BELL,
            Page::Links => &icons::LINK,
            Page::Pairing => &icons::QR_CODE,
            Page::Settings => &icons::SETTINGS,
            Page::About => &icons::INFO,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum TransferFilter {
    #[default]
    All,
    Active,
    Completed,
    Failed,
}
impl TransferFilter {
    fn accepts(self, transfer: &bridge::Transfer) -> bool {
        match self {
            Self::All => true,
            Self::Active => !transfer.terminal,
            Self::Completed => transfer.state == "completed",
            Self::Failed => transfer.state == "failed",
        }
    }
}

struct Ui {
    page: Page,
    snap: bridge::Snapshot,
    image_cache: Option<ImageCache>,
    wallpaper: Option<Image>,
    notification_icons: BTreeMap<String, (bridge::IconStamp, Image)>,
    expanded_notification: Option<String>,
    pairing_qr: Option<Image>,
    pairing_qr_code: String,
    search: TextEdit,
    transfer_filter: TransferFilter,
    appearance: String,
    material: window_material::Material,
    backdrop_active: bool,
    transparency: u8,
    smooth_scrolling: bool,
    motion_allowed: bool,
    wheel_scroll: smooth_scroll::SmoothScroll,
    relays: TextEdit,
    relay_mode: String,
    notification_retention: TextEdit,
    manual_proxy: TextEdit,
    receive_dir: TextEdit,
    proxy_mode: String,
    tray_icon: bool,
    settings_dirty: bool,
    confirm: Option<&'static str>,
    notice: Option<Notice>,
    notice_generation: u64,
    busy: bool,
    link_busy: bool,
    drop_over: bool,
}

impl Ui {
    fn init() -> Self {
        let snap = bridge::snapshot();
        let notice = launch_error().or_else(|| take_launch_notices().pop());
        Ui {
            page: if notice.is_some() {
                Page::Transfers
            } else {
                Page::Overview
            },
            search: TextEdit::new(""),
            transfer_filter: TransferFilter::All,
            appearance: bridge::theme_preference(),
            material: bridge::material_preference(),
            backdrop_active: false,
            transparency: bridge::transparency_preference(),
            smooth_scrolling: bridge::smooth_scroll_preference(),
            motion_allowed: window_material::animations_enabled(),
            wheel_scroll: smooth_scroll::SmoothScroll::default(),
            image_cache: None,
            wallpaper: None,
            notification_icons: BTreeMap::new(),
            expanded_notification: None,
            pairing_qr: None,
            pairing_qr_code: String::new(),
            relays: TextEdit::new(snap.settings.relays.join("\n")),
            relay_mode: snap.settings.relay_mode.clone(),
            notification_retention: TextEdit::new(
                snap.settings.notification_retention_days.to_string(),
            ),
            manual_proxy: TextEdit::new(snap.settings.manual_proxy.clone()),
            receive_dir: TextEdit::new(snap.settings.receive_dir.clone()),
            proxy_mode: snap.settings.proxy_mode.clone(),
            tray_icon: snap.settings.tray_icon,
            snap,
            settings_dirty: false,
            confirm: None,
            notice: notice.map(Notice::error),
            notice_generation: 0,
            busy: false,
            link_busy: false,
            drop_over: false,
        }
    }

    fn sync(&mut self, next: bridge::Snapshot) {
        let notifications_changed = self.snap.notifications != next.notifications;
        let pairing_finished = self.snap.pairing.active
            && !next.pairing.active
            && next.pairing.paired
            && next.connection.linked;
        if !self.settings_dirty {
            self.relays.replace_text(next.settings.relays.join("\n"));
            self.relay_mode = next.settings.relay_mode.clone();
            self.notification_retention
                .replace_text(next.settings.notification_retention_days.to_string());
            self.manual_proxy
                .replace_text(next.settings.manual_proxy.clone());
            self.receive_dir
                .replace_text(next.settings.receive_dir.clone());
            self.proxy_mode = next.settings.proxy_mode.clone();
            self.tray_icon = next.settings.tray_icon;
        }
        self.snap = next;
        self.refresh_pairing_qr();
        if notifications_changed {
            self.refresh_notification_icons();
        }
        if pairing_finished && self.notice.as_ref().is_some_and(|notice| notice.pairing) {
            self.show_notice("Phone paired", NoticeKind::Success);
            self.refresh_wallpaper();
        }
    }

    /// The code is short lived. Cache one bitmap per active code, never use a
    /// browser, web service or periodic background renderer.
    fn refresh_pairing_qr(&mut self) {
        let code = if self.snap.pairing.active {
            self.snap.pairing.code.clone()
        } else {
            String::new()
        };
        if !pairing_qr::needs_refresh(&self.pairing_qr_code, &code, self.pairing_qr.is_some()) {
            return;
        }
        self.pairing_qr_code = code.clone();
        self.pairing_qr = if code.is_empty() {
            None
        } else {
            self.image_cache.as_ref().and_then(|cache| {
                pairing_qr::png_for_code(&code)
                    .ok()
                    .map(|bytes| cache.load(bytes))
            })
        };
    }

    fn refresh_wallpaper(&mut self) {
        if let Some(cache) = &self.image_cache {
            self.wallpaper = bridge::wallpaper_bytes().map(|bytes| cache.load(bytes));
        }
    }

    fn refresh_notification_icons(&mut self) {
        let Some(cache) = &self.image_cache else {
            return;
        };
        let packages: BTreeSet<_> = self
            .snap
            .notifications
            .iter()
            .map(|notification| notification.package.clone())
            .filter(|package| !package.is_empty())
            .collect();
        self.notification_icons
            .retain(|package, _| packages.contains(package));
        for package in packages {
            let Some(stamp) = bridge::notification_icon_stamp(&package) else {
                continue;
            };
            if self
                .notification_icons
                .get(&package)
                .is_some_and(|(old, _)| old == &stamp)
            {
                continue;
            }
            if let Some(bytes) = bridge::notification_icon_bytes(&package) {
                self.notification_icons
                    .insert(package, (stamp, cache.load(bytes)));
            }
        }
    }

    fn show_notice(&mut self, message: impl Into<String>, kind: NoticeKind) -> u64 {
        self.notice_generation = self.notice_generation.wrapping_add(1);
        self.notice = Some(Notice::new(message, kind));
        self.notice_generation
    }

    fn dismiss_notice(&mut self) {
        self.notice_generation = self.notice_generation.wrapping_add(1);
        self.notice = None;
    }

    fn complete_notice(&mut self, ticket: u64, result: Result<(), String>, success: &str) {
        if self.notice_generation != ticket {
            return;
        }
        match result {
            Ok(()) => {
                self.show_notice(success, NoticeKind::Success);
            }
            Err(error) => {
                self.show_notice(error, NoticeKind::Error);
            }
        }
    }

    fn set_transparency(&mut self, percent: u8, persist: bool) {
        self.transparency = percent.min(100);
        if persist {
            if let Err(error) = bridge::save_transparency_preference(self.transparency) {
                self.show_notice(error, NoticeKind::Error);
            }
        }
    }

    fn operation(
        &mut self,
        message: &'static str,
        task: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        // Avoid racing two destructive settings/pairing operations while the
        // daemon still owns the underlying state.
        if self.busy {
            return;
        }
        self.busy = true;
        let ticket = self.show_notice(format!("{message}…"), NoticeKind::Progress);
        if let Some(notice) = &mut self.notice {
            notice.pairing = message == "Starting pairing";
        }
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let result = task();
            let next = bridge::snapshot();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    ui.sync(next);
                    ui.busy = false;
                    if result.is_ok()
                        && message == "Opening file picker"
                        && ui.notice_generation == ticket
                    {
                        ui.dismiss_notice();
                    } else {
                        ui.complete_notice(ticket, result, notices::completion_message(message));
                        if message == "Starting pairing" {
                            if let Some(notice) = &mut ui.notice {
                                notice.pairing = true;
                            }
                        }
                    }
                });
            }
        });
    }

    fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.search.replace_text("");
            self.wheel_scroll.reset();
            self.dismiss_notice();
            self.expanded_notification = None;
        }
        self.page = page;
        self.confirm = None;
    }

    fn set_material(&mut self, material: window_material::Material) {
        match bridge::save_material_preference(material) {
            Ok(()) => {
                self.material = material;
                self.backdrop_active = window_material::apply(material, &self.appearance);
            }
            Err(error) => {
                self.show_notice(error, NoticeKind::Error);
            }
        }
    }

    fn set_smooth_scrolling(&mut self, enabled: bool) {
        match bridge::save_smooth_scroll_preference(enabled) {
            Ok(()) => {
                self.smooth_scrolling = enabled;
                self.wheel_scroll.reset();
            }
            Err(error) => {
                self.show_notice(error, NoticeKind::Error);
            }
        }
    }

    fn save_settings(&mut self) {
        if self.busy {
            return;
        }
        let Ok(retention) = self.notification_retention.text.trim().parse::<u32>() else {
            self.show_notice(
                "Enter notification retention as a whole number of days (0 to 3650)",
                NoticeKind::Error,
            );
            return;
        };
        if retention > 3650 {
            self.show_notice(
                "Notification retention must be between 0 and 3650 days",
                NoticeKind::Error,
            );
            return;
        }
        let settings = bridge::Settings {
            relays: self
                .relays
                .text
                .split([';', '\r', '\n', ','])
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect(),
            relay_mode: self.relay_mode.clone(),
            notification_retention_days: retention,
            proxy_mode: self.proxy_mode.clone(),
            manual_proxy: self.manual_proxy.text.clone(),
            receive_dir: self.receive_dir.text.clone(),
            tray_icon: self.tray_icon,
            ..self.snap.settings.clone()
        };
        self.busy = true;
        let ticket = self.show_notice("Saving settings…", NoticeKind::Progress);
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let result = bridge::save_settings(settings);
            let next = bridge::snapshot();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    // On failure the editor must retain unsaved values instead
                    // of snapping back to the daemon's old config on its next
                    // file notification.
                    if result.is_ok() {
                        ui.settings_dirty = false;
                    }
                    ui.sync(next);
                    ui.busy = false;
                    ui.complete_notice(ticket, result, "Settings saved");
                });
            }
        });
    }

    fn open_picker(&mut self) {
        self.operation("Opening file picker", || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Send a file to your phone")
                .pick_file()
            else {
                return Ok(());
            };
            bridge::send_file(&path)
        });
    }

    fn cancel_transfer(&mut self, id: String) {
        let ticket = self.show_notice("Cancelling transfer…", NoticeKind::Progress);
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let result = bridge::cancel_transfer(&id);
            let next = bridge::snapshot();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    ui.sync(next);
                    ui.complete_notice(ticket, result, "Transfer cancellation requested");
                });
            }
        });
    }

    fn toggle_connection(&mut self) {
        if self.link_busy {
            return;
        }
        let enable = self.snap.connection.paused;
        self.link_busy = true;
        let ticket = self.show_notice(
            if enable {
                "Resuming connection..."
            } else {
                "Disconnecting..."
            },
            NoticeKind::Progress,
        );
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let result =
                bridge::daemon_command(&["link", if enable { "connect" } else { "disconnect" }]);
            let next = bridge::snapshot();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    ui.link_busy = false;
                    ui.sync(next);
                    let success = if enable {
                        "Connection enabled; waiting for your phone"
                    } else {
                        "Disconnected. Pairing is preserved; connect to resume."
                    };
                    ui.complete_notice(ticket, result.map(|_| ()), success);
                });
            }
        });
    }

    fn pick_receive_dir(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .set_title("Choose a folder for received files")
                .pick_folder();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    ui.busy = false;
                    if let Some(path) = chosen {
                        ui.receive_dir
                            .replace_text(path.to_string_lossy().into_owned());
                        ui.settings_dirty = true;
                        ui.show_notice(
                            "Receive folder selected; save settings to apply",
                            NoticeKind::Success,
                        );
                    }
                });
            }
        });
    }

    fn send_dropped(&mut self, files: &[PathBuf]) {
        let files = files.to_vec();
        if files.is_empty() {
            return;
        }
        self.navigate(Page::Transfers);
        self.operation("Queueing files", move || {
            let mut failures = Vec::new();
            for path in files {
                if let Err(error) = bridge::send_file(&path) {
                    failures.push(error);
                }
            }
            if failures.is_empty() {
                Ok(())
            } else {
                Err(failures.join("; "))
            }
        });
    }

    fn confirm_action(&mut self, what: &'static str) -> bool {
        if self.confirm == Some(what) {
            self.confirm = None;
            true
        } else {
            self.confirm = Some(what);
            false
        }
    }
}

#[derive(Clone, Copy)]
struct Paint {
    t: Theme,
    accent: Color,
    primary_fill: Color,
    surface: Color,
    sidebar: Color,
    card: Color,
    inset: Color,
    line: Color,
    subdued: Color,
    faint: Color,
    green: Color,
    danger: Color,
    selected: Color,
    hover_wash: Color,
    press_wash: Color,
}

impl Paint {
    fn new(dark: bool) -> Self {
        let color = |hex| rune::color::hex(hex).expect("valid Conduit design token");
        let t = tok::theme(dark);
        // Keep every neutral surface derived from Rune's semantic theme. The small
        // light-mode mix is intentional: Rune's reference card is pure white, while
        // Conduit needs a quieter elevated surface so cards do not flash against the
        // page when the Windows theme changes.
        let card = t.bg.mix(t.card, if dark { 0.82 } else { 0.20 });
        let inset = t.bg.mix(t.card, if dark { 0.42 } else { 0.10 });
        let selected = t.bg.mix(t.card, if dark { 0.58 } else { 0.34 });
        Paint {
            t,
            accent: if dark {
                color("#82a9ff")
            } else {
                color("#2d6fdb")
            },
            primary_fill: if dark {
                color("#315fbd")
            } else {
                color("#2d6fdb")
            },
            surface: t.bg,
            sidebar: t.bg,
            card,
            inset,
            // Rune's stock card_border/hover tokens are intentionally much
            // brighter than this neutral shell. Borders and interaction
            // overlays must be calibrated against the *actual* card surface.
            line: t.text.alpha(if dark { 0.18 } else { 0.14 }),
            subdued: t.text_secondary,
            faint: t.text_secondary.alpha(0.70),
            green: if dark {
                color("#66cfa9")
            } else {
                color("#137b55")
            },
            danger: if dark {
                color("#f38b8b")
            } else {
                color("#c43d4b")
            },
            selected,
            // El::hover is an OVERLAY, not a replacement fill. Passing the
            // opaque theme hover tint (or giving it 65% opacity) bleaches
            // dark notification cards and destroys text contrast.
            hover_wash: t.text.alpha(if dark { 0.055 } else { 0.040 }),
            press_wash: t.text.alpha(if dark { 0.11 } else { 0.09 }),
        }
    }
}

impl Paint {
    fn with_backdrop(mut self, active: bool, transparency: u8) -> Self {
        if active {
            // Paint the shell ONCE at the root. Its title bar, navigation and
            // content are transparent children, not differently tinted layers.
            self.surface = self
                .surface
                .alpha(1.0 - f32::from(transparency.min(100)) / 100.0);
            self.card = self.card.alpha(0.82);
            self.inset = self.inset.alpha(0.88);
        }
        self.sidebar = self.surface;
        self
    }
}

fn button(
    key: impl Into<String>,
    label: impl Into<String>,
    paint: Paint,
    primary: bool,
    f: impl Fn(&mut Ui) + 'static,
) -> El {
    action_button(key, label, None, paint, primary, f)
}

fn action_button(
    key: impl Into<String>,
    label: impl Into<String>,
    glyph: Option<&'static Icon>,
    paint: Paint,
    primary: bool,
    f: impl Fn(&mut Ui) + 'static,
) -> El {
    let title = label.into();
    let mut content = row()
        .key(key.into())
        .role("button")
        .label(title.clone())
        .h(36.0)
        .px(13.0)
        .gap(8.0)
        .center()
        .radius(9.0)
        .continuous_corners()
        .fill(if primary {
            paint.primary_fill
        } else {
            paint.card
        })
        .border(if primary { 0.0 } else { 1.0 }, paint.line)
        .hover(if primary {
            Color::WHITE.alpha(0.08)
        } else {
            paint.hover_wash
        })
        .press(if primary {
            Color::WHITE.alpha(0.15)
        } else {
            paint.press_wash
        })
        .on_press(f);
    if let Some(glyph) = glyph {
        content = content.child(icon(glyph, 17.0).color(if primary {
            Color::WHITE
        } else {
            paint.t.text
        }));
    }
    content.child(
        text(title)
            .font(rune::TextStyle::new(12.0, 580.0))
            .color(if primary { Color::WHITE } else { paint.t.text }),
    )
}

fn segment(
    key: impl Into<String>,
    label: &'static str,
    selected: bool,
    paint: Paint,
    action: impl Fn(&mut Ui) + 'static,
) -> El {
    row()
        .key(key.into())
        .role("button")
        .label(label)
        .h(34.0)
        .px(13.0)
        .center()
        .radius(7.0)
        .fill(if selected {
            paint.card
        } else {
            Color::TRANSPARENT
        })
        .border(if selected { 1.0 } else { 0.0 }, paint.line)
        .hover(paint.hover_wash)
        .on_press(action)
        .child(
            text(label)
                .font(rune::TextStyle::new(
                    12.0,
                    if selected { 630.0 } else { 480.0 },
                ))
                .color(if selected {
                    paint.t.text
                } else {
                    paint.subdued
                }),
        )
}

fn preference_toggle(
    key: impl Into<String>,
    label: &'static str,
    enabled: bool,
    paint: Paint,
    action: impl Fn(&mut Ui) + 'static,
) -> El {
    row()
        .key(key.into())
        .role("button")
        .label(label)
        .h(38.0)
        .px(13.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .fill(if enabled {
            paint.green.alpha(0.11)
        } else {
            paint.inset
        })
        .border(
            1.0,
            if enabled {
                paint.green.alpha(0.22)
            } else {
                paint.line
            },
        )
        .hover(paint.hover_wash)
        .on_press(action)
        .child(el().size(7.0, 7.0).radius(4.0).fill(if enabled {
            paint.green
        } else {
            paint.faint
        }))
        .child(
            text(if enabled { "On" } else { "Off" })
                .font(rune::TextStyle::new(12.0, 630.0))
                .color(if enabled { paint.green } else { paint.subdued }),
        )
}

fn tag(label: impl Into<String>, paint: Paint, good: bool) -> El {
    row()
        .radius(9.0)
        .px(12.0)
        .gap(8.0)
        .h(30.0)
        .center()
        .fill(if good {
            paint.green.alpha(0.11)
        } else {
            paint.inset
        })
        .border(
            1.0,
            if good {
                paint.green.alpha(0.19)
            } else {
                paint.line
            },
        )
        .child(
            el().size(7.0, 7.0)
                .radius(4.0)
                .fill(if good { paint.green } else { paint.subdued }),
        )
        .child(text(label.into()).font(type_::LABEL).color(if good {
            paint.green
        } else {
            paint.subdued
        }))
}

fn card(key: impl Into<String>, paint: Paint, child: El) -> El {
    col()
        .key(key.into())
        .w_full()
        .padding(18.0)
        .gap(14.0)
        .radius(12.0)
        .continuous_corners()
        .fill(paint.card)
        .border(1.0, paint.line)
        .child(child)
}

fn heading(label: &str, paint: Paint) -> El {
    text(label)
        .font(rune::TextStyle::new(14.0, 650.0))
        .color(paint.t.text)
}

fn page_title(label: &str, paint: Paint) -> El {
    text(label)
        .font(rune::TextStyle::new(24.0, 650.0))
        .color(paint.t.text)
}

fn muted(label: impl Into<String>, paint: Paint) -> El {
    text(label.into()).font(type_::BODY).color(paint.subdued)
}

fn eyebrow(label: impl Into<String>, paint: Paint) -> El {
    text(label.into())
        .font(rune::TextStyle::new(11.0, 650.0))
        .color(paint.faint)
}

fn section_heading(glyph: &'static Icon, label: &str, subtitle: &str, paint: Paint) -> El {
    row()
        .gap(10.0)
        .center_y()
        .child(icon(glyph, 18.0).color(paint.subdued))
        .child(
            col()
                .gap(3.0)
                .child(heading(label, paint))
                .child(text(subtitle).font(type_::LABEL).color(paint.faint)),
        )
}

fn nav(page: Page, selected: bool, paint: Paint) -> El {
    row()
        .key(format!("nav-{}", page.name()))
        .role("button")
        .label(page.name())
        .w_full()
        .h(36.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(7.0)
        .fill(if selected {
            paint.selected
        } else {
            Color::TRANSPARENT
        })
        .hover(paint.hover_wash)
        .on_press(move |ui: &mut Ui| ui.navigate(page))
        .child(
            icon(page.glyph(), 17.0)
                .key(format!("nav-icon-{}", page.name()))
                .color(if selected { paint.accent } else { paint.faint }),
        )
        .child(
            text(page.name())
                .font(rune::TextStyle::new(
                    13.0,
                    if selected { 600.0 } else { 450.0 },
                ))
                .color(if selected {
                    paint.t.text
                } else {
                    paint.subdued
                }),
        )
}

fn sidebar(ui: &Ui, paint: Paint, compact: bool) -> El {
    let logo = icon(&icons::CONDUIT, 24.0)
        .key("conduit-brand-icon")
        .color(paint.t.text);
    let mut menu = col().key("side-navigation").w_full().gap(4.0);
    for page in [
        Page::Overview,
        Page::Transfers,
        Page::Notifications,
        Page::Links,
    ] {
        menu = menu.child(nav(page, ui.page == page, paint));
    }
    menu = menu
        .child(el().h(12.0))
        .child(el().w_full().h(1.0).fill(paint.line.alpha(0.65)))
        .child(el().h(8.0));
    for page in [Page::Pairing, Page::Settings, Page::About] {
        menu = menu.child(nav(page, ui.page == page, paint));
    }
    col()
        .key("sidebar")
        .w(if compact { 176.0 } else { 192.0 })
        .h_full()
        .padding(12.0)
        .gap(18.0)
        .fill(Color::TRANSPARENT)
        .child(
            row()
                .key("brand")
                .h(44.0)
                .gap(9.0)
                .px(7.0)
                .center_y()
                .child(logo)
                .child(
                    text("Conduit")
                        .font(rune::TextStyle::new(16.0, 680.0))
                        .color(paint.t.text),
                ),
        )
        .child(menu)
        .child(el().grow(1.0))
        .child(
            row()
                .key("sidebar-device")
                .role("button")
                .label("Manage phone pairing")
                .w_full()
                .h(44.0)
                .px(8.0)
                .gap(9.0)
                .center_y()
                .radius(8.0)
                .hover(paint.hover_wash)
                .on_press(|ui: &mut Ui| ui.navigate(Page::Pairing))
                .child(icon(&icons::SMARTPHONE, 17.0).color(paint.faint))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(
                            text(if ui.snap.connection.linked {
                                "Phone connected"
                            } else if ui.snap.connection.paused {
                                "Connection paused"
                            } else {
                                "Connect a phone"
                            })
                            .font(rune::TextStyle::new(11.0, 550.0))
                            .color(paint.subdued),
                        )
                        .child(
                            text(if ui.snap.connection.linked {
                                "Encrypted connection"
                            } else {
                                "Ready when you are"
                            })
                            .font(rune::TextStyle::new(10.0, 450.0))
                            .color(paint.faint),
                        ),
                )
                .child(
                    el().size(5.0, 5.0)
                        .radius(3.0)
                        .fill(if ui.snap.connection.linked {
                            paint.green
                        } else {
                            paint.faint
                        }),
                ),
        )
}

fn summary_connection(ui: &Ui, paint: Paint, compact: bool) -> El {
    let conn = &ui.snap.connection;
    let device = if !conn.device.is_empty() {
        conn.device.as_str()
    } else if !ui.snap.pairing.device.is_empty() {
        ui.snap.pairing.device.as_str()
    } else {
        "Your Android phone"
    };
    let route = if conn.paused {
        "Paused"
    } else if !conn.linked {
        "Offline"
    } else if conn.route.to_ascii_lowercase().contains("lan") {
        "Direct LAN"
    } else if conn.route.to_ascii_lowercase().contains("relay") {
        "Relay"
    } else {
        "Connected"
    };
    let phone = if let Some(image) = &ui.wallpaper {
        el().key("phone-wallpaper")
            .size(52.0, 72.0)
            .radius(9.0)
            .pixels_fit(image.pixels(), SurfaceFit::Fill)
    } else {
        el().key("phone-placeholder")
            .size(52.0, 72.0)
            .radius(9.0)
            .fill(paint.inset)
            .center()
            .role("button")
            .label("Wallpaper unavailable: enable Device wallpaper preview in the phone's Settings")
            .focusable(true)
            .child(icon(&icons::SMARTPHONE, 25.0).color(paint.faint))
            .on_press(|ui: &mut Ui| { ui.show_notice(
                "To show the real wallpaper, open Conduit Settings on your phone and enable Device wallpaper preview. Android requires explicit permission.",
                NoticeKind::Error,
            ); })
    };
    let endpoint = if conn.paused {
        "Pairing kept; reconnect when ready".into()
    } else if conn.linked && !conn.relay.is_empty() {
        conn.relay.clone()
    } else if conn.linked {
        "End-to-end encrypted".into()
    } else {
        "Pair a phone or reconnect your device".into()
    };
    card(
        "connection",
        paint,
        row()
            .w_full()
            .center_y()
            .gap(if compact { 14.0 } else { 18.0 })
            .child(phone)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(8.0)
                    .child(
                        text(device)
                            .font(rune::TextStyle::new(21.0, 650.0))
                            .color(paint.t.text)
                            .truncate(),
                    )
                    .child(
                        row()
                            .key("connection-route")
                            .w_full()
                            .min_w(0.0)
                            .center_y()
                            .gap(7.0)
                            .child(el().size(6.0, 6.0).radius(3.0).fill(if conn.linked {
                                paint.green
                            } else {
                                paint.faint
                            }))
                            .child(
                                text(route)
                                    .font(rune::TextStyle::new(12.0, 570.0))
                                    .color(paint.subdued),
                            )
                            .child(
                                text(endpoint)
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .font(rune::TextStyle::new(11.0, 450.0))
                                    .color(paint.faint)
                                    .truncate_middle(),
                            ),
                    ),
            )
            .child(
                col()
                    .gap(8.0)
                    .child(action_button(
                        "connection-send",
                        "Send file",
                        Some(&icons::SEND),
                        paint,
                        true,
                        |ui| ui.open_picker(),
                    ))
                    .child(
                        row()
                            .gap(6.0)
                            .child(button("connection-pair", "Pairing", paint, false, |ui| {
                                ui.navigate(Page::Pairing)
                            }))
                            .child(button(
                                "connection-toggle",
                                if ui.link_busy {
                                    "Working..."
                                } else if conn.paused {
                                    "Connect"
                                } else {
                                    "Disconnect"
                                },
                                paint,
                                conn.paused,
                                |ui| ui.toggle_connection(),
                            )),
                    ),
            ),
    )
}

fn byte_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn transfer_row(transfer: &bridge::Transfer, paint: Paint) -> El {
    let tr = transfer.clone();
    let progress = if tr.total == 0 {
        0
    } else {
        ((tr.bytes as f64 / tr.total as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u32
    };
    let direction = if tr.direction == "outbound" {
        "To phone"
    } else {
        "From phone"
    };
    let detail = if tr.terminal {
        if tr.state == "failed" && !tr.detail.is_empty() {
            format!("{} / {}", direction, tr.detail)
        } else {
            format!("{} / {}", direction, byte_size(tr.total))
        }
    } else {
        format!(
            "{} of {}  /  {}%",
            byte_size(tr.bytes),
            byte_size(tr.total),
            progress
        )
    };
    let mut details = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(5.0)
        .child(
            text(&tr.name)
                .font(rune::TextStyle::new(13.0, 560.0))
                .color(paint.t.text)
                .truncate_middle(),
        )
        .child(
            text(detail)
                .font(rune::TextStyle::new(11.0, 440.0))
                .color(paint.faint)
                .truncate(),
        );
    if !tr.terminal {
        details = details.child(
            row()
                .key(format!("progress-{}", tr.id))
                .w(170.0)
                .h(3.0)
                .radius(2.0)
                .fill(paint.line)
                .child(
                    el().w(progress as f32 * 1.7)
                        .h(3.0)
                        .radius(2.0)
                        .fill(paint.accent),
                ),
        );
    }
    let mut body = row()
        .key(format!("transfer-{}", tr.id))
        .w_full()
        .center_y()
        .gap(12.0)
        .h(if tr.terminal { 55.0 } else { 65.0 })
        .px(6.0)
        .radius(6.0)
        .hover(paint.hover_wash)
        .child(
            el().size(32.0, 36.0)
                .center()
                .child(icon(file_glyph(&tr.name), 20.0).color(paint.subdued)),
        )
        .child(details);
    if !tr.terminal {
        let id = tr.id.clone();
        body = body.child(button(
            format!("cancel-{}", tr.id),
            "Cancel",
            paint,
            false,
            move |ui| {
                let id = id.clone();
                ui.cancel_transfer(id);
            },
        ));
    } else {
        let status_color = match tr.state.as_str() {
            "completed" => paint.green,
            "failed" => paint.danger,
            _ => paint.faint,
        };
        let mut status = row().center_y().gap(6.0);
        if tr.state == "completed" || tr.state == "failed" {
            status = status.child(
                icon(
                    if tr.state == "completed" {
                        &icons::CHECK
                    } else {
                        &icons::CLOSE
                    },
                    14.0,
                )
                .color(status_color),
            );
        }
        body = body.child(
            status.child(
                text(match tr.state.as_str() {
                    "completed" => "Completed",
                    "failed" => "Failed",
                    "cancelled" => "Cancelled",
                    _ => "Finished",
                })
                .font(rune::TextStyle::new(11.0, 530.0))
                .color(status_color),
            ),
        );
    }
    body
}

fn empty_state(title: &str, detail: &str, glyph: &'static Icon, paint: Paint) -> El {
    col()
        .w_full()
        .h(160.0)
        .gap(10.0)
        .center()
        .child(
            el().size(38.0, 38.0)
                .radius(10.0)
                .fill(paint.inset)
                .center()
                .child(icon(glyph, 19.0).color(paint.subdued)),
        )
        .child(heading(title, paint))
        .child(muted(detail, paint))
}

fn transfer_list(ui: &Ui, paint: Paint, limit: usize) -> El {
    let mut list = col().key("transfer-list").w_full().gap(1.0);
    let query = ui.search.text.trim().to_lowercase();
    let records = ui
        .snap
        .transfers
        .iter()
        .filter(|transfer| ui.page != Page::Transfers || ui.transfer_filter.accepts(transfer))
        .filter(|transfer| {
            ui.page != Page::Transfers
                || query.is_empty()
                || [
                    transfer.name.as_str(),
                    transfer.state.as_str(),
                    transfer.direction.as_str(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&query))
        })
        .take(limit)
        .collect::<Vec<_>>();
    if records.is_empty() {
        list = list.child(empty_state(
            if query.is_empty() && ui.transfer_filter == TransferFilter::All {
                "No transfers yet"
            } else {
                "No matching transfers"
            },
            if query.is_empty() && ui.transfer_filter == TransferFilter::All {
                "Send a file or drag it into Conduit."
            } else {
                "Choose another filter or search term."
            },
            &icons::TRANSFER,
            paint,
        ));
    } else {
        for tr in records {
            list = list.child(transfer_row(tr, paint));
        }
    }
    list
}

fn overview(ui: &Ui, paint: Paint) -> El {
    col()
        .key("overview-summary")
        .w_full()
        .gap(12.0)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(heading("Recent transfers", paint))
                .child(button("show-transfers", "View all", paint, false, |ui| {
                    ui.navigate(Page::Transfers)
                })),
        )
        .child(el().w_full().h(1.0).fill(paint.line))
        .child(transfer_list(ui, paint, 4))
        .child(
            row()
                .key("drop-hint")
                .w_full()
                .h(40.0)
                .center_y()
                .gap(8.0)
                .child(icon(&icons::SEND, 14.0).color(paint.faint))
                .child(
                    text("Drop files anywhere to send to your phone")
                        .font(rune::TextStyle::new(11.0, 450.0))
                        .color(paint.faint),
                ),
        )
}

fn transfer_filters(ui: &Ui, paint: Paint) -> El {
    let mut filters = row()
        .key("transfer-filters")
        .gap(2.0)
        .padding(3.0)
        .radius(8.0)
        .fill(paint.inset);
    for (key, label, value) in [
        ("all", "All", TransferFilter::All),
        ("active", "Active", TransferFilter::Active),
        ("completed", "Completed", TransferFilter::Completed),
        ("failed", "Failed", TransferFilter::Failed),
    ] {
        filters = filters.child(segment(
            format!("filter-{key}"),
            label,
            ui.transfer_filter == value,
            paint,
            move |ui| ui.transfer_filter = value,
        ));
    }
    filters
}

fn transfers(ui: &Ui, paint: Paint) -> El {
    col()
        .key("transfers")
        .w_full()
        .gap(16.0)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(transfer_filters(ui, paint))
                .child(action_button(
                    "send-new-file",
                    "Send file",
                    Some(&icons::SEND),
                    paint,
                    true,
                    |ui| ui.open_picker(),
                )),
        )
        .child(el().w_full().h(1.0).fill(paint.line.alpha(0.65)))
        .child(transfer_list(ui, paint, usize::MAX))
}

fn file_glyph(name: &str) -> &'static Icon {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "heic" => &icons::FILE_IMAGE,
        "zip" | "7z" | "rar" | "tar" | "gz" => &icons::FILE_ARCHIVE,
        "mp4" | "mkv" | "mov" | "webm" => &icons::FILE_VIDEO,
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => &icons::FILE_AUDIO,
        _ => &icons::FILE_TEXT,
    }
}

fn notification_toolbar(ui: &Ui, paint: Paint) -> El {
    let days = ui.snap.settings.notification_retention_days;
    let retention = if days == 0 {
        "No age limit".to_owned()
    } else {
        format!("Keep {days} days")
    };
    row()
        .key("notification-toolbar")
        .w_full()
        .center_y()
        .gap(8.0)
        .child(
            text(format!("{} notifications", ui.snap.notifications.len()))
                .grow(1.0)
                .min_w(0.0)
                .font(rune::TextStyle::new(12.0, 500.0))
                .color(paint.subdued)
                .truncate(),
        )
        .child(button(
            "notification-retention",
            retention,
            paint,
            false,
            |ui| ui.navigate(Page::Settings),
        ))
        .child(button(
            "clear-notifications",
            if ui.confirm == Some("notifications") {
                "Confirm clear"
            } else {
                "Clear history"
            },
            paint,
            ui.confirm == Some("notifications"),
            |ui| {
                if ui.confirm_action("notifications") {
                    ui.operation("Clearing notifications", || {
                        bridge::clear_history("notifications")
                    });
                }
            },
        ))
}

fn notification_app_icon(ui: &Ui, notification: &bridge::Notification, paint: Paint) -> El {
    let container = el()
        .key(format!("notification-icon-{}", notification.id))
        .size(34.0, 34.0)
        .min_w(34.0)
        .radius(9.0)
        .role("image")
        .label(format!("{} app icon", notification.app))
        .center();
    if let Some((_, image)) = ui.notification_icons.get(&notification.package) {
        return container.pixels_fit(image.pixels(), SurfaceFit::Fit);
    }
    // A missing cache entry has an honest app initial, never an unrelated logo.
    let initial = notification
        .app
        .chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    container.fill(paint.accent.alpha(0.09)).child(
        text(initial)
            .font(rune::TextStyle::new(16.0, 600.0))
            .color(paint.accent),
    )
}

fn notifications(ui: &Ui, paint: Paint) -> El {
    let query = ui.search.text.trim().to_lowercase();
    let records: Vec<_> = ui
        .snap
        .notifications
        .iter()
        .filter(|record| {
            query.is_empty()
                || [&record.app, &record.title, &record.body]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&query))
        })
        .collect();
    let mut list = col().key("notification-timeline").w_full().gap(9.0);
    if records.is_empty() {
        return list.child(empty_state(
            if query.is_empty() {
                "No notifications yet"
            } else {
                "No matching notifications"
            },
            if query.is_empty() {
                "Your phone's notifications will appear here, with their app icons and time."
            } else {
                "Try another app, sender or keyword."
            },
            &icons::BELL,
            paint,
        ));
    }
    let mut day = String::new();
    for notification in records {
        let stamp = notification_time::format(notification.timestamp_ms);
        if day != stamp.day_key {
            day = stamp.day_key.clone();
            list = list.child(
                row()
                    .key(format!("notification-day-{day}"))
                    .w_full()
                    .h(28.0)
                    .center_y()
                    .gap(12.0)
                    .child(
                        text(stamp.day.clone())
                            .font(rune::TextStyle::new(11.0, 560.0))
                            .color(paint.subdued),
                    )
                    .child(el().grow(1.0).h(1.0).fill(paint.line.alpha(0.65))),
            );
        }
        let expanded = ui.expanded_notification.as_deref() == Some(notification.id.as_str());
        let mut message = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(6.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(12.0)
                    .child(
                        text(notification.app.clone())
                            .grow(1.0)
                            .min_w(0.0)
                            .font(rune::TextStyle::new(11.0, 550.0))
                            .color(paint.subdued)
                            .truncate(),
                    )
                    .child(
                        text(stamp.time.clone())
                            .key(format!("notification-time-{}", notification.id))
                            .font(rune::TextStyle::new(11.0, 450.0))
                            .color(paint.faint)
                            .label(stamp.full.clone()),
                    ),
            )
            .child(
                text(notification.title.clone())
                    .w_full()
                    .font(rune::TextStyle::new(13.0, 600.0))
                    .color(paint.t.text)
                    .max_lines(if expanded { 4 } else { 2 }),
            );
        if !notification.body.is_empty() {
            message = message.child(
                text(notification.body.clone())
                    .w_full()
                    .font(rune::TextStyle::new(12.0, 450.0))
                    .color(paint.subdued)
                    .max_lines(if expanded { 20 } else { 2 }),
            );
        }
        if expanded {
            message = message.child(
                text(stamp.full)
                    .font(rune::TextStyle::new(10.0, 450.0))
                    .color(paint.faint),
            );
        }
        let id = notification.id.clone();
        list = list.child(
            row()
                .key(format!("notification-{}", notification.id))
                .w_full()
                .padding(14.0)
                .gap(12.0)
                .radius(11.0)
                .fill(paint.card)
                .border(1.0, paint.line.alpha(0.8))
                .hover(paint.hover_wash)
                .press(paint.press_wash)
                .focusable(true)
                .role("button")
                .label(format!(
                    "{}, {}, {}",
                    notification.app, notification.title, stamp.time
                ))
                .child(notification_app_icon(ui, notification, paint))
                .child(message)
                .on_press(move |ui: &mut Ui| {
                    ui.expanded_notification =
                        if ui.expanded_notification.as_deref() == Some(id.as_str()) {
                            None
                        } else {
                            Some(id.clone())
                        };
                }),
        );
    }
    list
}

fn links(ui: &Ui, paint: Paint) -> El {
    let mut list = col().gap(3.0);
    let query = ui.search.text.trim().to_lowercase();
    let records = ui
        .snap
        .links
        .iter()
        .filter(|item| {
            query.is_empty()
                || [item.title.as_str(), item.url.as_str(), item.source.as_str()]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        list = list.child(empty_state(
            if query.is_empty() {
                "No shared links yet"
            } else {
                "No matching links"
            },
            if query.is_empty() {
                "Links sent from your phone appear here."
            } else {
                "Try a different title or address."
            },
            &icons::LINK,
            paint,
        ));
    } else {
        for item in records {
            let url = item.url.clone();
            let title = if item.title.chars().count() > 58 {
                format!("{}…", item.title.chars().take(57).collect::<String>())
            } else {
                item.title.clone()
            };
            let preview = if item.url.chars().count() > 70 {
                format!("{}…", item.url.chars().take(69).collect::<String>())
            } else {
                item.url.clone()
            };
            list = list.child(
                row()
                    .key(format!("link-{}", item.id))
                    .w_full()
                    .padding(12.0)
                    .gap(13.0)
                    .center_y()
                    .radius(9.0)
                    .hover(paint.hover_wash)
                    .child(
                        el().size(36.0, 36.0)
                            .radius(9.0)
                            .fill(paint.inset)
                            .center()
                            .child(icon(&icons::LINK, 18.0).color(paint.subdued)),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(5.0)
                            .child(
                                text(title)
                                    .font(rune::TextStyle::new(13.0, 590.0))
                                    .color(paint.t.text)
                                    .truncate(),
                            )
                            .child(
                                muted(format!("{} · {}", item.source, preview), paint)
                                    .truncate_middle(),
                            ),
                    )
                    .child(action_button(
                        format!("open-{}", item.id),
                        "Open",
                        Some(&icons::ARROW_UP_RIGHT),
                        paint,
                        false,
                        move |ui| {
                            let url = url.clone();
                            ui.operation("Opening link", move || bridge::open_external(&url));
                        },
                    )),
            );
        }
    }
    card(
        "links",
        paint,
        col()
            .gap(14.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(section_heading(
                        &icons::LINK,
                        "Shared links",
                        "Links received from your phone",
                        paint,
                    ))
                    .child(button(
                        "clear-links",
                        if ui.confirm == Some("links") {
                            "Confirm clear"
                        } else {
                            "Clear"
                        },
                        paint,
                        ui.confirm == Some("links"),
                        |ui| {
                            if ui.confirm_action("links") {
                                ui.operation("Clearing links", || bridge::clear_history("links"));
                            }
                        },
                    )),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(list),
    )
}

fn pairing(ui: &Ui, paint: Paint) -> El {
    let p = &ui.snap.pairing;
    let mut content = col().w_full().gap(18.0).child(
        row()
            .w_full()
            .center_y()
            .justify_between()
            .child(heading(
                if p.active {
                    "Scan to pair"
                } else {
                    "Phone connection"
                },
                paint,
            ))
            .child(tag(
                if p.paired { "Paired" } else { "Not paired" },
                paint,
                p.paired,
            )),
    );
    if p.active {
        let qr = el()
            .key("pairing-qr")
            .label("Temporary pairing QR code")
            .size(212.0, 212.0)
            .radius(8.0)
            .fill(Color::WHITE);
        let qr = if let Some(image) = &ui.pairing_qr {
            qr.pixels_fit(image.pixels(), SurfaceFit::Fit)
        } else {
            qr.center()
                .child(icon(&icons::QR_CODE, 40.0).color(paint.faint))
        };
        content = content.child(
            row().w_full().gap(18.0).center_y().child(qr).child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(14.0)
                    .child(
                        muted(
                            "Open Conduit on your phone, then choose Pair desktop > Scan QR code.",
                            paint,
                        )
                        .w_full()
                        .max_lines(3),
                    )
                    .child(
                        text("Or enter this code")
                            .font(rune::TextStyle::new(11.0, 470.0))
                            .color(paint.faint),
                    )
                    .child(
                        text(&p.code)
                            .font(rune::TextStyle::new(28.0, 650.0))
                            .color(paint.t.text),
                    )
                    .child(
                        text("Valid for two minutes")
                            .font(rune::TextStyle::new(11.0, 450.0))
                            .color(paint.faint),
                    )
                    .child(row().child(button(
                        "cancel-pair",
                        "Cancel pairing",
                        paint,
                        false,
                        |ui| {
                            ui.operation("Cancelling pairing", || {
                                bridge::daemon_command(&["pair", "cancel"]).map(|_| ())
                            })
                        },
                    ))),
            ),
        );
    } else {
        if p.paired {
            content = content.child(
                row()
                    .w_full()
                    .center_y()
                    .gap(12.0)
                    .child(icon(&icons::SMARTPHONE, 25.0).color(paint.accent))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(5.0)
                            .child(
                                text(&p.device)
                                    .font(rune::TextStyle::new(17.0, 620.0))
                                    .color(paint.t.text)
                                    .truncate(),
                            )
                            .child(muted("Trusted phone", paint)),
                    )
                    .child(button(
                        "forget-pair",
                        if ui.confirm == Some("forget") {
                            "Confirm unpair"
                        } else {
                            "Forget device"
                        },
                        paint,
                        ui.confirm == Some("forget"),
                        |ui| {
                            if ui.confirm_action("forget") {
                                ui.operation("Forgetting device", || {
                                    bridge::daemon_command(&["pair", "forget"]).map(|_| ())
                                })
                            }
                        },
                    )),
            );
        } else {
            content = content.child(
                muted(
                    "Connect your Android phone using a QR code or six-digit code.",
                    paint,
                )
                .max_lines(2),
            );
        }
        content = content.child(row().child(action_button(
            "start-pair",
            "Start pairing",
            Some(&icons::QR_CODE),
            paint,
            true,
            |ui| {
                ui.operation("Starting pairing", || {
                    bridge::daemon_command(&["pair", "start"]).map(|_| ())
                })
            },
        )));
    }
    content = content.child(
        muted(
            "Each computer needs its own identity. Do not copy identity.bin between PCs.",
            paint,
        )
        .w_full()
        .max_lines(2),
    );
    card("pairing", paint, content)
}

fn field(label: &str, edit: &TextEdit, paint: Paint, editable: fn(&mut Ui) -> &mut TextEdit) -> El {
    col()
        .gap(7.0)
        .w_full()
        .child(text(label).font(type_::LABEL).color(paint.subdued))
        .child(
            row()
                .w_full()
                .min_h(40.0)
                .px(12.0)
                .center_y()
                .fill(paint.inset)
                .radius(8.0)
                .border(1.0, paint.line)
                .child(
                    text_input(edit, type_::BODY)
                        .w_full()
                        .single_line()
                        .color(paint.t.text)
                        .on_edit(move |ui: &mut Ui| {
                            ui.settings_dirty = true;
                            editable(ui)
                        }),
                ),
        )
}

fn relays_field(ui: &Ui, paint: Paint) -> El {
    let mut modes = row().gap(3.0).padding(3.0).radius(9.0).fill(paint.inset);
    for (mode, label) in [
        ("default", "Default nodes"),
        ("custom", "Custom"),
        ("off", "Disabled"),
    ] {
        modes = modes.child(segment(
            format!("relay-mode-{mode}"),
            label,
            ui.relay_mode == mode,
            paint,
            move |ui| {
                ui.relay_mode = mode.into();
                if mode == "default" {
                    ui.relays
                        .replace_text(bridge::split_relays(bridge::DEFAULT_RELAYS).join("\n"));
                }
                ui.settings_dirty = true;
            },
        ));
    }
    let mut content = col().w_full().gap(9.0).child(
        row()
            .w_full()
            .center_y()
            .justify_between()
            .child(text("Relay nodes").font(type_::LABEL).color(paint.subdued))
            .child(modes),
    );
    if ui.relay_mode == "custom" {
        content = content
            .child(
                col()
                    .w_full()
                    .min_h(98.0)
                    .padding(12.0)
                    .fill(paint.inset)
                    .radius(8.0)
                    .border(1.0, paint.line)
                    .child(
                        text_input(&ui.relays, type_::BODY)
                            .key("relay-nodes-editor")
                            .w_full()
                            .min_h(74.0)
                            .color(paint.t.text)
                            .on_edit(|s: &mut Ui| {
                                s.settings_dirty = true;
                                &mut s.relays
                            })
                            .on_submit(|s: &mut Ui, _| {
                                s.relays.insert("\n");
                                s.settings_dirty = true;
                            }),
                    ),
            )
            .child(
                text("One host:port per line. Changes apply when you save.")
                    .font(rune::TextStyle::new(10.0, 450.0))
                    .color(paint.faint),
            );
    } else if ui.relay_mode == "off" {
        content = content.child(
            muted(
                "Relay is disabled. Use a direct LAN connection to reach your phone.",
                paint,
            )
            .max_lines(2),
        );
    } else {
        let mut nodes = col()
            .key("default-relay-nodes")
            .w_full()
            .gap(9.0)
            .padding(12.0)
            .radius(8.0)
            .fill(paint.inset)
            .border(1.0, paint.line);
        for (index, endpoint) in bridge::split_relays(bridge::DEFAULT_RELAYS)
            .iter()
            .enumerate()
        {
            nodes = nodes.child(
                row()
                    .key(format!("default-relay-{index}"))
                    .gap(9.0)
                    .center_y()
                    .child(icon(&icons::LINK, 13.0).color(paint.faint))
                    .child(
                        text(endpoint.clone())
                            .font(rune::TextStyle::new(12.0, 450.0))
                            .color(paint.t.text),
                    ),
            );
        }
        content = content.child(nodes).child(
            text("Built-in fallback nodes; an empty configuration does not disable them.")
                .font(rune::TextStyle::new(10.0, 450.0))
                .color(paint.faint)
                .max_lines(2),
        );
    }
    if ui.snap.settings.relays_source == "environment" {
        content = content.child(
            text(format!(
                "Environment override is active: {}",
                ui.snap.settings.effective_relays.join(", ")
            ))
            .font(rune::TextStyle::new(11.0, 500.0))
            .color(paint.subdued)
            .max_lines(3),
        );
    }
    content
}

fn transparency_at(x: f32, width: f32) -> u8 {
    if !x.is_finite() || !width.is_finite() || width <= 16.0 {
        return 0;
    }
    (((x - 8.0) / (width - 16.0)).clamp(0.0, 1.0) * 100.0).round() as u8
}

fn transparency_control(ui: &Ui, paint: Paint) -> El {
    let slider_width = 216.0;
    let slider = row()
        .key("transparency-slider")
        .size(slider_width, 28.0)
        .center_y()
        .role("slider")
        .label(format!(
            "Background transparency, {} percent",
            ui.transparency
        ))
        .focusable(true)
        .child(
            el().w((slider_width - 16.0) * f32::from(ui.transparency) / 100.0)
                .h(4.0)
                .radius(2.0)
                .fill(paint.accent),
        )
        .child(
            el().size(16.0, 16.0)
                .radius(8.0)
                .fill(paint.accent)
                .border(2.0, paint.card),
        )
        .child(
            el().grow(1.0)
                .min_w(0.0)
                .h(4.0)
                .radius(2.0)
                .fill(paint.line),
        )
        .on_drag(|s: &mut Ui, drag, tx| {
            if drag.phase == rune::Phase::Down {
                drag.data = f32::from(s.transparency);
            }
            let percent = if drag.cancelled() {
                drag.data as u8
            } else {
                transparency_at(drag.x, drag.w)
            };
            s.set_transparency(percent, drag.phase == rune::Phase::Up && !drag.cancelled());
            tx.instant();
        })
        .on_step(|s: &mut Ui, delta, tx| {
            s.set_transparency(
                (i32::from(s.transparency) + delta).clamp(0, 100) as u8,
                true,
            );
            tx.instant();
        })
        .on_step_limit(|s: &mut Ui, maximum, tx| {
            s.set_transparency(if maximum { 100 } else { 0 }, true);
            tx.instant();
        });
    col().w_full().gap(8.0)
        .child(row().w_full().center_y().justify_between()
            .child(heading("Background transparency", paint))
            .child(text(format!("{}%", ui.transparency)).key("transparency-value")
                .font(rune::TextStyle::new(13.0, 600.0)).color(paint.t.text)))
        .child(row().w_full().center_y().gap(16.0)
            .child(text("One material across the title bar, sidebar and content. Text stays opaque.")
                .grow(1.0).min_w(0.0).max_lines(2).font(rune::TextStyle::new(10.0, 450.0)).color(paint.faint))
            .child(slider))
}

fn notification_history_settings(ui: &Ui, paint: Paint) -> El {
    let mut presets = row().gap(3.0).padding(3.0).radius(9.0).fill(paint.inset);
    for (days, label) in [
        ("7", "7 days"),
        ("30", "30 days"),
        ("90", "90 days"),
        ("0", "Off"),
    ] {
        presets = presets.child(segment(
            format!("retention-{days}"),
            label,
            ui.notification_retention.text.trim() == days,
            paint,
            move |s| {
                s.notification_retention.replace_text(days);
                s.settings_dirty = true;
            },
        ));
    }
    card("settings-notification-history", paint, col().w_full().gap(13.0)
        .child(heading("Notification history", paint))
        .child(muted("Automatically delete desktop records older than the chosen number of days.", paint).max_lines(2))
        .child(row().w_full().center_y().gap(10.0)
            .child(text("Keep for").font(type_::BODY).color(paint.subdued))
            .child(row().w(74.0).h(36.0).px(10.0).center_y().fill(paint.inset).radius(8.0).border(1.0, paint.line)
                .child(text_input(&ui.notification_retention, type_::BODY).key("notification-retention-days")
                    .w_full().single_line().color(paint.t.text).label("Notification history retention in days")
                    .on_edit(|s: &mut Ui| { s.settings_dirty = true; &mut s.notification_retention })))
            .child(text("days").font(type_::BODY).color(paint.subdued))
            .child(el().grow(1.0)))
        .child(row().child(presets))
        .child(text("0 disables age-based cleanup. Saving applies the rule immediately. Phone notifications are not deleted.")
            .font(rune::TextStyle::new(10.0, 450.0)).color(paint.faint).max_lines(3)))
}

fn settings(ui: &Ui, paint: Paint) -> El {
    let mut proxy_modes = row().gap(3.0).padding(3.0).radius(9.0).fill(paint.inset);
    for (mode, label) in [
        ("direct", "Direct"),
        ("system", "System proxy"),
        ("manual", "SOCKS5"),
    ] {
        let selected = ui.proxy_mode == mode;
        proxy_modes = proxy_modes.child(segment(
            format!("proxy-{mode}"),
            label,
            selected,
            paint,
            move |ui| {
                ui.settings_dirty = true;
                ui.proxy_mode = mode.to_owned();
            },
        ));
    }
    let mut appearance = row().gap(3.0).padding(3.0).radius(9.0).fill(paint.inset);
    for (mode, label) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
        appearance = appearance.child(segment(
            format!("appearance-{mode}"),
            label,
            ui.appearance == mode,
            paint,
            move |ui| match bridge::save_theme_preference(mode) {
                Ok(()) => {
                    ui.appearance = mode.into();
                    ui.backdrop_active = window_material::apply(ui.material, mode);
                    #[cfg(windows)]
                    if let Err(error) = window_icon::update_appearance(mode) {
                        ui.show_notice(
                            format!("Could not update window appearance: {error}"),
                            NoticeKind::Error,
                        );
                    }
                }
                Err(error) => {
                    ui.show_notice(error, NoticeKind::Error);
                }
            },
        ));
    }
    let mut materials = row().gap(3.0).padding(3.0).radius(9.0).fill(paint.inset);
    for (material, label) in [
        (window_material::Material::Acrylic, "Acrylic"),
        (window_material::Material::Mica, "Mica"),
        (window_material::Material::Solid, "Solid"),
    ] {
        materials = materials.child(segment(
            format!("material-{}", material.name()),
            label,
            ui.material == material,
            paint,
            move |ui| ui.set_material(material),
        ));
    }
    let appearance_card = card(
        "settings-appearance",
        paint,
        col()
            .w_full()
            .gap(16.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(5.0)
                            .child(heading("Appearance", paint))
                            .child(muted("Choose how Conduit looks", paint)),
                    )
                    .child(appearance),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(5.0)
                            .child(heading("Window material", paint))
                            .child(
                                text(if ui.backdrop_active {
                                    "Native Windows backdrop"
                                } else {
                                    "Solid fallback when transparency is unavailable"
                                })
                                .font(rune::TextStyle::new(10.0, 450.0))
                                .color(paint.faint)
                                .max_lines(2),
                            ),
                    )
                    .child(materials),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(transparency_control(ui, paint))
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(5.0)
                            .child(heading("Smooth wheel scrolling", paint))
                            .child(
                                text("Respects Windows animation settings; trackpad stays direct")
                                    .font(rune::TextStyle::new(10.0, 450.0))
                                    .color(paint.faint)
                                    .max_lines(2),
                            ),
                    )
                    .child(preference_toggle(
                        "smooth-scroll",
                        "Toggle smooth wheel scrolling",
                        ui.smooth_scrolling,
                        paint,
                        |ui| ui.set_smooth_scrolling(!ui.smooth_scrolling),
                    )),
            ),
    );
    let mut connection = col()
        .gap(15.0)
        .child(heading("Connection & network", paint))
        .child(muted(
            "Relays are used when a direct LAN path is unavailable.",
            paint,
        ))
        .child(relays_field(ui, paint))
        .child(eyebrow("PROXY ROUTE", paint))
        .child(row().child(proxy_modes));
    if ui.proxy_mode == "manual" {
        connection = connection.child(field("SOCKS5 URL", &ui.manual_proxy, paint, |s| {
            &mut s.manual_proxy
        }));
    }
    let files_card = card(
        "settings-files",
        paint,
        col()
            .gap(14.0)
            .child(heading("Files & background", paint))
            .child(field("Receive folder", &ui.receive_dir, paint, |s| {
                &mut s.receive_dir
            }))
            .child(
                row()
                    .gap(8.0)
                    .child(action_button(
                        "browse-receive-folder",
                        "Choose folder",
                        Some(&icons::FOLDER),
                        paint,
                        false,
                        |ui| ui.pick_receive_dir(),
                    ))
                    .child(button(
                        "folder-received",
                        "Open downloads",
                        paint,
                        false,
                        |ui| ui.operation("Opening downloads", || bridge::open_folder("received")),
                    )),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(4.0)
                            .child(heading("System tray icon", paint))
                            .child(muted("Keep Conduit available in the background", paint)),
                    )
                    .child(preference_toggle(
                        "toggle-tray",
                        "Toggle system tray icon",
                        ui.tray_icon,
                        paint,
                        |s| {
                            s.tray_icon = !s.tray_icon;
                            s.settings_dirty = true;
                        },
                    )),
            ),
    );
    col()
        .w_full()
        .gap(14.0)
        .child(appearance_card)
        .child(notification_history_settings(ui, paint))
        .child(card("settings", paint, connection))
        .child(files_card)
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(
                    text(if ui.settings_dirty {
                        "You have unsaved changes"
                    } else {
                        "Settings are up to date"
                    })
                    .font(type_::LABEL)
                    .color(paint.faint),
                )
                .child(action_button(
                    "save-settings",
                    "Save settings",
                    Some(&icons::CHECK),
                    paint,
                    true,
                    |ui| ui.save_settings(),
                )),
        )
}

fn windows_integration(ui: &Ui, paint: Paint) -> El {
    card(
        "windows-integration",
        paint,
        col()
            .gap(15.0)
            .child(heading("Windows integration", paint))
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(4.0)
                            .child(heading("Start with Windows", paint))
                            .child(muted(
                                "Run the Conduit background service on sign-in",
                                paint,
                            )),
                    )
                    .child(preference_toggle(
                        "autostart",
                        "Toggle Windows autostart",
                        ui.snap.settings.autostart,
                        paint,
                        |ui| {
                            let cmd = if ui.snap.settings.autostart {
                                "remove"
                            } else {
                                "install"
                            };
                            ui.operation("Updating startup", move || {
                                bridge::daemon_command(&["autostart", cmd]).map(|_| ())
                            });
                        },
                    )),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        col()
                            .gap(4.0)
                            .child(heading("Explorer Send to phone", paint))
                            .child(muted("Add Conduit to the Windows context menu", paint)),
                    )
                    .child(preference_toggle(
                        "explorer",
                        "Toggle Explorer Send to phone",
                        ui.snap.settings.explorer,
                        paint,
                        |ui| {
                            let cmd = if ui.snap.settings.explorer {
                                "remove"
                            } else {
                                "install"
                            };
                            ui.operation("Updating Explorer", move || {
                                bridge::daemon_command(&["explorer", cmd]).map(|_| ())
                            });
                        },
                    )),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(eyebrow("LOCAL DATA", paint))
            .child(row().gap(8.0).child(button(
                "data-folder",
                "Open data folder",
                paint,
                false,
                |ui| ui.operation("Opening data folder", || bridge::open_folder("data")),
            ))),
    )
}

fn about(ui: &Ui, paint: Paint) -> El {
    card(
        "about",
        paint,
        col()
            .gap(18.0)
            .child(
                row()
                    .center_y()
                    .gap(14.0)
                    .child(
                        el().size(54.0, 54.0)
                            .radius(13.0)
                            .fill(paint.selected)
                            .center()
                            .child(icon(&icons::CONDUIT, 27.0).color(paint.accent)),
                    )
                    .child(
                        col()
                            .gap(5.0)
                            .child(
                                text("Conduit for Windows")
                                    .font(rune::TextStyle::new(19.0, 660.0))
                                    .color(paint.t.text),
                            )
                            .child(muted(
                                format!(
                                    "Version {}  ·  Rune native desktop",
                                    env!("CARGO_PKG_VERSION")
                                ),
                                paint,
                            )),
                    ),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(
                muted(
                    "Securely transfer files, notifications and links between Windows and Android.",
                    paint,
                )
                .max_lines(2),
            )
            .child(tag(
                if ui.snap.daemon_available {
                    "Background service available"
                } else {
                    "Background service unavailable"
                },
                paint,
                ui.snap.daemon_available,
            ))
            .child(
                muted(
                    format!("Data directory: {}", bridge::data_dir().display()),
                    paint,
                )
                .truncate_middle(),
            )
            .child(eyebrow("PROJECT", paint))
            .child(row().gap(8.0).child(action_button(
                "about-github",
                "GitHub",
                Some(&icons::ARROW_UP_RIGHT),
                paint,
                false,
                |ui| {
                    ui.operation("Opening GitHub", || {
                        bridge::open_external("https://github.com/nonlog/Conduit")
                    })
                },
            ))),
    )
}

fn content(ui: &Ui, paint: Paint, compact: bool) -> El {
    let mut body = col().key("page-content").w_full().min_w(0.0).gap(24.0);
    if ui.page == Page::Overview {
        body = body
            .child(summary_connection(ui, paint, compact))
            .child(overview(ui, paint));
    } else {
        body = body.child(match ui.page {
            Page::Transfers => transfers(ui, paint),
            Page::Notifications => notifications(ui, paint),
            Page::Links => links(ui, paint),
            Page::Pairing => pairing(ui, paint),
            Page::Settings => settings(ui, paint),
            Page::About => about(ui, paint),
            Page::Overview => unreachable!(),
        });
    }
    if ui.page == Page::Settings {
        body = body.child(windows_integration(ui, paint));
    }
    body
}

fn search_field(ui: &Ui, paint: Paint) -> El {
    row()
        .key("search-box")
        .w(255.0)
        .h(38.0)
        .px(12.0)
        .center_y()
        .radius(8.0)
        .fill(paint.inset)
        .border(1.0, paint.line)
        .child(
            text_input(&ui.search, type_::BODY)
                .key("search-input")
                .w_full()
                .single_line()
                .placeholder("Search…", paint.t.text_secondary)
                .color(paint.t.text)
                .on_edit(|ui: &mut Ui| &mut ui.search),
        )
}

fn caption(paint: Paint) -> El {
    let control = |key: &'static str, label: &'static str, glyph: El, action: fn()| {
        el().key(key)
            .w(window_chrome::BUTTON_WIDTH)
            .h(window_chrome::CAPTION_HEIGHT)
            .center()
            .role("button")
            .label(label)
            .focusable(true)
            .hover(if key == "caption-close" {
                paint.danger.alpha(0.2)
            } else {
                paint.hover_wash
            })
            .child(glyph)
            .on_press(move |_: &mut Ui| action())
    };
    row()
        .key("caption")
        .w_full()
        .h(window_chrome::CAPTION_HEIGHT)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .padding(12.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CONDUIT, 14.0).color(paint.subdued))
                .child(
                    text("Conduit")
                        .font(rune::TextStyle::new(11.0, 500.0))
                        .color(paint.subdued),
                ),
        )
        .child(control(
            "caption-minimize",
            "Minimize window",
            el().size(10.0, 1.0).fill(paint.subdued),
            window_chrome::minimize,
        ))
        .child(control(
            "caption-maximize",
            "Maximize or restore window",
            el().size(10.0, 10.0).border(1.0, paint.subdued).radius(1.0),
            window_chrome::toggle_maximize,
        ))
        .child(control(
            "caption-close",
            "Close window",
            text("×")
                .font(rune::TextStyle::new(20.0, 400.0))
                .color(paint.subdued),
            window_chrome::close,
        ))
}

fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let compact = cx.layout_size("window").is_some_and(|size| size[0] < 930.0);
    let dark = match ui.appearance.as_str() {
        "light" => false,
        "dark" => true,
        _ => window_material::effective_dark("system"),
    };
    let paint = Paint::new(dark).with_backdrop(ui.backdrop_active, ui.transparency);
    let searchable = matches!(ui.page, Page::Transfers | Page::Notifications | Page::Links);
    let mut right = row().gap(10.0).center_y();
    if searchable && !compact {
        right = right.child(search_field(ui, paint));
    }
    if !searchable {
        right = right.child(
            row()
                .key("header-status")
                .gap(7.0)
                .center_y()
                .child(
                    el().size(6.0, 6.0)
                        .radius(3.0)
                        .fill(if ui.snap.connection.linked {
                            paint.green
                        } else {
                            paint.faint
                        }),
                )
                .child(
                    text(if ui.snap.connection.linked {
                        "Connected"
                    } else if ui.snap.connection.paused {
                        "Paused"
                    } else {
                        "Offline"
                    })
                    .font(rune::TextStyle::new(11.0, 530.0))
                    .color(paint.subdued),
                ),
        );
    }
    let mut body = col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(if compact { 20.0 } else { 26.0 })
        .fill(Color::TRANSPARENT)
        .gap(16.0)
        .child(
            row()
                .key("page-header")
                .w_full()
                .h(44.0)
                .justify_between()
                .center_y()
                .child(page_title(ui.page.name(), paint))
                .child(right),
        );
    if searchable && compact {
        body = body.child(row().child(search_field(ui, paint)));
    }
    if ui.page == Page::Notifications {
        body = body.child(notification_toolbar(ui, paint));
    }
    if let Some(notice) = &ui.notice {
        let tint = if notice.kind == NoticeKind::Error {
            paint.danger
        } else {
            paint.accent
        };
        body = body.child(
            row()
                .key("notice")
                .w_full()
                .padding(8.0)
                .center_y()
                .gap(10.0)
                .radius(8.0)
                .fill(tint.alpha(0.1))
                .child(
                    muted(notice.message.clone(), paint)
                        .grow(1.0)
                        .min_w(0.0)
                        .max_lines(3),
                )
                .child(
                    el().key("dismiss-notice")
                        .size(28.0, 28.0)
                        .center()
                        .radius(6.0)
                        .role("button")
                        .label("Dismiss message")
                        .focusable(true)
                        .hover(paint.hover_wash)
                        .child(
                            text("×")
                                .font(rune::TextStyle::new(18.0, 450.0))
                                .color(paint.subdued),
                        )
                        .on_press(|ui: &mut Ui| ui.dismiss_notice()),
                ),
        );
    }
    let scroll_key = format!("page-scroll-{}", ui.page.name());
    body = body.child(
        ui.wheel_scroll.viewport::<Ui>(
            col()
                .key(scroll_key.clone())
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(content(ui, paint, compact)),
            scroll_key,
            ui.smooth_scrolling && ui.motion_allowed,
        ),
    );
    let shell_body = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(sidebar(ui, paint, compact))
        .child(el().w(1.0).h_full().fill(paint.line.alpha(0.65)))
        .child(body);
    let mut root = col()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(paint.surface)
        .child(caption(paint))
        .child(shell_body);
    if ui.drop_over {
        root = root.outline(2.0, 0.0, paint.accent);
    }
    root.on_drop_files(|ui: &mut Ui, files, _| ui.send_dropped(files))
        .on_drop_hover(|ui: &mut Ui, over, _| ui.drop_over = over)
}

fn on_key(ui: &mut Ui, event: &KeyEvent, _: &mut Tx) -> bool {
    match event.key {
        Key::Escape => {
            ui.confirm = None;
            ui.dismiss_notice();
            true
        }
        Key::Command('1') => {
            ui.navigate(Page::Overview);
            true
        }
        Key::Command('2') => {
            ui.navigate(Page::Transfers);
            true
        }
        Key::Command('3') => {
            ui.navigate(Page::Notifications);
            true
        }
        Key::Command('4') => {
            ui.navigate(Page::Links);
            true
        }
        Key::Command('5') => {
            ui.navigate(Page::Pairing);
            true
        }
        Key::Command('6') => {
            ui.navigate(Page::Settings);
            true
        }
        Key::Command('7') => {
            ui.navigate(Page::About);
            true
        }
        _ => false,
    }
}

fn refresh_native_preferences(ui: &mut Ui, focused: bool, tx: &mut Tx) {
    if focused {
        ui.motion_allowed = window_material::animations_enabled();
        ui.backdrop_active = window_material::apply(ui.material, &ui.appearance);
        #[cfg(windows)]
        let _ = window_icon::update_appearance(&ui.appearance);
        tx.instant();
    }
}

fn setup_native_window(window: &rune::WindowHandle) {
    #[cfg(windows)]
    let _ = window_icon::install(window);
    #[cfg(windows)]
    if let Err(error) = window_chrome::install(window) {
        eprintln!("Conduit native caption: {error}");
    }
    let active = window_material::install(
        window,
        bridge::material_preference(),
        &bridge::theme_preference(),
    );
    if let Some(handle) = HANDLE.get() {
        handle.update(move |ui, tx| {
            ui.backdrop_active = active;
            tx.instant();
        });
    }
}

fn connected_scene(ui: &mut Ui) {
    ui.snap.connection = bridge::Connection {
        state: "Connected".into(),
        device: "Android phone".into(),
        route: "LAN".into(),
        linked: true,
        ..Default::default()
    };
    ui.snap.transfers = vec![bridge::Transfer {
        id: "aabbccddee".into(),
        name: "image.jpg".into(),
        direction: "outbound".into(),
        bytes: 75,
        total: 100,
        state: "transferring".into(),
        terminal: false,
        ..Default::default()
    }];
}

// Deterministic visual fixtures; no real names, history, endpoints or pairings.
fn qa_overview(ui: &mut Ui) {
    ui.snap = bridge::Snapshot::default();
    ui.page = Page::Overview;
    ui.notice = None;
    ui.notification_icons.clear();
    ui.wallpaper = None;
    ui.relay_mode = "default".into();
    ui.relays.replace_text("relay.example.org:41113");
    ui.notification_retention.replace_text("30");
    ui.snap.settings.notification_retention_days = 30;
    ui.expanded_notification = None;
    ui.appearance = "system".into();
    connected_scene(ui);
    ui.snap.connection.device = "Android phone".into();
    ui.snap.daemon_available = true;
    ui.snap.transfers[0].name = "Vacation photos.zip".into();
    ui.snap.transfers[0].bytes = 45 * 1024 * 1024;
    ui.snap.transfers[0].total = 64 * 1024 * 1024;
    ui.snap.transfers.extend([
        bridge::Transfer {
            id: "finished-report".into(),
            name: "Project notes.pdf".into(),
            direction: "outbound".into(),
            bytes: 258048,
            total: 258048,
            state: "completed".into(),
            terminal: true,
            ..Default::default()
        },
        bridge::Transfer {
            id: "received-photo".into(),
            name: "Camera photo.jpg".into(),
            direction: "inbound".into(),
            bytes: 3145728,
            total: 3145728,
            state: "completed".into(),
            terminal: true,
            ..Default::default()
        },
        bridge::Transfer {
            id: "cancelled-archive".into(),
            name: "Archive.zip".into(),
            direction: "outbound".into(),
            bytes: 1024,
            total: 10485760,
            state: "cancelled".into(),
            terminal: true,
            ..Default::default()
        },
    ]);
    ui.snap.transfers.push(bridge::Transfer {
        id: "failed-video".into(),
        name: "Video.mp4".into(),
        direction: "inbound".into(),
        bytes: 1024,
        total: 4 * 1024 * 1024,
        state: "failed".into(),
        detail: "Connection lost".into(),
        terminal: true,
    });
    ui.snap.notifications = vec![bridge::Notification {
        id: "qa-notification".into(),
        timestamp_ms: 1_793_888_400_000,
        package: "example.messages".into(),
        app: "Messages".into(),
        title: "Project update".into(),
        body: "The review notes are ready to read.".into(),
    }];
    ui.snap.links = vec![bridge::SharedLink {
        id: "qa-link".into(),
        title: "Conduit source code".into(),
        url: "https://github.com/nonlog/Conduit".into(),
        source: "Android phone".into(),
    }];
}
fn qa_transfers(ui: &mut Ui) {
    qa_overview(ui);
    ui.page = Page::Transfers;
}
fn qa_notifications(ui: &mut Ui) {
    qa_overview(ui);
    ui.page = Page::Notifications;
    ui.snap.notifications.extend([
        bridge::Notification {
            id: "qa-calendar".into(), timestamp_ms: 1_793_887_200_000,
            package: "example.calendar".into(), app: "Calendar".into(),
            title: "Design review".into(), body: "The team meeting starts in 15 minutes. Room 4 or join online.".into(),
        },
        bridge::Notification {
            id: "qa-mail".into(), timestamp_ms: 1_793_883_600_000,
            package: "example.mail".into(), app: "Mail".into(),
            title: "Weekly project notes".into(), body: "The latest project notes are ready. Review the changes and share your feedback before the next planning session.".into(),
        },
        bridge::Notification {
            id: "qa-message-previous".into(), timestamp_ms: 1_793_802_000_000,
            package: "example.messages".into(), app: "Messages".into(),
            title: "Avery".into(), body: "Thanks for sending the files. Everything arrived safely.".into(),
        },
    ]);
}
fn qa_links(ui: &mut Ui) {
    qa_overview(ui);
    ui.page = Page::Links;
}
fn qa_pairing(ui: &mut Ui) {
    qa_overview(ui);
    ui.page = Page::Pairing;
    ui.snap.pairing = bridge::Pairing {
        active: true,
        paired: true,
        device: "Android phone".into(),
        code: "123456".into(),
    };
    ui.refresh_pairing_qr();
}
fn qa_settings(ui: &mut Ui) {
    qa_overview(ui);
    ui.page = Page::Settings;
    ui.relay_mode = "custom".into();
    ui.snap.settings.relay_mode = "custom".into();
    ui.snap.settings.relays = vec!["relay.example.org:41113".into()];
    ui.snap.settings.effective_relays = ui.snap.settings.relays.clone();
    ui.snap.settings.relays_source = "config".into();
}
fn qa_offline(ui: &mut Ui) {
    qa_overview(ui);
    ui.snap.connection = bridge::Connection::default();
    ui.snap.transfers.clear();
}

fn setup_watcher(handle: AppHandle<Ui>) {
    std::thread::spawn(move || {
        let dir = bridge::data_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };
        // Icons can arrive just after the TSV write on the notifier thread.
        // Observe their real file events too; never fetch or poll artwork.
        if watcher.watch(&dir, RecursiveMode::Recursive).is_err() {
            return;
        }
        for message in take_launch_notices() {
            handle.update(move |ui, _| {
                ui.page = Page::Transfers;
                ui.show_notice(message, NoticeKind::Error);
            });
        }
        while let Ok(event) = rx.recv() {
            let Ok(event) = event else { continue };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                continue;
            }
            if event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("wallpaper.jpg"))
            }) {
                handle.update(|ui, _| ui.refresh_wallpaper());
            }
            if event.paths.iter().any(|path| {
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("icons"))
            }) {
                handle.update(|ui, _| ui.refresh_notification_icons());
            }
            if event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(LAUNCH_NOTICE_PREFIX) && name.ends_with(".txt")
                    })
            }) {
                for message in take_launch_notices() {
                    handle.update(move |ui, _| {
                        ui.page = Page::Transfers;
                        ui.show_notice(message, NoticeKind::Error);
                    });
                }
            }
            if !event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        bridge::OBSERVED_FILES
                            .iter()
                            .any(|key| name.eq_ignore_ascii_case(key))
                    })
            }) {
                continue;
            }
            let next = bridge::snapshot();
            handle.update_if(move |ui, _| {
                if ui.snap == next {
                    return false;
                }
                ui.sync(next);
                true
            });
        }
    });
}

fn initialize_images(ui: &mut Ui, wake: Waker) {
    let cache = ImageCache::new(wake);
    ui.image_cache = Some(cache);
    ui.refresh_wallpaper();
    ui.refresh_pairing_qr();
    ui.refresh_notification_icons();
}

fn poll_ui(ui: &mut Ui, now: f64, tx: &mut Tx) -> bool {
    let Some(notice) = &mut ui.notice else {
        return false;
    };
    let tick = notice.tick(now);
    if let Some(delay) = tick.wake_after {
        tx.wake_after(delay);
    }
    if tick.expired {
        ui.dismiss_notice();
        tx.instant();
        return true;
    }
    false
}

#[cfg(windows)]
fn acquire_instance() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let name = if stable_executable() {
        "Local\\Conduit.Rune.Stable.2026\0"
    } else {
        "Local\\Conduit.Rune.Preview.2026\0"
    };
    let key: Vec<u16> = name.encode_utf16().collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, key.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Some(message) = launch_error() {
            let _ = write_launch_notice(&message);
        }
        let title: Vec<u16> = format!("{}\0", window_title()).encode_utf16().collect();
        let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !window.is_null() {
            unsafe {
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window);
            }
        }
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        return None;
    }
    Some(handle)
}

fn main() {
    let is_headless = std::env::args_os().any(|a| {
        matches!(
            a.to_string_lossy().as_ref(),
            "--snap" | "--tree" | "--script"
        )
    });
    #[cfg(windows)]
    let _instance_guard = if is_headless {
        None
    } else {
        let Some(handle) = acquire_instance() else {
            return;
        };
        Some(handle)
    };

    let app = App::new(AppSpec {
        scenes: &[
            ("connected", connected_scene),
            ("qa-overview", qa_overview),
            ("qa-transfers", qa_transfers),
            ("qa-notifications", qa_notifications),
            ("qa-links", qa_links),
            ("qa-pairing", qa_pairing),
            ("qa-settings", qa_settings),
            ("qa-offline", qa_offline),
        ],
        on_key: Some(on_key),
        setup: Some(initialize_images),
        poll: Some(poll_ui),
        min_size: Some((760.0, 510.0)),
        ..AppSpec::new(window_title(), (1020.0, 605.0), Ui::init, view)
    })
    .window_options(rune::WindowOptions {
        transparent: cfg!(windows),
        ..Default::default()
    })
    .on_focus(refresh_native_preferences)
    .tab_visits_controls(true)
    // Rune's first Windows frame can be initialized before the final DPI-sized
    // client area settles. Nudge its native size once to trigger a real resize
    // event and synchronize the surface + logical viewport without polling.
    .on_window(|window| {
        window.resize_instant((1020.0, 604.0));
        setup_native_window(window);
    })
    // Try again after the first GPU frame if the native HWND was not yet
    // available when Rune first invoked on_window().
    .on_ready(setup_native_window);
    let handle = app.handle();
    let _ = HANDLE.set(handle.clone());
    // Only a user-launched desktop starts or watches the installed daemon. Headless
    // storybook/screenshot tests must not touch an identity or launch a background process.
    if !is_headless {
        setup_watcher(handle.clone());
        std::thread::spawn(move || {
            let error = bridge::ensure_daemon().err();
            let next = bridge::snapshot();
            handle.update(move |ui, _| {
                ui.sync(next);
                if let Some(error) = error {
                    ui.show_notice(error, NoticeKind::Error);
                }
            });
        });
    }
    app.run();
    #[cfg(windows)]
    window_icon::release();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_success_expires_and_navigation_cannot_revive_a_stale_banner() {
        let mut ui = Ui::init();
        qa_overview(&mut ui);
        let old = ui.show_notice("Starting pairing", NoticeKind::Progress);
        ui.navigate(Page::Notifications);
        ui.complete_notice(old, Ok(()), "Starting pairing completed");
        assert!(ui.notice.is_none());
        ui.show_notice("Settings saved", NoticeKind::Success);
        assert!(!poll_ui(&mut ui, 10.0, &mut Tx::default()));
        assert!(!poll_ui(&mut ui, 14.9, &mut Tx::default()));
        assert!(poll_ui(&mut ui, 15.0, &mut Tx::default()));
        assert!(ui.notice.is_none());
    }

    #[test]
    fn notification_cards_have_app_identity_time_and_searchable_content() {
        let mut ui = Ui::init();
        qa_notifications(&mut ui);
        let mut rt = Runtime::new(ui, view, (1020.0, 720.0), Env::new(false, 1.0));
        rt.frame(0.0);
        for key in [
            "notification-toolbar",
            "notification-timeline",
            "notification-icon-qa-notification",
            "notification-time-qa-notification",
        ] {
            assert!(rt.find(key).is_some(), "missing {key}");
        }
        rt.update(|s, tx| {
            s.search.replace_text("Calendar");
            tx.instant();
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("notification-qa-calendar").is_some());
        assert!(rt.find("notification-qa-notification").is_none());
    }

    #[test]
    fn hover_overlays_preserve_readable_notification_text_in_both_themes() {
        for dark in [false, true] {
            for backdrop in [false, true] {
                let paint = Paint::new(dark).with_backdrop(backdrop, 70);
                // Rune composites El::hover on top of the *existing* fill.
                // An opaque or high-alpha hover wash would produce the
                // unreadable pale rectangle seen in the desktop regression.
                assert!(paint.hover_wash.a > 0.0 && paint.hover_wash.a <= 0.06);
                assert!(paint.press_wash.a <= 0.12);
                assert!(paint.line.a <= 0.19);
                assert!(paint.card.a >= 0.80);
                assert_eq!(paint.hover_wash.a, Paint::new(dark).hover_wash.a);
            }
        }
    }

    #[test]
    fn transparency_is_bounded_and_uses_one_background_tint() {
        assert_eq!(transparency_at(8.0, 216.0), 0);
        assert_eq!(transparency_at(108.0, 216.0), 50);
        assert_eq!(transparency_at(208.0, 216.0), 100);
        assert_eq!(transparency_at(999.0, 216.0), 100);
        assert_eq!(transparency_at(f32::NAN, 216.0), 0);
        for dark in [false, true] {
            for percent in [0, 25, 60, 100] {
                let palette = Paint::new(dark).with_backdrop(true, percent);
                assert_eq!(palette.surface, palette.sidebar);
                assert!((palette.surface.a - (1.0 - f32::from(percent) / 100.0)).abs() < 0.001);
                assert_eq!(palette.t.text.a, 1.0);
            }
        }
    }

    #[test]
    fn paused_connection_has_reconnect_without_losing_pairing_controls() {
        let mut ui = Ui::init();
        connected_scene(&mut ui);
        ui.snap.connection.linked = false;
        ui.snap.connection.paused = true;
        ui.snap.pairing.paired = true;
        let mut rt = Runtime::new(ui, view, (760.0, 510.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        assert!(rt.find("connection-toggle").is_some());
        assert!(rt.find("connection-pair").is_some());
        assert!(rt.state.snap.pairing.paired);
        assert!(!rt.state.snap.connection.linked);
    }

    #[test]
    fn native_appearance_controls_expose_real_material_and_motion_preferences() {
        let mut ui = Ui::init();
        ui.page = Page::Settings;
        let mut rt = Runtime::new(ui, view, (1020.0, 605.0), Env::new(true, 1.0));
        rt.frame(0.0);
        for key in [
            "material-acrylic",
            "material-mica",
            "material-solid",
            "smooth-scroll",
        ] {
            assert!(rt.find(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn native_ui_renders_and_navigates_without_daemon() {
        let mut ui = Ui::init();
        connected_scene(&mut ui);
        let mut rt = Runtime::new(ui, view, (1060.0, 750.0), Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("connection-send").is_some());
        for page in Page::ALL {
            assert!(rt.find(&format!("nav-icon-{}", page.name())).is_some());
        }
        let (x, y) = rt.find("nav-Transfers").expect("transfers navigation");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.page, Page::Transfers);
        assert!(rt.find("transfer-aabbccddee").is_some());
    }

    #[test]
    fn workbench_layout_preserves_core_controls_in_both_themes() {
        for dark in [false, true] {
            for size in [(760.0, 510.0), (1020.0, 605.0)] {
                let mut ui = Ui::init();
                connected_scene(&mut ui);
                ui.appearance = if dark { "dark" } else { "light" }.into();
                let mut rt = Runtime::new(ui, view, size, Env::new(dark, 1.0));
                rt.frame(0.0);
                for key in [
                    "sidebar",
                    "sidebar-device",
                    "connection",
                    "connection-route",
                    "connection-send",
                    "overview-summary",
                    "drop-hint",
                    "page-header",
                ] {
                    assert!(rt.find(key).is_some(), "missing {key} in {dark:?} {size:?}");
                }
            }
            let mut ui = Ui::init();
            ui.navigate(Page::Settings);
            ui.appearance = if dark { "dark" } else { "light" }.into();
            let mut rt = Runtime::new(ui, view, (1020.0, 605.0), Env::new(dark, 1.0));
            rt.frame(0.0);
            for key in [
                "settings-appearance",
                "settings",
                "settings-files",
                "save-settings",
                "windows-integration",
            ] {
                assert!(rt.find(key).is_some(), "missing Settings control {key}");
            }
        }
    }

    #[test]
    fn settings_edit_does_not_get_replaced_on_notification_updates() {
        let mut ui = Ui::init();
        ui.settings_dirty = true;
        ui.manual_proxy.replace_text("socks5://manual:1080");
        let mut next = ui.snap.clone();
        next.notifications.push(bridge::Notification {
            id: "1".into(),
            ..Default::default()
        });
        ui.sync(next);
        assert_eq!(ui.manual_proxy.text, "socks5://manual:1080");
    }

    #[test]
    fn shell_send_errors_are_allowlisted_and_bounded() {
        let base = vec![
            "conduit-rune.exe".into(),
            "--send-error".into(),
            "Bad\0file".into(),
        ];
        assert_eq!(send_error_from_args(&base).as_deref(), Some("Badfile"));
        assert!(
            send_error_from_args(&["app.exe".into(), "--unknown".into(), "x".into()]).is_none()
        );
        assert_eq!(
            send_error_from_args(&["app.exe".into(), "--send-error".into(), "x".repeat(5000)])
                .unwrap()
                .len(),
            1024
        );
    }

    #[test]
    fn native_search_and_about_render_headlessly() {
        let mut ui = Ui::init();
        connected_scene(&mut ui);
        ui.navigate(Page::Transfers);
        ui.search.replace_text("no-such-transfer");
        let mut rt = Runtime::new(ui, view, (1020.0, 605.0), Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("search-box").is_some());
        assert!(rt.find("transfer-aabbccddee").is_none());
        let mut about = Ui::init();
        about.navigate(Page::About);
        about.appearance = "light".into();
        let mut about_rt = Runtime::new(about, view, (1020.0, 605.0), Env::new(false, 1.0));
        about_rt.frame(0.0);
        assert!(about_rt.find("about").is_some());
        assert!(about_rt.find("about-github").is_some());
        assert!(about_rt.find("about-nova").is_none());
        assert!(about_rt.find("about-winui").is_none());
    }

    #[test]
    fn native_search_accepts_click_focus_and_keyboard_input() {
        let mut ui = Ui::init();
        connected_scene(&mut ui);
        ui.navigate(Page::Transfers);
        let mut rt = Runtime::new(ui, view, (1020.0, 605.0), Env::new(true, 1.0));
        rt.frame(0.0);
        let (x, y) = rt.find("search-input").expect("search field present");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.key(KeyEvent {
            key: Key::Text("rune".into()),
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.search.text, "rune");
        assert!(rt.find("transfer-aabbccddee").is_none());
    }
    #[test]
    fn overview_keeps_primary_actions_visible_without_dashboard_metrics() {
        for dark in [false, true] {
            for size in [(760.0, 510.0), (1020.0, 605.0), (1280.0, 800.0)] {
                let mut ui = Ui::init();
                connected_scene(&mut ui);
                ui.appearance = if dark { "dark" } else { "light" }.into();
                let mut rt = Runtime::new(ui, view, size, Env::new(dark, 1.0));
                rt.frame(0.0);
                rt.frame(1.0 / 60.0);
                for key in [
                    "connection-send",
                    "connection-pair",
                    "show-transfers",
                    "sidebar-device",
                ] {
                    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
                    assert!(
                        x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1,
                        "{key} is clipped at {size:?}: {x}, {y}"
                    );
                }
                assert!(rt.find("metric-Transfers").is_none());
            }
        }
    }

    #[test]
    fn readable_transfer_sizes_cover_empty_and_large_files() {
        assert_eq!(byte_size(0), "0 B");
        assert_eq!(byte_size(1024), "1.0 KB");
        assert_eq!(byte_size(1024 * 1024), "1.0 MB");
        assert_eq!(byte_size(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
    #[test]
    fn paired_active_qr_and_cancel_fit_in_the_smallest_window() {
        let mut ui = Ui::init();
        qa_pairing(&mut ui);
        let mut rt = Runtime::new(ui, view, (760.0, 510.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        for key in ["pairing-qr", "cancel-pair"] {
            let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
            assert!(
                x > 0.0 && x < 760.0 && y > 0.0 && y < 510.0,
                "clipped {key}"
            );
        }
        assert!(rt.find("forget-pair").is_none());
    }
    #[test]
    fn native_transfer_filters_are_interactive_and_combine_with_search() {
        let mut ui = Ui::init();
        qa_transfers(&mut ui);
        let mut rt = Runtime::new(ui, view, (1020.0, 605.0), Env::new(true, 1.0));
        rt.frame(0.0);
        let (x, y) = rt.find("filter-failed").expect("real failed filter");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
        assert!(rt.find("transfer-failed-video").is_some());
        assert!(rt.find("transfer-finished-report").is_none());
        rt.update(|ui, tx| {
            ui.search.replace_text("Video");
            tx.instant();
        });
        rt.frame(0.2);
        assert!(rt.find("transfer-failed-video").is_some());
        rt.update(|ui, tx| {
            ui.search.replace_text("nonexistent");
            tx.instant();
        });
        rt.frame(0.3);
        assert!(rt.find("transfer-failed-video").is_none());
    }

    #[test]
    fn cancelled_transfers_are_not_mislabelled_as_failed() {
        let t = bridge::Transfer {
            state: "cancelled".into(),
            terminal: true,
            ..Default::default()
        };
        assert!(TransferFilter::All.accepts(&t));
        assert!(!TransferFilter::Failed.accepts(&t));
        assert!(!TransferFilter::Active.accepts(&t));
        assert_eq!(file_glyph("PHOTO.JPG").name, icons::FILE_IMAGE.name);
        assert_eq!(file_glyph("song.mp3").name, icons::FILE_AUDIO.name);
    }
}
