#!/system/bin/sh
# Root-only, narrowly scoped preservation helper for the one-time APK certificate migration.
# Does not uninstall, install, grant permissions, change certificates or start the application.
set -eu
umask 077
PACKAGE=com.conduit.sync
MODE=${1:-}
BACKUP=${2:-}
case "$MODE" in backup|verify|restore) ;; *) echo 'usage: helper backup|verify|restore /data/local/tmp/conduit-backup-ID' >&2; exit 1;; esac
case "$BACKUP" in /data/local/tmp/conduit-backup-*) ;; *) echo 'Unexpected backup path' >&2; exit 1;; esac
case "${BACKUP#/data/local/tmp/conduit-backup-}" in ''|*[!a-zA-Z0-9_-]*) echo 'Invalid backup identifier' >&2; exit 1;; esac
[ "$(id -u)" = 0 ] || { echo 'Root is required' >&2; exit 1; }
[ ! -L "$BACKUP" ] || { echo 'Backup must not be a symlink' >&2; exit 1; }

quiet_app() {
    # Only data belonging to this package in the unlocked primary user is allowed.
    if pidof "$PACKAGE" >/dev/null 2>&1; then
        echo 'Force-stop Conduit before taking or restoring its consistent snapshot' >&2
        exit 1
    fi
}

check_members() {
    archive=$1
    tar -tf "$archive" | while IFS= read -r member; do
        case "$member" in
            files|files/*|shared_prefs|shared_prefs/*|databases|databases/*|no_backup|no_backup/*) ;;
            *) echo 'Unsafe archive member' >&2; exit 1;;
        esac
        case "/$member/" in */../*|*/./*) echo 'Noncanonical archive member' >&2; exit 1;; esac
    done
}

snapshot() {
    store=$1
    source=$2
    [ -d "$source" ] || return 0
    [ ! -L "$source" ] || { echo 'Unexpected symlink at application data root' >&2; exit 1; }
    stat -c '%u:%g' "$source" > "$BACKUP/$store.owner"
    : > "$BACKUP/$store.members"
    for name in files shared_prefs databases no_backup; do
        if [ -d "$source/$name" ]; then
            [ -z "$(find "$source/$name" -type l -print -quit)" ] || { echo 'Refusing symlink in application snapshot' >&2; exit 1; }
            printf '%s\n' "$name" >> "$BACKUP/$store.members"
        fi
    done
    if [ ! -s "$BACKUP/$store.members" ]; then return 0; fi
    (cd "$source" && tar -cf "$BACKUP/$store.tar" -T "$BACKUP/$store.members")
    (
        cd "$source"
        while IFS= read -r name; do find "$name" -type f -exec sha256sum '{}' \;; done < "$BACKUP/$store.members"
    ) | sort > "$BACKUP/$store.files.sha256"
    check_members "$BACKUP/$store.tar"
}

verify_store() {
    store=$1
    [ -f "$BACKUP/$store.tar" ] || return 0
    check_members "$BACKUP/$store.tar"
    scratch="$BACKUP/verify-$store"
    [ ! -e "$scratch" ] || { echo 'Previous verification directory needs inspection' >&2; exit 1; }
    mkdir "$scratch"
    (cd "$scratch" && tar -xf "$BACKUP/$store.tar")
    (cd "$scratch" && sha256sum -c "$BACKUP/$store.files.sha256" >/dev/null)
    # The exact, validated private scratch directory is the only recursive deletion here.
    rm -rf "$scratch"
}

verify_backup() {
    [ -f "$BACKUP/complete" ] || { echo 'Backup is not marked complete' >&2; exit 1; }
    [ "$(cat "$BACKUP/package")" = "$PACKAGE" ] || { echo 'Backup belongs to another package' >&2; exit 1; }
    (cd "$BACKUP" && sha256sum -c archive.sha256 >/dev/null)
    verify_store ce
    verify_store de
}

restore_store() {
    store=$1
    destination=$2
    [ -f "$BACKUP/$store.tar" ] || return 0
    [ -d "$destination" ] && [ ! -L "$destination" ] || { echo 'Expected fresh app data directory is missing' >&2; exit 1; }
    owner=$(stat -c '%u:%g' "$destination")
    uid=${owner%:*}
    [ "$uid" -ge 10000 ] && [ "$uid" -lt 20000 ] || { echo 'Unexpected primary-user app UID' >&2; exit 1; }
    while IFS= read -r name; do
        case "$name" in files|shared_prefs|databases|no_backup) ;; *) exit 1;; esac
        # Never overwrite newly-created application content without inspecting it.
        if [ -d "$destination/$name" ] && [ -n "$(find "$destination/$name" -mindepth 1 -print -quit)" ]; then
            echo "Restore target is not fresh: $name" >&2; exit 1
        fi
        [ ! -L "$destination/$name" ] || exit 1
    done < "$BACKUP/$store.members"
    (cd "$destination" && tar -xf "$BACKUP/$store.tar")
    while IFS= read -r name; do
        chown -R "$owner" "$destination/$name"
        restorecon -RF "$destination/$name"
    done < "$BACKUP/$store.members"
    (cd "$destination" && sha256sum -c "$BACKUP/$store.files.sha256" >/dev/null)
}

case "$MODE" in
    backup)
        quiet_app
        [ ! -e "$BACKUP" ] || { echo 'Backup already exists; never replace it' >&2; exit 1; }
        [ -d "/data/user/0/$PACKAGE/files" ] || { echo 'Expected existing application data not found' >&2; exit 1; }
        mkdir "$BACKUP"
        chmod 700 "$BACKUP"
        printf '%s\n' "$PACKAGE" > "$BACKUP/package"
        snapshot ce "/data/user/0/$PACKAGE"
        snapshot de "/data/user_de/0/$PACKAGE"
        [ -s "$BACKUP/ce.tar" ] || { echo 'Credential-encrypted snapshot is empty' >&2; exit 1; }
        verify_store ce
        verify_store de
        (cd "$BACKUP" && sha256sum *.tar *.files.sha256 *.members package > archive.sha256)
        touch "$BACKUP/complete"
        echo 'CONDUIT_PRIVATE_SNAPSHOT_VERIFIED'
        ;;
    verify)
        verify_backup
        echo 'CONDUIT_PRIVATE_SNAPSHOT_VERIFIED'
        ;;
    restore)
        quiet_app
        verify_backup
        restore_store ce "/data/user/0/$PACKAGE"
        restore_store de "/data/user_de/0/$PACKAGE"
        echo 'CONDUIT_APP_STATE_RESTORED_AND_HASH_VERIFIED'
        ;;
esac
