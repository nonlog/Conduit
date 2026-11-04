#!/usr/bin/env bash
set -euo pipefail
apk="${1:?APK path is required}"
expected="$(tr -d '\r\n ' < android/signing-cert.sha256)"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'Invalid expected public certificate digest' >&2; exit 1; }
signer="$ANDROID_SDK_ROOT/build-tools/37.0.0/apksigner"
python3 scripts/apk-certificate-digests.py --self-test
report="$($signer verify --verbose --print-certs-pem "$apk")"
# Hash all verified DER identities, including repeats for different signature schemes.
# Additional or rotated certificates are rejected unless explicitly approved in source.
actual="$(printf '%s\n' "$report" | python3 scripts/apk-certificate-digests.py)"
[[ "$actual" == "$expected" ]] || {
  echo 'Release APK signing certificate mismatch or unexpected additional signer' >&2
  printf 'Expected public digest: %s\nObserved public digest(s): %s\n' "$expected" "$actual" >&2
  exit 1
}
printf 'PERSISTENT_SIGNER_VERIFIED certificate_sha256=%s\n' "$actual"
sha256sum "$apk"
