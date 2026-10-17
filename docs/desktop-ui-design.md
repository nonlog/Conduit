# Conduit desktop visual system

The Windows UI uses a native Uno/WinUI adaptation of the warm, layered design ideas
published in [Caho1/open-claude-ui](https://github.com/Caho1/open-claude-ui)
(MIT license). It is an original Conduit interface, not an embedded React or Electron
application. It contains no Claude or Anthropic logo, font, audio or proprietary assets.

- **Identity:** keep Conduit's transparent two-track bidirectional arrow glyph;
  generate the brand, taskbar, tray and Explorer icons in CDS Clay `#D97757` via
  `windows/conduit-daemon/tools/icon-render/render-icons.mjs`.
- **Surfaces:** light canvas `#FFFEFC`, warm rail `#F1EEE8`, light cards white; dark
  canvas `#171615`, rail `#211F1D`, elevated cards `#252321`.
- **Accent controls:** native checkboxes and toggle switches use Clay-filled theme resources;
  high-contrast mode retains system-resolved colors.
- **Hierarchy:** 246-pixel navigation rail; status shown with the paired device;
  separate notifications, shared-links, settings and about surfaces.
- **Shapes:** 9-pixel buttons, 11–13-pixel cards, 1-pixel hairlines; neutral text
  hierarchy and Clay emphasis rather than colorful, heavy UI chrome.
- **Behavior:** preserve existing view-model bindings, daemon IPC, all file/drop,
  notification, pairing and settings handlers, theme-aware Windows caption buttons,
  share target and single-instance behavior. No periodic work or animation loop.

Theme brushes and reusable button styles live in `windows/conduit-ui/App.xaml`.
Navigation and page presentation live in `MainPage.xaml` and `MainPage.xaml.cs`.
Use GitHub Actions for all builds and production artifacts; do not build locally
or on VPS hosts.
