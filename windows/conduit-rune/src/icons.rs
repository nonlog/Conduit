//! Vector navigation and action icons using Lucide's 24px stroke conventions.
//! Rendered directly by Rune's SVG glyph atlas: no emoji, icon fonts, or WebView.
//! Lucide icons: https://lucide.dev/ (ISC License).

use rune::Icon;

/// Shared native brand mark; theme color is supplied by Rune, never a bitmap tile.
pub static CONDUIT: Icon = Icon {
    name: "conduit-link-mark",
    view: 24.0,
    body: include_str!("../../conduit-daemon/assets/conduit-mark.fragment.svg"),
};

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
    MONITOR,
    "lucide-monitor",
    r#"<rect x="2" y="3" width="20" height="14" rx="2" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M8 21h8M12 17v4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"/>"#
);
lucide!(
    SEND,
    "lucide-send",
    r#"<path d="m22 2-7 20-4-9-9-4 20-7ZM22 2 11 13" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ARROW_UP_RIGHT,
    "lucide-arrow-up-right",
    r#"<path d="M7 17 17 7M7 7h10v10" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>"#
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

lucide!(
    FILE_IMAGE,
    "file-image",
    r#"<rect x="3" y="3" width="18" height="18" rx="3" fill="none" stroke="currentColor" stroke-width="1.7"/><circle cx="8" cy="8" r="1.5" fill="currentColor"/><path d="m3 16 5-5 4 4 3-3 6 6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/>"#
);
lucide!(
    FILE_ARCHIVE,
    "file-archive",
    r#"<rect x="4" y="3" width="16" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M12 3v11m-2 3h4v3h-4zM10 6h4m-4 4h4" fill="none" stroke="currentColor" stroke-width="1.7"/>"#
);
lucide!(
    FILE_VIDEO,
    "file-video",
    r#"<rect x="3" y="4" width="18" height="16" rx="3" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="m10 8 6 4-6 4z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/>"#
);
lucide!(
    FILE_AUDIO,
    "file-audio",
    r#"<path d="M9 18V5l11-2v13M9 9l11-2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/><ellipse cx="6" cy="18" rx="3" ry="3" fill="none" stroke="currentColor" stroke-width="1.7"/><ellipse cx="17" cy="16" rx="3" ry="3" fill="none" stroke="currentColor" stroke-width="1.7"/>"#
);
lucide!(
    FILE_TEXT,
    "file-text",
    r#"<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9l-6-6zm0 0v6h6M8 13h8M8 17h5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CLOSE,
    "close",
    r#"<path d="m6 6 12 12M18 6 6 18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>"#
);
lucide!(
    PANEL_LEFT,
    "lucide-panel-left",
    r#"<rect x="3" y="3" width="18" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="1.75"/><path d="M9 3v18" fill="none" stroke="currentColor" stroke-width="1.75"/>"#
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_brand_mark_keeps_both_link_interiors_hollow() {
        // Rune's glyph parser does not inherit attributes from SVG groups.
        // Compare actual coverage inside each loop, not just nonempty pixels.
        let alpha = rune::text::icon_coverage(&CONDUIT, 48);
        assert_eq!(alpha.len(), 48 * 48);
        assert!(alpha.iter().any(|value| *value > 128));
        for (x, y) in [(32, 14), (16, 34)] {
            assert!(alpha[y * 48 + x] < 32, "link interior is filled at {x},{y}");
        }
    }

    #[test]
    fn every_sidebar_icon_has_visible_strokes() {
        for icon in [
            &CONDUIT,
            &HOME,
            &TRANSFER,
            &BELL,
            &LINK,
            &QR_CODE,
            &SETTINGS,
            &INFO,
            &FILE_IMAGE,
            &FILE_ARCHIVE,
            &FILE_VIDEO,
            &FILE_AUDIO,
            &FILE_TEXT,
            &MONITOR,
            &CLOSE,
            &PANEL_LEFT,
        ] {
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
