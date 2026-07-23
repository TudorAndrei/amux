#!/usr/bin/env bash
set -euo pipefail

AMUX_ROOT="${AMUX_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AMUX="$AMUX_ROOT/bin/amux"

message() {
    if command -v tmux >/dev/null 2>&1; then
        tmux display-message "$1" 2>/dev/null || printf '%s\n' "$1"
    else
        printf '%s\n' "$1"
    fi
}

target="$(
    "$AMUX" sessions --json |
        jq -r '
          [.[] | .agents[]]
          | map(select(.attention == true and .live == true))
          | sort_by(.updated_at)
          | reverse
          | .[0] // empty
          | [.session // "", .pane // ""]
          | @tsv
        '
)"

if [ -z "$target" ]; then
    message "amux: no agents need attention"
    exit 0
fi

session="$(printf '%s' "$target" | cut -f1)"
pane="$(printf '%s' "$target" | cut -f2)"

if [ -z "$session" ] && [ -z "$pane" ]; then
    message "amux: attention record has no tmux target"
    exit 0
fi

if [ -n "$session" ]; then
    tmux switch-client -t "$session"
fi

if [ -n "$pane" ]; then
    tmux select-pane -t "$pane"
fi
