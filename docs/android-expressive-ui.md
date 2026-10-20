# Conduit Android: expressive material and glass

This Android presentation rewrite is informed by the visual language in
[FlClash](https://github.com/chen08209/FlClash) (Material You with Apple-style
interaction, layered Material surfaces, large continuous corners, floating navigation,
spring transitions, and opt-in liquid glass). FlClash is Flutter/GPL-3.0; **no Dart,
shader, icon, asset or other source code is copied**. Conduit is still native Kotlin
and Jetpack Compose.

## Appearance contract

- Material 3 Expressive design language built with the project's supported public
  Compose APIs: Android wallpaper dynamic colors, continuous rounded shapes, elevated
  tonal surfaces, pill navigation, and event-driven spring motion. The experimental
  `MaterialExpressiveTheme` symbol in the pinned 2026.06 BOM is still internal;
  upgrading to its public 1.5 beta also requires the SDK 37/AGP 9 toolchain, which
  is deliberately not bundled with this UI-only patch.
- Page background and hierarchy: the Android wallpaper's light/dark dynamic Material
  color scheme, with tonal Material 3 surfaces and consistent, rounded cards.
- Suspended bottom dock: 76 dp high, rounded lens, two destinations, animated selected
  pill, 48+ dp touch target; page content scrolls behind no occluding fixed toolbar.
- Connected device hero: clearly differentiated paired, connecting, connected,
  disconnected and pairing states; live route chip; same original actions.
- File/photo actions: two exactly 60 dp high buttons with matched width on a
  standard 360 dp phone; when the usable content width drops below 270 dp, or the
  font scale is at least 1.35, stack vertically instead of wrapping labels.
- Active transfers: filename, direction, independent progress, completion/error state
  and the same per-transfer cancel callback. Clipboard history remains searchable.
- Settings: separate Appearance, Devices, Sync & Privacy, Background delivery and About
  groups, preserving the old device management, sensitive-notification preference,
  Accessibility and OPlus-specific controls.
- Glass can be turned off without disabling Material Expressive. A persisted
  liquid_glass choice only changes Compose foreground surfaces. It does not change
  a service, add a renderer, start an effect loop or wake a sleeping device.
- Glass here is an inexpensive *optical* treatment (gradient, translucent tint,
  rim reflection, shadow), not a claim that Compose implements FlClash's custom
  Flutter refraction/capture fragment shader. Text-bearing settings cards stay
  readable opaque tonal surfaces.

## Acceptance

1. Build and unit-test through **GitHub Actions only**, not the Windows workstation
   or VPS. Run the CI Android debug APK on a real device only with data-preserving
   installation and verify settings, UID, pair ID and notification access.
2. Test a narrow device and larger font scale: both send actions remain 60 dp high;
   bottom dock is not clipped; panels scroll; cancel is at least 48 dp.
3. Validate Home, Settings, clipboard search/history, pairing/code/QR flow,
   connect/disconnect, notification hiding, file/photo pickers, progress and cancel.
4. Switch dark/light system theme and wallpaper accent. Toggle glass off/on and
   restart the activity: the choice persists without resetting linkWanted or
   hideNotificationContent.
5. Do not merge this stacked UI PR ahead of the transfer reliability base PR.
   Never overwrite the user's untracked agent notes or existing worktrees.
