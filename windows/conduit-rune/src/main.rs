#![cfg_attr(windows, windows_subsystem = "windows")]

mod bridge;

use notify::{RecursiveMode, Watcher};
use rune::tok::{self, Theme, type_};
use rune::{
    App, AppHandle, AppSpec, Cx, El, Env, Key, KeyEvent, Runtime, ScrollSpec, TextEdit, Tx, col,
    el, row, text, text_input,
};
use std::path::PathBuf;
use std::sync::OnceLock;

const WINDOW_TITLE: &str = "Conduit Rune Preview";
static HANDLE: OnceLock<AppHandle<Ui>> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Overview,
    Transfers,
    Notifications,
    Links,
    Pairing,
    Settings,
}

impl Page {
    const ALL: [Page; 6] = [
        Page::Overview,
        Page::Transfers,
        Page::Notifications,
        Page::Links,
        Page::Pairing,
        Page::Settings,
    ];

    fn name(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Transfers => "Transfers",
            Page::Notifications => "Notifications",
            Page::Links => "Shared links",
            Page::Pairing => "Pairing",
            Page::Settings => "Settings",
        }
    }
}

struct Ui {
    page: Page,
    snap: bridge::Snapshot,
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
        Ui {
            page: Page::Overview,
            relays: TextEdit::new(snap.settings.relays.join("\n")),
            manual_proxy: TextEdit::new(snap.settings.manual_proxy.clone()),
            receive_dir: TextEdit::new(snap.settings.receive_dir.clone()),
            proxy_mode: snap.settings.proxy_mode.clone(),
            tray_icon: snap.settings.tray_icon,
            snap,
            settings_dirty: false,
            confirm: None,
            notice: None,
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

    fn operation(
        &mut self,
        message: &'static str,
        task: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
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
        self.page = page;
        self.confirm = None;
    }

    fn save_settings(&mut self) {
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
        self.settings_dirty = false;
        self.operation("Saving settings", move || bridge::save_settings(settings));
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
    accent: rune::Color,
}

fn button(
    key: impl Into<String>,
    label: impl Into<String>,
    paint: Paint,
    primary: bool,
    f: impl Fn(&mut Ui) + 'static,
) -> El {
    let title = label.into();
    row()
        .key(key.into())
        .role("button")
        .label(title.clone())
        .h(34.0)
        .px(13.0)
        .center()
        .radius(8.0)
        .fill(if primary { paint.accent } else { paint.t.card })
        .border(if primary { 0.0 } else { 1.0 }, paint.t.card_border)
        .hover(if primary {
            paint.accent.alpha(0.8)
        } else {
            paint.t.hover
        })
        .press(paint.t.press)
        .on_press(f)
        .child(text(title).font(type_::LABEL).color(if primary {
            rune::Color::WHITE
        } else {
            paint.t.text
        }))
}

fn tag(label: impl Into<String>, paint: Paint, good: bool) -> El {
    row()
        .radius(7.0)
        .px(10.0)
        .h(25.0)
        .center()
        .fill(if good {
            paint.accent.alpha(0.13)
        } else {
            paint.t.control_off
        })
        .child(text(label.into()).font(type_::LABEL).color(if good {
            paint.accent
        } else {
            paint.t.text_secondary
        }))
}

fn card(key: impl Into<String>, paint: Paint, child: El) -> El {
    col()
        .key(key.into())
        .w_full()
        .padding(18.0)
        .gap(14.0)
        .radius(12.0)
        .fill(paint.t.card)
        .border(1.0, paint.t.card_border)
        .child(child)
}

fn heading(label: &str, paint: Paint) -> El {
    text(label).font(type_::TITLE).color(paint.t.text)
}

fn muted(label: impl Into<String>, paint: Paint) -> El {
    text(label.into())
        .font(type_::BODY)
        .color(paint.t.text_secondary)
}

fn nav(page: Page, selected: bool, paint: Paint) -> El {
    let label = page.name();
    row()
        .key(format!("nav-{label}"))
        .role("button")
        .label(label)
        .w_full()
        .h(40.0)
        .px(14.0)
        .gap(10.0)
        .center_y()
        .radius(9.0)
        .fill(if selected {
            paint.accent.alpha(0.14)
        } else {
            rune::Color::TRANSPARENT
        })
        .hover(paint.t.hover)
        .on_press(move |ui: &mut Ui| ui.navigate(page))
        .child(el().size(5.0, 18.0).radius(2.0).fill(if selected {
            paint.accent
        } else {
            rune::Color::TRANSPARENT
        }))
        .child(text(label).font(type_::BODY).color(if selected {
            paint.t.text
        } else {
            paint.t.text_secondary
        }))
}

fn sidebar(ui: &Ui, paint: Paint) -> El {
    let mut menu = col().key("side-navigation").w_full().gap(5.0);
    for page in Page::ALL {
        menu = menu.child(nav(page, ui.page == page, paint));
    }
    col()
        .key("sidebar")
        .w(222.0)
        .h_full()
        .padding(16.0)
        .gap(24.0)
        .fill(paint.t.card)
        .border(1.0, paint.t.card_border)
        .child(
            row()
                .key("brand")
                .h(48.0)
                .gap(10.0)
                .center_y()
                .child(
                    el().size(27.0, 27.0)
                        .radius(9.0)
                        .fill(paint.accent)
                        .center()
                        .child(text("⇄").font(type_::TITLE).color(rune::Color::WHITE)),
                )
                .child(text("Conduit").font(type_::TITLE).color(paint.t.text)),
        )
        .child(menu)
        .child(el().grow(1.0))
        .child(
            col()
                .gap(5.0)
                .child(tag("Rune native preview", paint, false))
                .child(
                    text("Connection engine unchanged")
                        .font(type_::LABEL)
                        .color(paint.t.text_secondary),
                ),
        )
}

fn summary_connection(ui: &Ui, paint: Paint) -> El {
    let conn = &ui.snap.connection;
    let route = if conn.route.is_empty() {
        "No active route".into()
    } else if conn.relay.is_empty() {
        conn.route.clone()
    } else {
        format!("{} · {}", conn.route, conn.relay)
    };
    card(
        "connection",
        paint,
        col()
            .gap(12.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(heading("Connected device", paint))
                    .child(tag(conn.state.clone(), paint, conn.linked)),
            )
            .child(text(&conn.device).font(type_::TITLE).color(paint.t.text))
            .child(muted(route, paint))
            .child(
                row()
                    .gap(9.0)
                    .child(button("connection-pair", "Pairing", paint, false, |ui| {
                        ui.navigate(Page::Pairing)
                    }))
                    .child(button("connection-send", "Send file", paint, true, |ui| {
                        ui.open_picker()
                    })),
            ),
    )
}

fn transfer_row(transfer: &bridge::Transfer, paint: Paint) -> El {
    let tr = transfer.clone();
    let progress = if tr.total == 0 {
        0
    } else {
        (tr.bytes * 100 / tr.total) as u32
    };
    let name = if tr.name.chars().count() > 60 {
        format!("{}…", tr.name.chars().take(59).collect::<String>())
    } else {
        tr.name.clone()
    };
    let mut body = row()
        .key(format!("transfer-{}", tr.id))
        .w_full()
        .center_y()
        .gap(12.0)
        .h(50.0)
        .child(el().size(8.0, 8.0).radius(4.0).fill(if tr.terminal {
            paint.t.text_secondary
        } else {
            paint.accent
        }))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(text(name).font(type_::BODY).color(paint.t.text))
                .child(muted(
                    format!("{} · {} · {}%", tr.direction, tr.state, progress),
                    paint,
                )),
        );
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
    let mut list = col().key("transfer-list").w_full().gap(9.0);
    if ui.snap.transfers.is_empty() {
        list = list.child(empty("No transfers yet", paint));
    } else {
        for tr in ui.snap.transfers.iter().take(limit) {
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
            .gap(16.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(heading("Activity", paint))
                    .child(button(
                        "show-transfers",
                        "View transfers",
                        paint,
                        false,
                        |ui| ui.navigate(Page::Transfers),
                    )),
            )
            .child(
                row()
                    .gap(12.0)
                    .child(stat("Transfers", ui.snap.transfers.len(), paint))
                    .child(stat("Notifications", ui.snap.notifications.len(), paint))
                    .child(stat("Shared links", ui.snap.links.len(), paint)),
            )
            .child(transfer_list(ui, paint, 4)),
    )
}

fn stat(label: &str, count: usize, paint: Paint) -> El {
    col()
        .grow(1.0)
        .gap(4.0)
        .padding(12.0)
        .radius(9.0)
        .fill(paint.t.bg)
        .child(
            text(count.to_string())
                .font(type_::TITLE)
                .color(paint.t.text),
        )
        .child(muted(label, paint))
}

fn transfers(ui: &Ui, paint: Paint) -> El {
    card(
        "transfers",
        paint,
        col()
            .gap(12.0)
            .child(
                row()
                    .justify_between()
                    .center_y()
                    .child(heading("File transfers", paint))
                    .child(button("send-new-file", "Send file", paint, true, |ui| {
                        ui.open_picker()
                    })),
            )
            .child(transfer_list(ui, paint, usize::MAX)),
    )
}

fn notifications(ui: &Ui, paint: Paint) -> El {
    let mut list = col().gap(5.0);
    if ui.snap.notifications.is_empty() {
        list = list.child(empty("No notifications", paint));
    } else {
        for notification in &ui.snap.notifications {
            list = list.child(
                col()
                    .key(format!("notification-{}", notification.id))
                    .w_full()
                    .padding(11.0)
                    .gap(4.0)
                    .radius(7.0)
                    .fill(paint.t.bg)
                    .child(
                        row()
                            .gap(10.0)
                            .child(
                                text(&notification.app)
                                    .font(type_::LABEL)
                                    .color(paint.accent),
                            )
                            .child(
                                text(&notification.title)
                                    .font(type_::BODY)
                                    .color(paint.t.text),
                            ),
                    )
                    .child(muted(notification.body.clone(), paint)),
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
                    .child(heading("Notifications", paint))
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
            .child(list),
    )
}

fn links(ui: &Ui, paint: Paint) -> El {
    let mut list = col().gap(8.0);
    if ui.snap.links.is_empty() {
        list = list.child(empty("No links shared from the phone", paint));
    } else {
        for item in &ui.snap.links {
            let url = item.url.clone();
            list = list.child(
                row()
                    .key(format!("link-{}", item.id))
                    .w_full()
                    .padding(10.0)
                    .gap(12.0)
                    .center_y()
                    .radius(7.0)
                    .fill(paint.t.bg)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(text(&item.title).font(type_::BODY).color(paint.t.text))
                            .child(muted(format!("{} · {}", item.source, item.url), paint)),
                    )
                    .child(button(
                        format!("open-{}", item.id),
                        "Open",
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
                    .child(heading("Shared links", paint))
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
            .child(list),
    )
}

fn pairing(ui: &Ui, paint: Paint) -> El {
    let p = &ui.snap.pairing;
    let mut content = col().gap(18.0).child(
        row()
            .justify_between()
            .center_y()
            .child(heading("Phone pairing", paint))
            .child(tag(
                if p.paired { "Paired" } else { "Not paired" },
                paint,
                p.paired,
            )),
    );
    if p.paired {
        content = content
            .child(text(&p.device).font(type_::TITLE).color(paint.t.text))
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
            .child(text(&p.code).font(type_::TITLE).color(paint.accent))
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
            .child(muted("Connect your Android phone to Conduit", paint))
            .child(button("start-pair", "Start pairing", paint, true, |ui| {
                ui.operation("Starting pairing", || {
                    bridge::daemon_command(&["pair", "start"]).map(|_| ())
                })
            }));
    }
    card("pairing", paint, content)
}

fn field(label: &str, edit: &TextEdit, paint: Paint, editable: fn(&mut Ui) -> &mut TextEdit) -> El {
    col()
        .gap(7.0)
        .w_full()
        .child(text(label).font(type_::LABEL).color(paint.t.text_secondary))
        .child(
            row()
                .w_full()
                .min_h(36.0)
                .px(10.0)
                .center_y()
                .fill(paint.t.bg)
                .radius(8.0)
                .border(1.0, paint.t.card_border)
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
                .color(paint.t.text_secondary),
        )
        .child(
            col()
                .w_full()
                .min_h(98.0)
                .padding(10.0)
                .fill(paint.t.bg)
                .radius(8.0)
                .border(1.0, paint.t.card_border)
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
    let mut proxy_modes = row().gap(8.0);
    for (mode, label) in [
        ("direct", "Direct"),
        ("system", "System proxy"),
        ("manual", "SOCKS5"),
    ] {
        let selected = ui.proxy_mode == mode;
        proxy_modes = proxy_modes.child(button(
            format!("proxy-{mode}"),
            label,
            paint,
            selected,
            move |ui| {
                ui.settings_dirty = true;
                ui.proxy_mode = mode.to_owned();
            },
        ));
    }
    let mut content = col()
        .gap(16.0)
        .child(heading("Connection settings", paint))
        .child(relays_field(ui, paint))
        .child(muted("Proxy route", paint))
        .child(proxy_modes);
    if ui.proxy_mode == "manual" {
        content = content.child(field("SOCKS5 URL", &ui.manual_proxy, paint, |s| {
            &mut s.manual_proxy
        }));
    }
    content = content
        .child(field("Receive folder", &ui.receive_dir, paint, |s| {
            &mut s.receive_dir
        }))
        .child(row().child(button(
            "toggle-tray",
            if ui.tray_icon {
                "Tray icon: On"
            } else {
                "Tray icon: Off"
            },
            paint,
            ui.tray_icon,
            |s| {
                s.tray_icon = !s.tray_icon;
                s.settings_dirty = true;
            },
        )))
        .child(
            row()
                .gap(9.0)
                .child(button(
                    "save-settings",
                    "Save settings",
                    paint,
                    true,
                    |ui| ui.save_settings(),
                ))
                .child(button(
                    "folder-received",
                    "Open downloads",
                    paint,
                    false,
                    |ui| ui.operation("Opening downloads", || bridge::open_folder("received")),
                )),
        );

    card("settings", paint, content)
}

fn windows_integration(ui: &Ui, paint: Paint) -> El {
    card(
        "windows-integration",
        paint,
        col()
            .gap(14.0)
            .child(heading("Windows integration", paint))
            .child(
                row()
                    .gap(10.0)
                    .child(button(
                        "autostart",
                        if ui.snap.settings.autostart {
                            "Autostart: On"
                        } else {
                            "Autostart: Off"
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
                    ))
                    .child(button(
                        "explorer",
                        if ui.snap.settings.explorer {
                            "Explorer: On"
                        } else {
                            "Explorer: Off"
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
            .child(
                row()
                    .gap(9.0)
                    .child(button(
                        "data-folder",
                        "Open data folder",
                        paint,
                        false,
                        |ui| ui.operation("Opening data folder", || bridge::open_folder("data")),
                    ))
                    .child(button(
                        "legacy-window",
                        "Open legacy UI",
                        paint,
                        false,
                        |ui| ui.operation("Opening legacy UI", bridge::open_legacy_ui),
                    )),
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
            Page::Overview => unreachable!(),
        });
    }
    if ui.page == Page::Settings {
        body = body.child(windows_integration(ui, paint));
    }
    body
}

fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let t = tok::theme(cx.system_dark());
    let paint = Paint {
        t,
        accent: tok::accent(0, cx.system_dark()),
    };
    let state = if ui.snap.connection.linked {
        "Connected"
    } else {
        "Offline"
    };
    let mut body = col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(24.0)
        .gap(17.0)
        .child(
            row()
                .key("page-header")
                .w_full()
                .justify_between()
                .center_y()
                .child(heading(ui.page.name(), paint))
                .child(tag(state, paint, ui.snap.connection.linked)),
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
        .fill(t.bg)
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
        while let Ok(event) = rx.recv() {
            let Ok(event) = event else { continue };
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

#[cfg(windows)]
fn acquire_instance() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let key: Vec<u16> = "Local\\Conduit.Rune.Preview.2026\0"
        .encode_utf16()
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, key.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let title: Vec<u16> = format!("{WINDOW_TITLE}\0").encode_utf16().collect();
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
    #[cfg(windows)]
    let Some(_instance_guard) = acquire_instance() else {
        return;
    };

    let app = App::new(AppSpec {
        scenes: &[("connected", connected_scene)],
        on_key: Some(on_key),
        min_size: Some((760.0, 510.0)),
        ..AppSpec::new(WINDOW_TITLE, (1060.0, 750.0), Ui::init, view)
    })
    .tab_visits_controls(true)
    // Rune's first Windows frame can be initialized before the final DPI-sized
    // client area settles. Nudge its native size once to trigger a real resize
    // event and synchronize the surface + logical viewport without polling.
    .on_window(|window| window.resize_instant((1060.0, 749.0)));
    let handle = app.handle();
    let _ = HANDLE.set(handle.clone());
    // Only a user-launched desktop starts or watches the installed daemon. Headless
    // storybook/screenshot tests must not touch an identity or launch a background process.
    let is_headless = std::env::args_os().any(|a| {
        matches!(
            a.to_string_lossy().as_ref(),
            "--snap" | "--tree" | "--script"
        )
    });
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
        let (x, y) = rt.find("nav-Transfers").expect("transfers navigation");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.page, Page::Transfers);
        assert!(rt.find("transfer-aabbccddee").is_some());
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
}
