#![cfg_attr(windows, windows_subsystem = "windows")]

mod bridge;
mod icons;
#[cfg(windows)]
mod window_icon;

use notify::{RecursiveMode, Watcher};
use rune::tok::{self, Theme, type_};
use rune::{
    App, AppHandle, AppSpec, Color, Cx, El, Env, Icon, Image, ImageCache, Key, KeyEvent, Runtime,
    ScrollSpec, SurfaceFit, TextEdit, Tx, Waker, col, el, icon, row, text, text_input,
};
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

    fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => "Your device, activity, and recent transfers.",
            Page::Transfers => "Send files securely and manage transfer progress.",
            Page::Notifications => "Updates forwarded from your connected phone.",
            Page::Links => "Links shared between your devices.",
            Page::Pairing => "Manage your encrypted phone connection.",
            Page::Settings => "Connection, appearance, and Windows integration.",
            Page::About => "Conduit version and system information.",
        }
    }
}

struct Ui {
    page: Page,
    snap: bridge::Snapshot,
    image_cache: Option<ImageCache>,
    wallpaper: Option<Image>,
    brand_image: Option<Image>,
    search: TextEdit,
    appearance: String,
    relays: TextEdit,
    manual_proxy: TextEdit,
    receive_dir: TextEdit,
    proxy_mode: String,
    tray_icon: bool,
    settings_dirty: bool,
    confirm: Option<&'static str>,
    notice: Option<String>,
    busy: bool,
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
            appearance: bridge::theme_preference(),
            image_cache: None,
            wallpaper: None,
            brand_image: None,
            relays: TextEdit::new(snap.settings.relays.join("\n")),
            manual_proxy: TextEdit::new(snap.settings.manual_proxy.clone()),
            receive_dir: TextEdit::new(snap.settings.receive_dir.clone()),
            proxy_mode: snap.settings.proxy_mode.clone(),
            tray_icon: snap.settings.tray_icon,
            snap,
            settings_dirty: false,
            confirm: None,
            notice,
            busy: false,
            drop_over: false,
        }
    }

    fn sync(&mut self, next: bridge::Snapshot) {
        if self.snap == next {
            return;
        }
        if !self.settings_dirty {
            self.relays.replace_text(next.settings.relays.join("\n"));
            self.manual_proxy
                .replace_text(next.settings.manual_proxy.clone());
            self.receive_dir
                .replace_text(next.settings.receive_dir.clone());
            self.proxy_mode = next.settings.proxy_mode.clone();
            self.tray_icon = next.settings.tray_icon;
        }
        self.snap = next;
    }

    fn refresh_wallpaper(&mut self) {
        if let Some(cache) = &self.image_cache {
            self.wallpaper = bridge::wallpaper_bytes().map(|bytes| cache.load(bytes));
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
        self.notice = Some(format!("{message}…"));
        let handle = HANDLE.get().cloned();
        std::thread::spawn(move || {
            let result = task();
            let next = bridge::snapshot();
            if let Some(handle) = handle {
                handle.update(move |ui, _| {
                    ui.sync(next);
                    ui.busy = false;
                    ui.notice = Some(match result {
                        Ok(()) => format!("{message} completed"),
                        Err(error) => error,
                    });
                });
            }
        });
    }

    fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.search.replace_text("");
        }
        self.page = page;
        self.confirm = None;
    }

    fn save_settings(&mut self) {
        if self.busy {
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
            proxy_mode: self.proxy_mode.clone(),
            manual_proxy: self.manual_proxy.text.clone(),
            receive_dir: self.receive_dir.text.clone(),
            tray_icon: self.tray_icon,
            ..self.snap.settings.clone()
        };
        self.busy = true;
        self.notice = Some("Saving settings…".into());
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
                    ui.notice = Some(match result {
                        Ok(()) => "Settings saved".into(),
                        Err(error) => error,
                    });
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
                        ui.notice = Some("Receive folder selected; save settings to apply".into());
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
    surface: Color,
    sidebar: Color,
    card: Color,
    inset: Color,
    line: Color,
    subdued: Color,
    faint: Color,
    green: Color,
    purple: Color,
    selected: Color,
}

