#!/usr/bin/env bash
set -euo pipefail
apk="${1:?APK path is required}"
expected="$(tr -d '\r\n ' < android/signing-cert.sha256)"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'Invalid expected public certificate digest' >&2; exit 1; }
signer="$ANDROID_SDK_ROOT/build-tools/37.0.0/apksigner"
report="$($signer verify --verbose --print-certs "$apk")"
actual="$(printf '%s\n' "$report" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p')"
[[ "$actual" == "$expected" ]] || { echo 'Release APK signing certificate mismatch' >&2; exit 1; }
[[ "$(printf '%s\n' "$report" | grep -c '^Signer #[0-9]* certificate SHA-256 digest: ')" == 1 ]] || {
  echo 'Unexpected multi-signer APK' >&2; exit 1;
}
printf 'PERSISTENT_SIGNER_VERIFIED certificate_sha256=%s\n' "$actual"
sha256sum "$apk"
