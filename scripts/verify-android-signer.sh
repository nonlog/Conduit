#!/usr/bin/env bash
set -euo pipefail
apk="${1:?APK path is required}"
expected="$(tr -d '\r\n ' < android/signing-cert.sha256)"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'Invalid expected public certificate digest' >&2; exit 1; }
signer="$ANDROID_SDK_ROOT/build-tools/37.0.0/apksigner"
report="$($signer verify --verbose --print-certs "$apk")"
# Recent apksigner versions can describe the same signer for multiple signature
# schemes. Compare normalized public certificate digests, not the output line count.
actual="$(printf '%s\n' "$report" | sed -nE 's/^Signer #[0-9]+.*certificate SHA-256 digest:[[:space:]]*([0-9a-fA-F:]+)[[:space:]]*$/\1/p' | tr -d '\r:' | tr '[:upper:]' '[:lower:]' | sort -u)"
signer_ids="$(printf '%s\n' "$report" | sed -nE 's/^Signer #([0-9]+).*certificate SHA-256 digest:.*$/\1/p' | sort -u)"
[[ "$actual" == "$expected" && "$signer_ids" == 1 ]] || {
  echo 'Release APK signing certificate mismatch or unexpected additional signer' >&2
  printf 'Expected public digest: %s\nObserved public digest(s): %s\nSigner IDs: %s\n' "$expected" "$actual" "$signer_ids" >&2
  exit 1
}
printf 'PERSISTENT_SIGNER_VERIFIED certificate_sha256=%s\n' "$actual"
sha256sum "$apk"
