# Android official Material 3 Expressive

## Supported build stack

- Android Gradle Plugin 9.1.1 with built-in Kotlin and the public Android DSL.
- Gradle 9.3.1; wrapper distribution SHA-256 is pinned.
- Compile SDK 37.0 (the SDK manager package is platforms;android-37.0).
- Target SDK remains 36 to keep Android 16 permission and background-sync behavior unchanged.
- Kotlin/Compose compiler 2.4.10; Compose BOM 2026.09.00.
- Material 3 1.5.0-beta01 is explicitly selected for its public MaterialExpressiveTheme.
- Fragment 1.9.1 fixes the obsolete transitive dependency exposed by ActivityResult lint.

ConduitTheme calls the official MaterialExpressiveTheme with MotionScheme.expressive().
Buttons use official animated ButtonShapes/IconButtonShapes. The device hero uses
MaterialShapes.Cookie9Sided and Clover4Leaf. No locally imitated theme or internal API
access suppression is used. The old pinned-stack limitation no longer applies.

## Visual contract

The user's FlClash screenshots define the layout reference. This is independent native
Kotlin/Compose implementation, not copied Flutter source, fonts, shaders, or assets.

- Neutral dark background #111414, cards #1E2020, raised surfaces #282A2A and #333535.
- Teal is limited to action/selection accents instead of tinting every surface green.
- Single 24sp semibold page headings, compact muted section captions, no marketing subtitles.
- Static expressive connection card with a clear main action; file/photo controls are a
  separate compact group of matched 56dp buttons. Narrow widths/large fonts stack safely.
- 62dp floating capsule navigation, neutral sliding indicator, four working destinations:
  Home, Transfers, Clipboard and Settings. The indicator uses official spatial motion.
- Settings uses grouped neutral rows, inset dividers, ordinary chevrons and one accessible
  toggle action per switch. Theme supports system/light/dark plus optional wallpaper accent.
- The former simulated Liquid glass UI is removed; its stored key is retained only for
  migration compatibility. There is no hidden renderer or continuously running animation.
- Home shows one compact transfer count/active-progress summary linked to Transfers;
  completed or cancelled file history cards appear only in the Transfers destination.
- The real OnePlus/Oppo Background delivery controls are in distinct vertical rows;
  a large-font small-screen regression checks their bounds and tap actions.
- Existing pairing, privacy, clipboard search, transfer progress/cancellation and background
  delivery controls retain their real service callbacks.

## Reliability preserved

PR #16 LAN/VPN/Relay fixes remain unchanged. Photos/Screenshots passive MediaStore observers
now treat a null 'no new capture' result as a no-op instead of emitting a misleading error
toast. Explicit sends and actual load/size failures remain visible.

## Verification

All builds, lint, release optimization and emulator tests run in GitHub Actions only.
android-expressive.yml runs automatically on Android-related pull requests to master
and matching master pushes, and remains manually dispatchable. It verifies debug/release
builds, unit tests, lint and Android 16 UI tests.
The UI tests verify actual public theme/motion, compact geometry, four destinations, dark
and light rendering, large fonts, and switch semantics. Screenshot export is checked for
nonempty PNG files before the visual job may pass.

Physical installation must use adb install -r -t without clearing application data. Verify
identity/peer hashes, existing preference values and sensitive-notification AppOp. Do not
change VPN, hotspot, system networking, or lock configuration for this UI task.
