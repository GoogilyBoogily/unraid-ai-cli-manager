#!/bin/bash
# Persist OpenAI Codex CLI (binary, ~/.codex) across Unraid reboots.
#
# Same model as /boot/config/claude-code/claude-persist.sh: the live copy stays
# in RAM (/root/.codex) so Codex keeps working while the array is stopped and
# never holds SQLite files open on /mnt/user. The persisted copy lives in
# appdata and is synced from RAM periodically and when the array stops.
#
# Nothing here ever creates paths under /mnt/user unless shfs is mounted:
# a stray /mnt/user on rootfs stops emhttpd from mounting user shares.
#
# Usage: codex-persist.sh restore|backup|status

set -u
export HOME=/root
export PATH="/root/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
export CODEX_HOME=/root/.codex

SHARE_MOUNT=/mnt/user
PERSIST=/mnt/user/appdata/codex
LIVE_DIR=/root/.codex
# Lives in RAM outside ~/.codex: resets every boot and is never synced.
MARKER=/root/.codex-persist-restored
BIN=/root/.local/bin/codex
LOCK=/var/run/codex-persist.lock
INSTALLER_URL=https://chatgpt.com/codex/install.sh
# The standalone package (binary + bundled bwrap/rg/helpers) is persisted with the rest.
PACKAGES=packages/standalone
EXCLUDES=(--exclude=/log/ --exclude=/tmp/ --exclude=/.tmp/
          --exclude=/history.jsonl --exclude='*.sqlite*')

log() { echo "codex-persist: $*"; logger -t codex-persist -- "$*"; }

shares_mounted() { mountpoint -q "$SHARE_MOUNT" && [ -d "$SHARE_MOUNT/appdata" ]; }

wait_for_shares() {
    local attempt
    for attempt in $(seq 1 24); do
        shares_mounted && return 0
        sleep 5
    done
    return 1
}

prepare_persist_dir() {
    mkdir -p "$PERSIST/.codex"
    chown root:root "$PERSIST"
    chmod 700 "$PERSIST"
}

installed_version() { "$BIN" --version 2>/dev/null | awk '{print $NF}'; }

restore_binary() {
    if [ ! -x "$BIN" ]; then
        mkdir -p /root/.local/bin "$LIVE_DIR"
        if [ -d "$PERSIST/.codex/$PACKAGES/releases" ]; then
            rsync -a "$PERSIST/.codex/packages/" "$LIVE_DIR/packages/"
            ln -sfn "$LIVE_DIR/$PACKAGES/current/bin/codex" "$BIN"
            log "restored package $(installed_version) from appdata"
        else
            log "no persisted package; running official installer"
            curl -fsSL "$INSTALLER_URL" | CODEX_INSTALL_DIR=/root/.local/bin sh || log "installer failed"
        fi
    fi
    [ -x "$BIN" ] && ln -sfn "$BIN" /usr/local/bin/codex
}

# Union of persisted and live prompt history, oldest first, without duplicates.
merge_history() {
    local merged
    merged=$(mktemp "$LIVE_DIR/history.jsonl.XXXXXX")
    cat "$PERSIST/.codex/history.jsonl" "$LIVE_DIR/history.jsonl" 2>/dev/null | awk '!seen[$0]++' >"$merged"
    chmod 600 "$merged"
    mv -f "$merged" "$LIVE_DIR/history.jsonl"
}

# Live SQLite files are copied with .backup so a running Codex never yields a torn copy.
backup_sqlite_databases() {
    local database relative destination
    find "$LIVE_DIR" -path "$LIVE_DIR/packages" -prune -o -type f -name '*.sqlite' -print |
    while read -r database; do
        relative=${database#"$LIVE_DIR"/}
        destination="$PERSIST/.codex/$relative"
        mkdir -p "$(dirname "$destination")"
        if sqlite3 "$database" ".backup '$destination.tmp'"; then
            mv -f "$destination.tmp" "$destination"
        else
            rm -f "$destination.tmp"
            log "could not back up $relative"
        fi
    done
}

restore_sqlite_databases() {
    local database relative
    find "$PERSIST/.codex" -type f -name '*.sqlite' -print |
    while read -r database; do
        relative=${database#"$PERSIST/.codex"/}
        [ -e "$LIVE_DIR/$relative" ] && continue
        mkdir -p "$(dirname "$LIVE_DIR/$relative")"
        cp -p "$database" "$LIVE_DIR/$relative"
    done
}

restore() {
    if ! wait_for_shares; then
        log "user shares not mounted; not touching $SHARE_MOUNT"
        return 1
    fi
    exec 9>"$LOCK"
    flock 9
    prepare_persist_dir

    restore_binary

    if [ -e "$MARKER" ]; then
        log "already restored this boot; RAM copy is authoritative"
        return 0
    fi

    mkdir -p "$LIVE_DIR"
    chmod 700 "$LIVE_DIR"
    rsync -a --update "${EXCLUDES[@]}" "$PERSIST/.codex/" "$LIVE_DIR/"
    restore_sqlite_databases
    merge_history

    touch "$MARKER"
    log "restored Codex config from $PERSIST"
}

backup() {
    if ! shares_mounted; then
        log "user shares not mounted; skipping backup"
        return 1
    fi
    if [ ! -e "$MARKER" ]; then
        log "restore has not run this boot; refusing to overwrite persisted copy"
        return 1
    fi
    exec 9>"$LOCK"
    if ! flock -n 9; then
        log "another sync is running; skipping"
        return 0
    fi
    prepare_persist_dir

    if [ -d "$LIVE_DIR" ]; then
        # --delete skips excluded paths, so persisted history and databases are kept.
        rsync -a --delete "${EXCLUDES[@]}" "$LIVE_DIR/" "$PERSIST/.codex/"
        [ -f "$LIVE_DIR/history.jsonl" ] && cp -p "$LIVE_DIR/history.jsonl" "$PERSIST/.codex/history.jsonl.tmp" &&
            mv -f "$PERSIST/.codex/history.jsonl.tmp" "$PERSIST/.codex/history.jsonl"
        backup_sqlite_databases
    fi
    log "backed up Codex config to $PERSIST"
}

status() {
    echo "shares mounted:  $(shares_mounted && echo yes || echo no)"
    echo "restored (boot): $([ -e "$MARKER" ] && echo "yes ($(date -r "$MARKER"))" || echo no)"
    echo "binary:          $([ -x "$BIN" ] && echo "$BIN $(installed_version)" || echo missing)"
    echo "persisted pkgs:  $(ls "$PERSIST/.codex/$PACKAGES/releases" 2>/dev/null | tr '\n' ' ')"
    echo "persisted:       $(du -sh "$PERSIST" 2>/dev/null | cut -f1) in $PERSIST"
}

case "${1:-}" in
    restore) restore ;;
    backup) backup ;;
    status) status ;;
    *) echo "Usage: $0 restore|backup|status" >&2; exit 2 ;;
esac
