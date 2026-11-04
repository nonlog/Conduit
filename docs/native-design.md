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

## Native materials and motion
- Appearance exposes Acrylic (default), Mica and Solid without changing Windows preferences.
  Real DWM composition owns the blur; text and functional cards retain readable contrast.
  Unsupported transparency and high-contrast mode fall back to fully opaque surfaces.
- Vulkan inherited alpha needs an explicit per-pixel HWND composition opt-in. The native backing
  is zeroed once at setup, material changes and geometry/erase events. Never remove the HWND
  redirection bitmap, capture/blur the desktop in a loop, or clear GDI on every GPU frame.
- Visual acceptance uses an actual temporary colored window behind the installed app, without
  changing its composition attributes. A successful DWM attribute call or headless image is insufficient.
- Notched mouse-wheel input glides to a bounded target with reversal from the presented offset.
  Precise touchpad input stays direct; disabled system animations and the user's scroll toggle win.
- Disconnect is a reversible, persistent pause, not unpair. Active transfers must be cancelled or
  completed first. Reconnect preserves identities and routes through the existing daemon.
- Android uses the same vector source and finite interaction-triggered motion; no idle animation
  loop or extra radio wakeup is added. The concrete motion contract is in `android/MOTION.md`.