impl Paint {
    fn new(dark: bool) -> Self {
        let color = |hex| rune::color::hex(hex).expect("valid Conduit design token");
        if dark {
            Paint {
                t: tok::theme(true),
                accent: color("#6b9eff"),
                surface: color("#10151d"),
                sidebar: color("#10151d"),
                card: color("#19212b"),
                inset: color("#121923"),
                line: color("#2a3542"),
                subdued: color("#a4b1c0"),
                faint: color("#79899b"),
                green: color("#61d8aa"),
                purple: color("#a4b1c0"),
                selected: color("#1c2a3b"),
            }
        } else {
            Paint {
                t: tok::theme(false),
                accent: color("#266de4"),
                surface: color("#f5f7fa"),
                sidebar: color("#ffffff"),
                card: color("#ffffff"),
                inset: color("#f3f6fa"),
                line: color("#e0e6ee"),
                subdued: color("#59697d"),
                faint: color("#7c8a9b"),
                green: color("#16865d"),
                purple: color("#59697d"),
                selected: color("#eaf1ff"),
            }
        }
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
        .h(40.0)
        .px(16.0)
        .gap(9.0)
        .center()
        .radius(9.0)
        .continuous_corners()
        .fill(if primary { paint.accent } else { paint.card })
        .border(if primary { 0.0 } else { 1.0 }, paint.line)
        .hover(if primary {
            paint.accent.mix(Color::WHITE, 0.12)
        } else {
            paint.t.hover
        })
        .press(paint.t.press)
        .on_press(f);
    if let Some(glyph) = glyph {
        content = content.child(icon(glyph, 17.0).color(if primary {
            Color::WHITE
        } else {
            paint.t.text
        }));
    }
    content.child(text(title).font(type_::LABEL).color(if primary {
        Color::WHITE
    } else {
        paint.t.text
    }))
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
        .hover(paint.selected)
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
        .padding(20.0)
        .gap(16.0)
        .radius(13.0)
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
        .font(rune::TextStyle::new(22.0, 700.0))
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
    let label = page.name();
    row()
        .key(format!("nav-{label}"))
        .role("button")
        .label(label)
        .w_full()
        .h(40.0)
        .px(11.0)
        .gap(11.0)
        .center_y()
        .radius(8.0)
        .continuous_corners()
        .fill(if selected {
            paint.selected
        } else {
            Color::TRANSPARENT
        })
        .hover(if selected {
            paint.selected
        } else {
            paint.t.hover
        })
        .on_press(move |ui: &mut Ui| ui.navigate(page))
        .child(
            icon(page.glyph(), 18.0)
                .key(format!("nav-icon-{label}"))
                .color(if selected {
                    paint.accent
                } else {
                    paint.subdued
                }),
        )
        .child(
            text(label)
                .font(rune::TextStyle::new(
                    14.0,
                    if selected { 600.0 } else { 450.0 },
                ))
                .color(if selected {
                    paint.t.text
                } else {
                    paint.subdued
                }),
        )
}

fn sidebar(ui: &Ui, paint: Paint) -> El {
    let mut menu = col()
        .key("side-navigation")
        .w_full()
        .gap(4.0)
        .child(eyebrow("WORKSPACE", paint));
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
        .child(eyebrow("DEVICE & APP", paint));
    for page in [Page::Pairing, Page::Settings, Page::About] {
        menu = menu.child(nav(page, ui.page == page, paint));
    }
    let logo = if let Some(image) = &ui.brand_image {
        el().key("conduit-brand-icon")
            .size(36.0, 36.0)
            .radius(10.0)
            .gradient(paint.accent, paint.accent.mix(paint.purple, 0.28))
            .pixels_fit(image.pixels(), SurfaceFit::Fit)
    } else {
        el().key("conduit-brand-icon")
            .size(36.0, 36.0)
            .radius(10.0)
            .gradient(paint.accent, paint.accent.mix(paint.purple, 0.25))
            .center()
            .child(icon(&icons::ARROWS_LEFT_RIGHT, 21.0).color(Color::WHITE))
    };
    col()
        .key("sidebar")
        .w(214.0)
        .h_full()
        .padding(16.0)
        .gap(20.0)
        .fill(paint.sidebar)
        .border(1.0, paint.line.alpha(0.65))
        .child(
            row()
                .key("brand")
                .h(54.0)
                .gap(11.0)
                .center_y()
                .child(logo)
                .child(
                    text("Conduit")
                        .font(rune::TextStyle::new(17.0, 680.0))
                        .color(paint.t.text),
                ),
        )
        .child(menu)
        .child(el().grow(1.0))
        .child(
            col()
                .gap(8.0)
                .child(el().w_full().h(1.0).fill(paint.line))
                .child(
                    row()
                        .key("sidebar-device")
                        .role("button")
                        .label("Manage connected device")
                        .w_full()
                        .h(46.0)
                        .gap(9.0)
                        .center_y()
                        .on_press(|ui: &mut Ui| ui.navigate(Page::Pairing))
                        .child(
                            el().size(28.0, 28.0)
                                .radius(8.0)
                                .fill(paint.selected)
                                .center()
                                .child(icon(&icons::SMARTPHONE, 16.0).color(paint.accent)),
                        )
                        .child(
                            col()
                                .min_w(0.0)
                                .gap(3.0)
                                .child(
                                    text(if ui.snap.connection.linked {
                                        "Phone connected"
                                    } else {
                                        "No device"
                                    })
                                    .font(rune::TextStyle::new(12.0, 620.0))
                                    .color(paint.t.text),
                                )
                                .child(
                                    text(if ui.snap.connection.linked {
                                        "Noise encrypted"
                                    } else {
                                        "Connect a phone"
                                    })
                                    .font(rune::TextStyle::new(11.0, 450.0))
                                    .color(paint.faint),
                                ),
                        ),
                ),
        )
}

fn summary_connection(ui: &Ui, paint: Paint) -> El {
    let conn = &ui.snap.connection;
    let transport = if !conn.linked {
        "Waiting for a connection"
    } else if conn.route.to_ascii_lowercase().contains("lan") {
        "Direct LAN connection"
    } else if conn.route.to_ascii_lowercase().contains("relay") {
        "Secure relay connection"
    } else {
        "Secure device connection"
    };
    let endpoint = if conn.relay.is_empty() {
        conn.route.clone()
    } else if conn.relay.chars().count() > 34 {
        format!("{}…", conn.relay.chars().take(33).collect::<String>())
    } else {
        conn.relay.clone()
    };
    let phone = if let Some(image) = &ui.wallpaper {
        el().key("phone-wallpaper")
            .size(84.0, 104.0)
            .radius(13.0)
            .pixels_fit(image.pixels(), SurfaceFit::Fill)
    } else {
        el().key("phone-placeholder")
            .size(84.0, 104.0)
            .radius(13.0)
            .fill(paint.inset)
            .center()
            .child(icon(&icons::SMARTPHONE, 31.0).color(paint.accent))
    };
    card(
        "connection",
        paint,
        col()
            .gap(17.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(eyebrow("CONNECTED DEVICE", paint))
                    .child(tag(conn.state.clone(), paint, conn.linked)),
            )
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(17.0)
                    .child(phone)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(9.0)
                            .child(
                                text(if conn.device.is_empty() {
                                    "Your Android phone"
                                } else {
                                    conn.device.as_str()
                                })
                                .font(rune::TextStyle::new(24.0, 680.0))
                                .color(paint.t.text),
                            )
                            .child(
                                text(if conn.linked {
                                    "Ready to exchange files"
                                } else {
                                    "Connect your phone to get started"
                                })
                                .font(rune::TextStyle::new(14.0, 450.0))
                                .color(paint.subdued),
                            )
                            .child(
                                row()
                                    .center_y()
                                    .gap(7.0)
                                    .child(icon(&icons::SHIELD_CHECK, 15.0).color(paint.green))
                                    .child(
                                        text("End-to-end encrypted with Noise")
                                            .font(rune::TextStyle::new(12.0, 500.0))
                                            .color(paint.subdued),
                                    ),
                            ),
                    )
                    .child(
                        col()
                            .gap(9.0)
                            .child(action_button(
                                "connection-send",
                                "Send file",
                                Some(&icons::SEND),
                                paint,
                                true,
                                |ui| ui.open_picker(),
                            ))
                            .child(action_button(
                                "connection-pair",
                                "Manage pairing",
                                None,
                                paint,
                                false,
                                |ui| ui.navigate(Page::Pairing),
                            )),
                    ),
            )
            .child(
                row()
                    .key("connection-route")
                    .w_full()
                    .h(42.0)
                    .px(13.0)
                    .gap(10.0)
                    .center_y()
                    .radius(8.0)
                    .fill(paint.inset)
                    .child(icon(&icons::ARROWS_LEFT_RIGHT, 15.0).color(paint.accent))
                    .child(
                        text(transport)
                            .font(rune::TextStyle::new(12.0, 600.0))
                            .color(paint.t.text),
                    )
                    .child(el().grow(1.0))
                    .child(
                        text(endpoint)
                            .font(rune::TextStyle::new(11.0, 450.0))
                            .color(paint.faint),
                    ),
            ),
    )
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
    let name = if tr.name.chars().count() > 46 {
        format!("{}…", tr.name.chars().take(45).collect::<String>())
    } else {
        tr.name.clone()
    };
    let mut transfer_details = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(5.0)
        .child(
            text(name)
                .font(rune::TextStyle::new(13.0, 570.0))
                .color(paint.t.text),
        )
        .child(
            text(format!("{}  ·  {}%", tr.direction, progress))
                .font(rune::TextStyle::new(11.0, 450.0))
                .color(paint.faint),
        );
    if !tr.terminal {
        transfer_details = transfer_details.child(
            row()
                .key(format!("progress-{}", tr.id))
                .w(210.0)
                .h(4.0)
                .radius(2.0)
                .fill(paint.line)
                .child(
                    el().w(progress as f32 * 2.1)
                        .h(4.0)
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
        .h(if tr.terminal { 58.0 } else { 68.0 })
        .px(12.0)
        .radius(8.0)
        .hover(paint.t.hover)
        .child(
            el().size(34.0, 34.0)
                .radius(8.0)
                .fill(paint.inset)
                .center()
                .child(icon(&icons::TRANSFER, 17.0).color(paint.subdued)),
        )
        .child(transfer_details);
    if !tr.terminal {
        let id = tr.id.clone();
        body = body.child(button(
            format!("cancel-{}", tr.id),
            "Cancel",
            paint,
            false,
            move |ui| {
                let id = id.clone();
                ui.operation("Cancelling transfer", move || bridge::cancel_transfer(&id));
            },
        ));
    } else {
        body = body
            .on_press(|ui: &mut Ui| ui.navigate(Page::Transfers))
            .child(
                text(match tr.state.as_str() {
                    "completed" => "Completed",
                    "failed" => "Failed",
                    "cancelled" => "Cancelled",
                    _ => "Finished",
                })
                .font(rune::TextStyle::new(11.0, 570.0))
                .color(if tr.state == "completed" {
                    paint.green
                } else {
                    paint.subdued
                }),
            )
            .child(icon(&icons::CHEVRON_RIGHT, 17.0).color(paint.subdued));
    }
    body
}

fn empty(message: &str, paint: Paint) -> El {
    col()
        .w_full()
        .h(135.0)
        .center()
        .child(muted(message, paint))
}

fn transfer_list(ui: &Ui, paint: Paint, limit: usize) -> El {
    let mut list = col().key("transfer-list").w_full().gap(1.0);
    let query = ui.search.text.trim().to_lowercase();
    let records = ui
        .snap
        .transfers
        .iter()
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
        list = list.child(empty(
            if query.is_empty() {
                "No transfers yet"
            } else {
                "No matching transfers"
            },
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
    card(
        "overview-summary",
        paint,
        col()
            .gap(15.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(section_heading(
                        &icons::TRANSFER,
                        "Recent transfers",
                        "Your latest files, at a glance",
                        paint,
                    ))
                    .child(action_button(
                        "show-transfers",
                        "See all",
                        Some(&icons::CHEVRON_RIGHT),
                        paint,
                        false,
                        |ui| ui.navigate(Page::Transfers),
                    )),
            )
            .child(
                row()
                    .gap(9.0)
                    .child(stat("Transfers", ui.snap.transfers.len(), paint))
                    .child(stat("Notifications", ui.snap.notifications.len(), paint))
                    .child(stat("Shared links", ui.snap.links.len(), paint)),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(transfer_list(ui, paint, 4)),
    )
}

fn stat(label: &str, count: usize, paint: Paint) -> El {
    let (glyph, page): (&'static Icon, Page) = match label {
        "Notifications" => (&icons::BELL, Page::Notifications),
        "Shared links" => (&icons::LINK, Page::Links),
        _ => (&icons::TRANSFER, Page::Transfers),
    };
    row()
        .key(format!("metric-{}", label))
        .role("button")
        .label(format!("{count} {label}"))
        .center_y()
        .gap(8.0)
        .px(11.0)
        .h(34.0)
        .radius(7.0)
        .fill(paint.inset)
        .hover(paint.selected)
        .on_press(move |ui: &mut Ui| ui.navigate(page))
        .child(icon(glyph, 15.0).color(paint.subdued))
        .child(
            text(format!("{count}  {label}"))
                .font(rune::TextStyle::new(11.0, 580.0))
                .color(paint.subdued),
        )
}

fn transfers(ui: &Ui, paint: Paint) -> El {
    card(
        "transfers",
        paint,
        col()
            .gap(16.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(section_heading(
                        &icons::TRANSFER,
                        "Transfer history",
                        "Send, monitor and cancel transfers",
                        paint,
                    ))
                    .child(action_button(
                        "send-new-file",
                        "Send file",
                        Some(&icons::SEND),
                        paint,
                        true,
                        |ui| ui.open_picker(),
                    )),
            )
            .child(
                row()
                    .h(34.0)
                    .gap(8.0)
                    .child(stat("Transfers", ui.snap.transfers.len(), paint))
                    .child(muted("Drag files into this window to send them", paint)),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(transfer_list(ui, paint, usize::MAX)),
    )
}

fn notifications(ui: &Ui, paint: Paint) -> El {
    let mut list = col().gap(3.0);
    let query = ui.search.text.trim().to_lowercase();
    let records = ui
        .snap
        .notifications
        .iter()
        .filter(|notification| {
            query.is_empty()
                || [
                    notification.app.as_str(),
                    notification.title.as_str(),
                    notification.body.as_str(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        list = list.child(empty(
            if query.is_empty() {
                "No notifications"
            } else {
                "No matching notifications"
            },
            paint,
        ));
    } else {
        for notification in records {
            let title = if notification.title.chars().count() > 70 {
                format!(
                    "{}…",
                    notification.title.chars().take(69).collect::<String>()
                )
            } else {
                notification.title.clone()
            };
            let body = if notification.body.chars().count() > 132 {
                format!(
                    "{}…",
                    notification.body.chars().take(131).collect::<String>()
                )
            } else {
                notification.body.clone()
            };
            list = list.child(
                row()
                    .key(format!("notification-{}", notification.id))
                    .w_full()
                    .padding(12.0)
                    .center_y()
                    .gap(13.0)
                    .radius(9.0)
                    .hover(paint.t.hover)
                    .child(
                        el().size(36.0, 36.0)
                            .radius(9.0)
                            .fill(paint.inset)
                            .center()
                            .child(icon(&icons::BELL, 18.0).color(paint.subdued)),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(5.0)
                            .child(
                                row()
                                    .center_y()
                                    .gap(10.0)
                                    .child(
                                        text(title)
                                            .font(rune::TextStyle::new(13.0, 590.0))
                                            .color(paint.t.text),
                                    )
                                    .child(
                                        text(notification.app.as_str())
                                            .font(rune::TextStyle::new(11.0, 450.0))
                                            .color(paint.accent),
                                    ),
                            )
                            .child(
                                text(body)
                                    .font(rune::TextStyle::new(12.0, 440.0))
                                    .color(paint.subdued),
                            ),
                    ),
            );
        }
    }
    card(
        "notifications",
        paint,
        col()
            .gap(14.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(section_heading(
                        &icons::BELL,
                        "Phone notifications",
                        "Recent updates from Android",
                        paint,
                    ))
                    .child(button(
                        "clear-notifications",
                        if ui.confirm == Some("notifications") {
                            "Confirm clear"
                        } else {
                            "Clear"
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
                    )),
            )
            .child(el().w_full().h(1.0).fill(paint.line))
            .child(list),
    )
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
        list = list.child(empty(
            if query.is_empty() {
                "No links shared from the phone"
            } else {
                "No matching links"
            },
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
                    .hover(paint.t.hover)
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
                                    .color(paint.t.text),
                            )
                            .child(muted(format!("{} · {}", item.source, preview), paint)),
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
    let mut content = col().gap(18.0).child(
        row()
            .justify_between()
            .center_y()
            .child(section_heading(
                &icons::QR_CODE,
                "Phone pairing",
                "Secure device identity",
                paint,
            ))
            .child(tag(
                if p.paired { "Paired" } else { "Not paired" },
                paint,
                p.paired,
            )),
    );
    if p.paired {
        content = content
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(14.0)
                    .padding(15.0)
                    .radius(10.0)
                    .fill(paint.inset)
                    .child(
                        el().size(42.0, 42.0)
                            .radius(10.0)
                            .fill(paint.selected)
                            .center()
                            .child(icon(&icons::SMARTPHONE, 24.0).color(paint.accent)),
                    )
                    .child(
                        col()
                            .gap(5.0)
                            .child(
                                text(&p.device)
                                    .font(rune::TextStyle::new(17.0, 650.0))
                                    .color(paint.t.text),
                            )
                            .child(muted("Trusted phone · Noise pairing identity", paint)),
                    ),
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
                        });
                    }
                },
            ));
    } else if p.active {
        content = content
            .child(muted("Enter this pairing code on your phone", paint))
            .child(
                row()
                    .w_full()
                    .h(86.0)
                    .radius(10.0)
                    .fill(paint.inset)
                    .center()
                    .child(
                        text(&p.code)
                            .font(rune::TextStyle::new(27.0, 700.0))
                            .color(paint.accent),
                    ),
            )
            .child(button(
                "cancel-pair",
                "Cancel pairing",
                paint,
                false,
                |ui| {
                    ui.operation("Cancelling pairing", || {
                        bridge::daemon_command(&["pair", "cancel"]).map(|_| ())
                    })
                },
            ));
    } else {
        content = content
            .child(muted(
                "Pair your Android phone to establish an encrypted connection.",
                paint,
            ))
            .child(action_button(
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
            ));
    }
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
    col()
        .gap(7.0)
        .w_full()
        .child(
            text("Relay nodes (one host:port per line)")
                .font(type_::LABEL)
                .color(paint.subdued),
        )
        .child(
            col()
                .w_full()
                .min_h(98.0)
                .padding(10.0)
                .fill(paint.inset)
                .radius(8.0)
                .border(1.0, paint.line)
                .child(
                    text_input(&ui.relays, type_::BODY)
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
                Ok(()) => ui.appearance = mode.into(),
                Err(error) => ui.notice = Some(error),
            },
        ));
    }
    let appearance_card = card(
        "settings-appearance",
        paint,
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
                    .child(button(
                        "toggle-tray",
                        if ui.tray_icon { "Enabled" } else { "Disabled" },
                        paint,
                        ui.tray_icon,
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
                    .child(button(
                        "autostart",
                        if ui.snap.settings.autostart {
                            "Enabled"
                        } else {
                            "Disabled"
                        },
                        paint,
                        ui.snap.settings.autostart,
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
                    .child(button(
                        "explorer",
                        if ui.snap.settings.explorer {
                            "Enabled"
                        } else {
                            "Disabled"
                        },
                        paint,
                        ui.snap.settings.explorer,
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
            .child(eyebrow("TOOLS & FALLBACKS", paint))
            .child(
                row()
                    .gap(8.0)
                    .child(button(
                        "data-folder",
                        "Open data folder",
                        paint,
                        false,
                        |ui| ui.operation("Opening data folder", || bridge::open_folder("data")),
                    ))
                    .child(button("nova-window", "Open Nova", paint, false, |ui| {
                        ui.operation("Opening Nova", bridge::open_nova_ui)
                    }))
                    .child(button("legacy-window", "Open WinUI", paint, false, |ui| {
                        ui.operation("Opening WinUI", bridge::open_legacy_ui)
                    })),
            ),
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
                            .child(icon(&icons::ARROWS_LEFT_RIGHT, 27.0).color(paint.accent)),
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
            .child(muted(
                "Securely transfer files, notifications and links between Windows and Android.",
                paint,
            ))
            .child(tag(
                if ui.snap.daemon_available {
                    "Background service available"
                } else {
                    "Background service unavailable"
                },
                paint,
                ui.snap.daemon_available,
            ))
            .child(muted(
                format!("Data directory: {}", bridge::data_dir().display()),
                paint,
            ))
            .child(eyebrow("PROJECT & ALTERNATIVE DESKTOPS", paint))
            .child(
                row()
                    .gap(8.0)
                    .child(action_button(
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
                    ))
                    .child(button("about-nova", "Open Nova", paint, false, |ui| {
                        ui.operation("Opening Nova", bridge::open_nova_ui)
                    }))
                    .child(button("about-winui", "Open WinUI", paint, false, |ui| {
                        ui.operation("Opening WinUI", bridge::open_legacy_ui)
                    })),
            ),
    )
}

fn content(ui: &Ui, paint: Paint) -> El {
    let mut body = col().key("page-content").gap(16.0);
    if ui.page == Page::Overview {
        body = body
            .child(summary_connection(ui, paint))
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

fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let dark = match ui.appearance.as_str() {
        "light" => false,
        "dark" => true,
        _ => cx.system_dark(),
    };
    let paint = Paint::new(dark);
    let state = if ui.snap.connection.linked {
        "Connected"
    } else {
        "Offline"
    };
    let mut right = row().gap(12.0).center_y();
    if matches!(ui.page, Page::Transfers | Page::Notifications | Page::Links) {
        right = right.child(search_field(ui, paint));
    }
    if ui.page == Page::Overview {
        right = right
            .child(icon(&icons::SHIELD_CHECK, 16.0).color(paint.green))
            .child(
                text("Private by design")
                    .font(rune::TextStyle::new(12.0, 570.0))
                    .color(paint.subdued),
            );
    } else {
        right = right.child(tag(state, paint, ui.snap.connection.linked));
    }
    let mut body = col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(26.0)
        .gap(19.0)
        .child(
            row()
                .key("page-header")
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    col()
                        .gap(6.0)
                        .child(eyebrow(
                            if ui.page == Page::Overview {
                                "DEVICE BRIDGE"
                            } else {
                                "CONDUIT / WORKSPACE"
                            },
                            paint,
                        ))
                        .child(page_title(
                            if ui.page == Page::Overview {
                                "Your devices"
                            } else {
                                ui.page.name()
                            },
                            paint,
                        ))
                        .child(muted(ui.page.subtitle(), paint)),
                )
                .child(right),
        );
    if let Some(notice) = &ui.notice {
        body = body.child(
            row()
                .key("notice")
                .w_full()
                .padding(10.0)
                .radius(8.0)
                .fill(paint.accent.alpha(0.1))
                .child(muted(notice.clone(), paint)),
        );
    }
    body = body.child(
        content(ui, paint)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
    );
    let mut root = row()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(paint.surface)
        .child(sidebar(ui, paint))
        .child(body);
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
            ui.notice = None;
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
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        for message in take_launch_notices() {
            handle.update(move |ui, _| {
                ui.page = Page::Transfers;
                ui.notice = Some(message);
            });
        }
        while let Ok(event) = rx.recv() {
            let Ok(event) = event else { continue };
            if event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("wallpaper.jpg"))
            }) {
                handle.update(|ui, _| ui.refresh_wallpaper());
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
                        ui.notice = Some(message);
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
    // Render the actual Conduit app mark, not a placeholder character or an
    // icon-font codepoint. The PNG ships in the repository and needs no I/O.
    ui.brand_image = Some(
        cache.load(include_bytes!("../../conduit-daemon/assets/conduit-icon-dark.png").to_vec()),
    );
    ui.image_cache = Some(cache);
    ui.refresh_wallpaper();
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
        scenes: &[("connected", connected_scene)],
        on_key: Some(on_key),
        setup: Some(initialize_images),
        min_size: Some((760.0, 510.0)),
        ..AppSpec::new(window_title(), (1020.0, 605.0), Ui::init, view)
    })
    .tab_visits_controls(true)
    // Rune's first Windows frame can be initialized before the final DPI-sized
    // client area settles. Nudge its native size once to trigger a real resize
    // event and synchronize the surface + logical viewport without polling.
    .on_window(|window| {
        window.resize_instant((1020.0, 604.0));
        #[cfg(windows)]
        let _ = window_icon::install(window);
    })
    // Try again after the first GPU frame if the native HWND was not yet
    // available when Rune first invoked on_window().
    .on_ready(|window| {
        #[cfg(windows)]
        let _ = window_icon::install(window);
        #[cfg(not(windows))]
        let _ = window;
    });
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
                    ui.notice = Some(error);
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
                    "metric-Transfers",
                    "metric-Notifications",
                    "metric-Shared links",
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
        assert!(about_rt.find("about-nova").is_some());
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
}
