#!/usr/bin/env bash
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && -n "${RUNNER_TEMP:-}" && -n "${GITHUB_ENV:-}" ]] || {
  echo 'Private signing preparation is restricted to GitHub Actions' >&2; exit 1;
}
[[ -n "${CONDUIT_ANDROID_KEYSTORE_BASE64:-}" ]] || {
  echo 'Persistent signing key is not configured; refusing ephemeral release signing' >&2; exit 1;
}
umask 077
key="$RUNNER_TEMP/conduit-release.p12"
printf '%s' "$CONDUIT_ANDROID_KEYSTORE_BASE64" | base64 --decode > "$key"
[[ -s "$key" ]]
printf 'CONDUIT_ANDROID_KEYSTORE=%s\n' "$key" >> "$GITHUB_ENV"
