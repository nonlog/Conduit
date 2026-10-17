# Conduit desktop visual system

Conduit is a native Uno/WinUI Windows application. The navigation and page
layout introduced in PR #11 remains unchanged: 246-pixel sidebar, paired-device
card, notifications, shared links, settings and about pages.

## Windows Fluent / Mica
- Real Mica: App.xaml.cs already applies MicaBackdrop. MainPage.xaml leaves the
  page, caption root and sidebar transparent so the OS backdrop is visible.
  The workspace uses the native LayerFillColorDefaultBrush translucent layer.
- Native surfaces: CardBackgroundFillColorDefaultBrush and CardStrokeColorDefaultBrush
  for cards, ControlFillColorDefaultBrush for subdued surfaces, and
  ControlStrokeColorDefaultBrush for borders. Navigation selection uses
  SubtleFillColorSecondaryBrush.
- Native text and accent: TextFillColor*Brush, AccentTextFillColorPrimaryBrush,
  AccentFillColorDefaultBrush and the built-in AccentButtonStyle. Native controls
  follow the Windows app theme and accent, without SystemAccentColor overrides.
- Light/dark/high contrast: resolve all colors through Windows theme resources,
  rather than using hard-coded light/dark hex colors.

## Identity and behavior
- **Icon unchanged:** retain the transparent, two-track bidirectional Conduit
  arrows, including their existing brand Clay glyph #D97757. This brand image
  color is intentionally independent from the user's Windows accent color.
- **Layout unchanged:** preserve original spacing, card geometry, navigation,
  accessibility names and screen reader targets.
- **Functionality unchanged:** preserve all view-model bindings, daemon IPC,
  notifications, file/drop, pairing and settings handlers, window captions,
  Share Target and single-instance behavior; no new background polling.
- Theme button styles live in windows/conduit-ui/App.xaml. Presentation lives in
  MainPage.xaml; commands remain in MainPage.xaml.cs.
- The layout was originally inspired by Caho1/open-claude-ui (MIT). No web UI or
  Claude/Anthropic logo is embedded.

Build all binaries with GitHub Actions. Do not build on Windows or VPS hosts.