# Mobile motion and shared identity

The behavior reference is FlClash's `lib/pages/home.dart` (upstream file SHA
`6df0c14673a88b7fe1d3e55a23bbf34858f854a3`): direct tab transitions, retained page
state, animated navigation glyphs, and inactive pages that do not keep ticking.
Conduit uses its existing public Jetpack Compose / Material 3 Expressive APIs;
it does not embed Flutter, copy FlClash artwork, or use image-generated mockups.

- Tabs slide/fade directly to the selected destination in 280 ms; nonadjacent
  navigation never renders an intervening destination. Saveable tab state survives.
- Navigation uses a moving spring indicator, animated icon/label ink and finite
  press/selection scaling. Touch targets are unchanged.
- Connection status crossfades, newly available send controls expand/collapse,
  and measured transfer progress interpolates between reported values only.
- Settings sections have reversible disclosure motion. Pair/theme/forget dialogs
  retain the native modal host until their finite dismiss transition completes.
- Android animator scale zero removes spatial transitions. A content observer
  updates this policy only when the system setting changes; no polling, continuous
  animation, or background service motion is added.

`ConduitMotionInstrumentedTest` asserts intermediate and final positions, fast
retargeting, saved state, reduced motion, disclosure height and modal teardown
using Compose's virtual clock on GitHub Actions, not the user's phone.

`scripts/sync-android-brand.py --check` checks launcher/adaptive/monochrome,
notification/tile, in-app and Direct Share vectors against the same source paths
as the Windows brand. An APK update is required before a device shows these changes.
