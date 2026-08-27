# Conduit UI design system

**Status:** authoritative after the 2026-08-27 rejected-UI reset.

## Product character

Conduit is a small native system utility, not a dashboard or marketing surface. The interface should disappear behind the task: show current connection state, expose the few actions/settings that matter, and avoid explaining obvious product behavior.

## Design dials

- Variance: 3/10 — calm, conventional, native.
- Motion: 1/10 — no decorative motion; use only platform feedback.
- Density: 5/10 — compact but touch/keyboard accessible.

## Copy rules

- Prefer labels over explanations.
- Do not add taglines, value propositions, workflow descriptions, implementation details, or repeated helper text to the main UI.
- Secondary text is allowed only when it resolves real ambiguity or reports live state.
- Never expose device fingerprints, MAC-like identifiers, protocol IDs, or pairing internals on the normal paired home screen.
- Status text should answer only: which peer, connected or not, and which route when useful.

## Android — Material 3

- Use the platform Material 3 theme and dynamic color; do not invent a fixed brand palette.
- One normal `TopAppBar` with the product name only.
- Home structure:
  1. compact neutral connection row/card;
  2. active transfer UI only while a transfer exists;
  3. compact settings rows.
- Do not use a large colored hero, promotional callout, oversized pills, or a dedicated identity/fingerprint section.
- Use neutral surface containers for persistent controls; reserve strong primary/tertiary colors for actions or transient state that genuinely needs emphasis.
- Entire navigation rows should be clickable; avoid redundant `Open` buttons when the row itself can navigate.
- Clipboard History is a real child destination: system Back/predictive-back must return to Home before the Activity can exit.
- History rows show direction, time, and content only. Do not add instructions such as “tap to copy”.

## Windows — Fluent / Windows 11

- Keep native Win32/DWM/Common Controls; this file defines visual/UX language, not a framework migration.
- Follow Windows 11 principles: calm, familiar, coherent, and system-accent aware.
- Use Segoe UI Variable, system light/dark mode, rounded top-level window corners, and the system accent sparingly.
- The Windows application/titlebar/taskbar icon, notification identity icon, and primary connection symbol use Microsoft Fluent `Phone Desktop` geometry, matching Android. The coloured app mark keeps the violet-to-blue background (`#6E5BD6` -> `#2F6FE0`); the tray uses dedicated monochrome 16/20/24 px regular glyphs rather than shrinking the coloured tile. Relay and Windows section headings stay text-only; never add decorative section glyphs.
- Use a compact two-pane utility layout rather than a vertical dashboard:
  - left: connection/peer status and on-demand actions;
  - right: Relay and Windows integration settings.
- Group related controls by spacing and a subtle surface/border; do not add explanatory paragraphs inside cards.
- Keep section labels short: `Connection`, `Relay`, `Windows`.
- Use native access keys and keyboard traversal.
- No fake navigation, resident control UI, animation loop, watcher, timer, or background refresh. The already-resident daemon may expose the optional event-driven tray icon; disabling it creates no tray thread.

## Accessibility and interaction

- Touch targets on Android: at least 44–48 dp for primary interactive controls.
- Keyboard focus and access keys remain visible on Windows.
- Color is never the only carrier of connection state.
- Text contrast follows the platform theme; avoid low-contrast decorative copy by removing it rather than dimming it.

## Explicit anti-patterns

Do not reintroduce:

- a bright full-width connection hero;
- “Phone companion / quiet idle” taglines;
- “Photo/screenshot -> Windows Snipping Tool …” workflow copy;
- `This phone's identity`, desktop fingerprint, MAC-like values, or copy-fingerprint actions on Home;
- section subtitles that merely explain the section title;
- separate `Open` CTA buttons on otherwise clickable rows;
- a Windows vertical stack of three large dashboard cards with repeated operational descriptions.
