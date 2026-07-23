#!/usr/bin/env bash

amux_die() {
    printf 'amux: %s\n' "$*" >&2
    exit 1
}

amux_need_jq() {
    command -v jq >/dev/null 2>&1 || amux_die "jq is required"
}

amux_state_dir() {
    if [ -n "${AMUX_STATE_DIR:-}" ]; then
        printf '%s\n' "$AMUX_STATE_DIR"
    elif [ -n "${XDG_STATE_HOME:-}" ]; then
        printf '%s/amux\n' "$XDG_STATE_HOME"
    else
        printf '%s/.local/state/amux\n' "$HOME"
    fi
}

amux_state_file() {
    printf '%s/state.json\n' "$(amux_state_dir)"
}

amux_events_file() {
    printf '%s/events.jsonl\n' "$(amux_state_dir)"
}

amux_state_lock_dir() {
    printf '%s/state.lock\n' "$(amux_state_dir)"
}

amux_stale_seconds() {
    printf '%s\n' "${AMUX_STALE_SECONDS:-86400}"
}

amux_hide_subagents() {
    case "${AMUX_HIDE_SUBAGENTS:-1}" in
        0|false|FALSE|no|NO|off|OFF)
            printf 'false\n'
            ;;
        *)
            printf 'true\n'
            ;;
    esac
}

amux_now() {
    date +%s
}

amux_iso_now() {
    date -u '+%Y-%m-%dT%H:%M:%SZ'
}

amux_mkdir_state() {
    mkdir -p "$(amux_state_dir)"
}

amux_tmux_value() {
    local format="$1" pane="${2:-${TMUX_PANE:-}}"
    if [ -n "${TMUX:-}" ] && command -v tmux >/dev/null 2>&1; then
        if [ -n "$pane" ]; then
            tmux display-message -p -t "$pane" "$format" 2>/dev/null || true
        else
            tmux display-message -p "$format" 2>/dev/null || true
        fi
    fi
}

amux_acquire_state_lock() {
    local lock_path="$1" attempts=0

    if command -v shlock >/dev/null 2>&1; then
        while ! shlock -p "$$" -f "$lock_path" 2>/dev/null; do
            attempts=$((attempts + 1))
            [ "$attempts" -lt 500 ] || amux_die "timed out waiting for state lock"
            sleep 0.01
        done
    else
        while ! mkdir "$lock_path" 2>/dev/null; do
            attempts=$((attempts + 1))
            [ "$attempts" -lt 500 ] || amux_die "timed out waiting for state lock"
            sleep 0.01
        done
    fi
}

amux_release_state_lock() {
    local lock_path="$1"
    if [ -d "$lock_path" ]; then
        rmdir "$lock_path" 2>/dev/null || true
    else
        rm -f "$lock_path"
    fi
}

amux_refresh_tmux_status() {
    if [ -n "${TMUX:-}" ] && command -v tmux >/dev/null 2>&1; then
        tmux refresh-client -S 2>/dev/null || true
    fi
}
