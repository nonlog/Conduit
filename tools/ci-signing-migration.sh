#!/usr/bin/env bash
# Rehearse normal certificate migration with synthetic data on a disposable emulator only.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true ]] || { echo 'CI only' >&2; exit 1; }
serial="$(adb get-serialno)"
[[ "$serial" == emulator-* ]] || { echo 'Refusing non-emulator device' >&2; exit 1; }
key="$RUNNER_TEMP/migration-test.p12"
new="$RUNNER_TEMP/migration-test.apk"
apk="android/app/build/outputs/apk/debug/app-debug.apk"
helper=/data/local/tmp/conduit-migration-helper.sh
backup=/data/local/tmp/conduit-backup-ci-certificate
signer="$ANDROID_SDK_ROOT/build-tools/37.0.0/apksigner"
cleanup() { rm -f "$key" "$new" "$new.idsig"; }
trap cleanup EXIT
adb root
adb wait-for-device
[[ "$(adb shell id -u | tr -d '\r')" == 0 ]]
# The instrumentation suite may leave its own installation; all data here is synthetic.
adb uninstall com.conduit.sync >/dev/null 2>&1 || true
adb install "$apk" >/dev/null
adb shell am force-stop com.conduit.sync
adb shell 'mkdir -p /data/user/0/com.conduit.sync/files /data/user/0/com.conduit.sync/databases; printf synthetic-noise-identity > /data/user/0/com.conduit.sync/files/identity.bin; printf synthetic-trusted-peer > /data/user/0/com.conduit.sync/files/peer.txt; printf synthetic-history > /data/user/0/com.conduit.sync/databases/history.db'
adb push scripts/android-state-migration.sh "$helper" >/dev/null
adb shell sh "$helper" backup "$backup"
adb shell sh "$helper" verify "$backup"
# No real key. This disposable key proves that the restore path survives a genuinely new cert.
keytool -genkeypair -keystore "$key" -storetype PKCS12 -storepass synthetic-ci-only -keypass synthetic-ci-only -alias migration -keyalg RSA -keysize 2048 -validity 2 -dname 'CN=Synthetic Migration Test' >/dev/null 2>&1
"$signer" sign --ks "$key" --ks-key-alias migration --ks-pass pass:synthetic-ci-only --key-pass pass:synthetic-ci-only --out "$new" "$apk"
old_cert="$($signer verify --print-certs-pem "$apk" | python3 scripts/apk-certificate-digests.py)"
new_cert="$($signer verify --print-certs-pem "$new" | python3 scripts/apk-certificate-digests.py)"
[[ -n "$old_cert" && -n "$new_cert" && "$old_cert" != "$new_cert" ]]
adb uninstall com.conduit.sync >/dev/null
adb install "$new" >/dev/null
adb shell am force-stop com.conduit.sync
adb shell sh "$helper" restore "$backup"
# Future same-signer upgrades must be ordinary in-place installs, without any data migration.
adb install -r "$new" >/dev/null
adb shell 'cd /data/user/0/com.conduit.sync && sha256sum -c /data/local/tmp/conduit-backup-ci-certificate/ce.files.sha256' >/dev/null
adb shell 'test "$(stat -c %u /data/user/0/com.conduit.sync/files/identity.bin)" = "$(stat -c %u /data/user/0/com.conduit.sync)"'
adb shell rm -rf "$backup"
adb shell rm -f "$helper"
adb uninstall com.conduit.sync >/dev/null
echo 'CERTIFICATE_MIGRATION_AND_SAME_SIGNER_UPDATE_PRESERVE_APP_STATE'
