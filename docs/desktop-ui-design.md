# Conduit desktop visual system

Conduit is a native Rust/Rune Windows application. Its navigation and page layout
remain compact and event-driven: paired-device state, transfers, notifications,
shared links, settings and about pages.

## Windows Fluent / Mica
- Rune paints surfaces, text, accents and selection from its semantic light/dark theme.
- The custom window frame follows the Windows theme without a web or XAML control tree.

## Identity and behavior
- **Icon unchanged:** retain the transparent, two-track bidirectional Conduit
  arrows, including their existing brand Clay glyph #D97757. This brand image
  color is intentionally independent from the user's Windows accent color.
- **Layout unchanged:** preserve original spacing, card geometry, navigation,
  accessibility names and screen reader targets.
- **Functionality unchanged:** preserve all view-model bindings, daemon IPC,
  notifications, file/drop, pairing and settings handlers, window captions,
  Share Target and single-instance behavior; no new background polling.
- Theme and layout code live in `windows/conduit-rune`; file sharing uses the separate
  Rust `windows/conduit-share-target` host and existing daemon IPC.
- The layout was originally inspired by Caho1/open-claude-ui (MIT). No web UI or
  Claude/Anthropic logo is embedded.

Build all binaries with GitHub Actions. Do not build on Windows or VPS hosts.
