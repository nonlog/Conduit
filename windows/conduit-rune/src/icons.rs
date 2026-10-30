//! Vector navigation and action icons using Lucide's 24px stroke conventions.
//! Rendered directly by Rune's SVG glyph atlas: no emoji, icon fonts, or WebView.
//! Lucide icons: https://lucide.dev/ (ISC License).

use rune::Icon;

macro_rules! lucide {
    ($name:ident, $label:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $label,
            view: 24.0,
            body: $body,
        };
    };
}

lucide!(
    HOME,
    "lucide-house",
    r#"<path d="m3 10 9-7 9 7v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V10Z" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/><path d="M9 22v-8h6v8" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    TRANSFER,
    "lucide-file-up",
    r#"<path d="M14 2v6h6M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6Z" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/><path d="m12 18 0-7m-3 3 3-3 3 3" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    BELL,
    "lucide-bell",
    r#"<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9ZM10 21h4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    LINK,
    "lucide-link",
    r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    QR_CODE,
    "lucide-qr-code",
    r#"<rect x="3" y="3" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.75"/><rect x="14" y="3" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.75"/><rect x="3" y="14" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M14 14h3v3h-3ZM21 14v3M17 21h4M14 18v3" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SETTINGS,
    "lucide-settings",
    r#"<path d="M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06-2 2-.06-.06A1.65 1.65 0 0 0 16 18.5a1.65 1.65 0 0 0-1.2 1.5V21h-5.6v-1a1.65 1.65 0 0 0-1.2-1.5 1.65 1.65 0 0 0-1.73.32l-.06.06-2-2 .06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.6-1.15H2v-3.7h1A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06 2-2 .06.06A1.65 1.65 0 0 0 8 5.5 1.65 1.65 0 0 0 9.2 4V3h5.6v1A1.65 1.65 0 0 0 16 5.5a1.65 1.65 0 0 0 1.73-.32l.06-.06 2 2-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.6 1.15h1v3.7h-1A1.65 1.65 0 0 0 19.4 15Z" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    INFO,
    "lucide-info",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M12 11v5M12 8h.01" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"#
);
lucide!(
    SMARTPHONE,
    "lucide-smartphone",
    r#"<rect x="5" y="2" width="14" height="20" rx="2" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M11 18h2" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"/>"#
);
lucide!(
    ACTIVITY,
    "lucide-chart-no-axes-column",
    r#"<path d="M4 20v-7M10 20V4M16 20v-11M22 20v-5" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>"#
);
lucide!(
    SEND,
    "lucide-send",
    r#"<path d="m22 2-7 20-4-9-9-4 20-7ZM22 2 11 13" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRON_RIGHT,
    "lucide-chevron-right",
    r#"<path d="m9 18 6-6-6-6" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ARROW_UP_RIGHT,
    "lucide-arrow-up-right",
    r#"<path d="M7 17 17 7M7 7h10v10" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SHIELD_CHECK,
    "lucide-shield-check",
    r#"<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linejoin="round"/><path d="m9 12 2 2 4-4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ARROWS_LEFT_RIGHT,
    "lucide-arrow-left-right",
    r#"<path d="M8 3 4 7l4 4M4 7h16M16 21l4-4-4-4M20 17H4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE,
    "lucide-circle",
    r#"<circle cx="12" cy="12" r="7" fill="currentColor"/>"#
);
lucide!(
    CHECK,
    "lucide-check",
    r#"<path d="m5 12 4 4L19 6" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FOLDER,
    "lucide-folder",
    r#"<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linejoin="round"/>"#
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sidebar_icon_has_visible_strokes() {
        for icon in [&HOME, &TRANSFER, &BELL, &LINK, &QR_CODE, &SETTINGS, &INFO] {
            assert!(
                rune::text::icon_coverage(icon, 24)
                    .iter()
                    .any(|alpha| *alpha > 0),
                "{} is blank",
                icon.name
            );
        }
    }
}
