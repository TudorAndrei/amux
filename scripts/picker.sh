#!/usr/bin/env bash
set -euo pipefail

AMUX_ROOT="${AMUX_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AMUX="$AMUX_ROOT/bin/amux"

amux_use_color=1
if [ "${AMUX_COLOR:-1}" = "0" ] || [ "${AMUX_PLAIN:-0}" = "1" ] || [ -n "${NO_COLOR:-}" ]; then
    amux_use_color=0
fi

render_rows() {
    "$AMUX" sessions --json |
        jq -r --argjson now "$(date +%s)" '
        def age($updated):
          ($now - $updated) as $delta
          | if $delta < 60 then "\($delta)s"
            elif $delta < 3600 then "\(($delta / 60) | floor)m"
            elif $delta < 86400 then "\(($delta / 3600) | floor)h"
            else "\(($delta / 86400) | floor)d"
            end;
        def clean_reason:
          (.reason // "") as $reason
          | if $reason == (.session // "") then "" else $reason end;
        .[]
        | (.status // "none") as $status
        | (if $status == "attention" then "▲"
           elif $status == "running" then "◐"
           elif $status == "done" then "●"
           elif $status == "offline" then "○"
           else "·"
           end) as $icon
        | [
            .session,
            (.pane // ""),
            (.cwd // ""),
            $status,
            $icon,
            .session,
            age(.last_attached),
            clean_reason
          ]
        | @tsv
      ' |
        awk -F '\t' -v color="$amux_use_color" '
        function icon_for(status, icon) {
            if (!color) return icon
            if (status == "attention") return "\033[31;1m" icon "\033[0m"
            if (status == "running") return "\033[33m" icon "\033[0m"
            if (status == "done") return "\033[32m" icon "\033[0m"
            if (status == "offline") return "\033[38;5;244m" icon "\033[0m"
            return icon
        }
        {
            printf "%s\t%s\t%s\t%s %-34.34s %5s  %.90s\n", \
                $1, $2, $3, icon_for($4, $5), $6, $7, $8
        }'
}

if [ "${1:-}" = "--rows" ]; then
    render_rows
    exit 0
fi

display_rows="$(render_rows)"

if command -v fzf >/dev/null 2>&1 && [ "${AMUX_PLAIN:-0}" != "1" ]; then
    if [ "$amux_use_color" = "1" ]; then
        header=$'amux   \033[31;1m▲ attention\033[0m  \033[32m● done\033[0m  \033[33m◐ running\033[0m  \033[38;5;244m○ offline\033[0m'
    else
        header='amux   ▲ attention  ● done  ◐ running  ○ offline'
    fi
    rows_command="$(printf '%q ' "$0" --rows)"
    refresh_header='ctrl-r: refresh'
    periodic_refresh="ctrl-r:reload:$rows_command"
    if printf '__amux__\n' |
        fzf --filter=__amux__ --bind='every(3600):abort' >/dev/null 2>&1; then
        refresh_header='live refresh: 1s   ctrl-r: refresh now'
        periodic_refresh="every(1):reload:$rows_command"
    fi

    selected="$(
        printf '%s\n' "$display_rows" |
            fzf --ansi --reverse \
                --with-nth=4 \
                --delimiter=$'\t' \
                --nth=1 \
                --header="$header   $refresh_header" \
                --bind="$periodic_refresh" \
                --bind="ctrl-r:reload:$rows_command" \
                --preview='printf "%s\n" {} | awk -F "\t" "{print \"session: \" \$1 \"\nrow: \" \$4 \"\npane: \" \$2 \"\ncwd: \" \$3}"'
    )" || exit 0
else
    printf '%s\n' "$display_rows" | cut -f4
    exit 0
fi

[ -n "${selected:-}" ] || exit 0

session="$(printf '%s' "$selected" | cut -f1)"
pane="$(printf '%s' "$selected" | cut -f2)"

if [ -n "$pane" ]; then
    tmux select-pane -t "$pane" 2>/dev/null || true
fi

if [ -n "$session" ]; then
    tmux switch-client -t "$session" 2>/dev/null || true
fi
