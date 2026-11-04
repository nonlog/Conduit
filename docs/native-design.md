# Native desktop design contract

Conduit is a device companion, not an analytics dashboard. The user opens it to send a file,
check a transfer or reconnect a phone. Keep these actions in the first screen.

## References applied
- interface-design: https://github.com/Dammyjay93/interface-design/blob/main/.claude/skills/interface-design/SKILL.md
- Linear: https://linear.app/now/behind-the-latest-design-refresh
- Raycast: https://www.raycast.com/

Apply hierarchy, restrained surfaces and native-tool density, not screenshots or copied brand assets.
No image-generation output belongs in the application.

## System
- Domain: paired identities, encrypted links, destinations, incoming/outgoing files, progress and recovery.
- Palette: charcoal chassis, graphite inset controls, slate metadata, cobalt primary action;
  green only for verified success, red only for failure. No blue/orange logo tile or decorative glow.
- Signature: two joined links, drawn from one vector fragment; desktop and shell marks agree.
- Sidebar: 192 logical pixels, 176 compact; 36-pixel navigation rows, 17-pixel icons, 13-pixel labels.
- Body: one 24-pixel/650 page title; 13-pixel/560 filenames, 11-pixel metadata; 4-pixel spacing unit.
- Surfaces: small tonal steps and quiet 1-pixel boundaries, 7-12-pixel corners. No nested metric cards.
- Interaction: real transfer filters, native pickers, cancellation, drag/drop, keyboard navigation.
  No fake pause, retry, unread indicators, speed or ETA when the protocol lacks that state.
- Rendering: event-driven; no background animation, periodic polling or runtime asset downloads.
- Verification: actual Rune-rendered light/dark screenshots at minimum and normal windows; pointer
  interaction tests and exact-size native app icons. A mockup never counts as an acceptance result.
