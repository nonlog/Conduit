#!/usr/bin/env bash
set -euo pipefail

if [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
  echo 'Optimized APK verification is restricted to GitHub Actions.' >&2
  exit 1
fi
apk="${1:?Usage: verify-android-entrypoints.sh path/to/release.apk}"
analyzer="${ANDROID_SDK_ROOT:?}/cmdline-tools/latest/bin/apkanalyzer"
[[ -f "$apk" && -x "$analyzer" ]]

# Check actual optimized DEX, not source or a debug build. The asset-loaded
# LSPosed class otherwise disappears silently during release shrinking.
entry="$("$analyzer" files cat --file /assets/xposed_init "$apk" | tr -d '\r\n')"
[[ "$entry" == 'com.conduit.sync.ClipboardHook' ]]
"$analyzer" dex code --class "$entry" "$apk" > /dev/null
"$analyzer" dex code --class com.conduit.sync.PairingScannerActivity "$apk" > /dev/null

defined="$("$analyzer" dex packages --defined-only "$apk")"
if grep -Eq 'com\.google\.mlkit\.vision\.codescanner|de\.robv\.android\.xposed\.IXposedHookLoadPackage' <<< "$defined"; then
  echo 'APK must not embed the retired Google scanner or the external Xposed framework API.' >&2
  exit 1
fi
echo 'OPTIMIZED_APK_ENTRYPOINTS_PASSED: offline scanner and external LSPosed entry point preserved'
