#!/bin/bash
# Persist Claude Code (binary, ~/.claude, ~/.claude.json) across Unraid reboots.
#
# The live copy stays in RAM (/root) so Claude keeps working while the array
# is stopped and never holds files open on /mnt/user. The persisted copy lives
# in appdata and is synced from RAM periodically and when the array stops.
#
# Nothing here ever creates paths under /mnt/user unless shfs is mounted:
# a stray /mnt/user on rootfs stops emhttpd from mounting user shares.
#
# Usage: claude-persist.sh restore|backup|status

set -u
export HOME=/root
export PATH="/root/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

SHARE_MOUNT=/mnt/user
PERSIST=/mnt/user/appdata/claude-code
LIVE_DIR=/root/.claude
LIVE_JSON=/root/.claude.json
# Lives in RAM outside ~/.claude: resets every boot and is never synced.
MARKER=/root/.claude-persist-restored
BIN_LINK=/root/.local/bin/claude
VERSIONS=/root/.local/share/claude/versions
LOCK=/var/run/claude-persist.lock
# Per-session scratch that is not worth persisting.
EXCLUDES=(--exclude=/shell-snapshots/ --exclude=/session-env/ --exclude=/cache/
          --exclude=/downloads/ --exclude=/paste-cache/)

log() { echo "claude-persist: $*"; logger -t claude-persist -- "$*"; }

shares_mounted() { mountpoint -q "$SHARE_MOUNT" && [ -d "$SHARE_MOUNT/appdata" ]; }

wait_for_shares() {
    local i
    for i in $(seq 1 24); do
        shares_mounted && return 0
        sleep 5
    done
    return 1
}

# Deep-merge two JSON files (live values win) into the live path.
merge_json() {
    local persisted=$1 live=$2 tmp
    if [ ! -f "$live" ]; then
        cp -p "$persisted" "$live"
        return
    fi
    tmp=$(mktemp "$live.XXXXXX")
    if jq -s '.[0] * .[1]' "$persisted" "$live" >"$tmp" 2>/dev/null; then
        chmod --reference="$live" "$tmp"
        mv -f "$tmp" "$live"
    else
        rm -f "$tmp"
        log "could not merge $(basename "$live") (invalid JSON?); kept live copy"
    fi
}

restore_binary() {
    local newest ver
    if [ ! -x "$BIN_LINK" ]; then
        mkdir -p /root/.local/bin "$VERSIONS"
        newest=$(ls -1 "$VERSIONS" 2>/dev/null | sort -V | tail -n 1)
        if [ -n "$newest" ] && [ -x "$VERSIONS/$newest" ]; then
            ln -sfn "$VERSIONS/$newest" "$BIN_LINK"
        elif [ -x "$PERSIST/bin/claude" ]; then
            ver=$(cat "$PERSIST/bin/.version" 2>/dev/null)
            [ -n "$ver" ] || ver=$("$PERSIST/bin/claude" --version 2>/dev/null | awk '{print $1}')
            [ -n "$ver" ] || ver=cached
            cp "$PERSIST/bin/claude" "$VERSIONS/$ver"
            chmod 755 "$VERSIONS/$ver"
            ln -sfn "$VERSIONS/$ver" "$BIN_LINK"
            log "restored binary $ver from appdata cache"
        else
            log "no cached binary; running official installer"
            curl -fsSL https://claude.ai/install.sh | bash || log "installer failed"
        fi
    fi
    [ -x "$BIN_LINK" ] && ln -sfn "$BIN_LINK" /usr/local/bin/claude
}

restore() {
    if ! wait_for_shares; then
        log "user shares not mounted; not touching $SHARE_MOUNT"
        return 1
    fi
    exec 9>"$LOCK"
    flock 9
    mkdir -p "$PERSIST"
    chown root:root "$PERSIST"
    chmod 700 "$PERSIST"

    restore_binary

    if [ -e "$MARKER" ]; then
        log "already restored this boot; RAM copy is authoritative"
        return 0
    fi

    if [ -d "$PERSIST/.claude" ]; then
        mkdir -p "$LIVE_DIR"
        rsync -a --update "${EXCLUDES[@]}" --exclude=/settings.json --exclude=/history.jsonl \
            "$PERSIST/.claude/" "$LIVE_DIR/"
        [ -f "$PERSIST/.claude/settings.json" ] && merge_json "$PERSIST/.claude/settings.json" "$LIVE_DIR/settings.json"
        if [ -f "$PERSIST/.claude/history.jsonl" ]; then
            local tmp
            tmp=$(mktemp "$LIVE_DIR/history.jsonl.XXXXXX")
            cat "$PERSIST/.claude/history.jsonl" "$LIVE_DIR/history.jsonl" 2>/dev/null | awk '!seen[$0]++' >"$tmp"
            chmod 600 "$tmp"
            mv -f "$tmp" "$LIVE_DIR/history.jsonl"
        fi
    fi
    [ -f "$PERSIST/.claude.json" ] && merge_json "$PERSIST/.claude.json" "$LIVE_JSON"

    touch "$MARKER"
    log "restored Claude Code config from $PERSIST"
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
    mkdir -p "$PERSIST/.claude"
    chown root:root "$PERSIST"
    chmod 700 "$PERSIST"

    [ -d "$LIVE_DIR" ] && rsync -a --delete "${EXCLUDES[@]}" "$LIVE_DIR/" "$PERSIST/.claude/"
    if [ -f "$LIVE_JSON" ]; then
        cp -p "$LIVE_JSON" "$PERSIST/.claude.json.tmp" && mv -f "$PERSIST/.claude.json.tmp" "$PERSIST/.claude.json"
    fi

    # Refresh the cached binary when the auto-updater has installed a new version.
    local real ver
    real=$(readlink -f "$BIN_LINK" 2>/dev/null)
    if [ -n "$real" ] && [ -x "$real" ]; then
        ver=$(basename "$real")
        if [ "$(cat "$PERSIST/bin/.version" 2>/dev/null)" != "$ver" ]; then
            mkdir -p "$PERSIST/bin"
            cp "$real" "$PERSIST/bin/claude.tmp" && mv -f "$PERSIST/bin/claude.tmp" "$PERSIST/bin/claude"
            echo "$ver" >"$PERSIST/bin/.version"
            log "cached binary $ver"
        fi
    fi
    log "backed up Claude Code config to $PERSIST"
}

status() {
    echo "shares mounted:  $(shares_mounted && echo yes || echo no)"
    echo "restored (boot): $([ -e "$MARKER" ] && echo "yes ($(date -r "$MARKER"))" || echo no)"
    echo "binary:          $(readlink -f "$BIN_LINK" 2>/dev/null || echo missing)"
    echo "cached binary:   $(cat "$PERSIST/bin/.version" 2>/dev/null || echo unknown)"
    echo "persisted:       $(du -sh "$PERSIST" 2>/dev/null | cut -f1) in $PERSIST"
}

case "${1:-}" in
    restore) restore ;;
    backup) backup ;;
    status) status ;;
    *) echo "Usage: $0 restore|backup|status" >&2; exit 2 ;;
esac
