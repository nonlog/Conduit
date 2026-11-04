#!/usr/bin/env bash
set -euo pipefail
cd "${GITHUB_WORKSPACE:-$(pwd)}"
mkdir -p artifacts/expressive-ui
python3 scripts/sync-android-brand.py --check
# A zero animator scale would let a broken animation pass as an instantaneous state change.
# This is a disposable CI emulator, never the user's phone.
adb shell settings put global animator_duration_scale 1
adb shell settings put global transition_animation_scale 1
adb shell settings put global window_animation_scale 1
status=0
(cd android && ./gradlew --no-daemon --stacktrace connectedDebugAndroidTest) || status=$?
# Tests copy PNGs to Downloads before AGP uninstalls the test APK. Do not accept a
# green visual job with missing screenshots, as the earlier workflow did.
if ! adb pull /sdcard/Download/conduit-ui-smoke artifacts/expressive-ui/; then
  echo 'UI screenshots were not exported' >&2
  status=1
fi
if ! find artifacts/expressive-ui -name '*.png' -print -quit | grep -q .; then
  echo 'No PNG screenshots collected' >&2
  status=1
fi
exit "$status"
