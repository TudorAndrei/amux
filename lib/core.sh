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
    local format="$1"
    if [ -n "${TMUX:-}" ] && command -v tmux >/dev/null 2>&1; then
        tmux display-message -p "$format" 2>/dev/null || true
    fi
}
