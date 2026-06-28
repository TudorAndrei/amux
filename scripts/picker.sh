#!/usr/bin/env bash
set -euo pipefail

AMUX_ROOT="${AMUX_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AMUX="$AMUX_ROOT/bin/amux"

age() {
    local updated now delta
    updated="$1"
    now="$(date +%s)"
    delta=$((now - updated))

    if [ "$delta" -lt 60 ]; then
        printf '%ss' "$delta"
    elif [ "$delta" -lt 3600 ]; then
        printf '%sm' "$((delta / 60))"
    elif [ "$delta" -lt 86400 ]; then
        printf '%sh' "$((delta / 3600))"
    else
        printf '%sd' "$((delta / 86400))"
    fi
}

rows="$("$AMUX" list --json | jq -r '
  .records
  | to_entries
  | sort_by(.value.attention | not, .value.updated_at)
  | reverse
  | .[]
  | [
      .key,
      .value.agent,
      .value.status,
      (.value.tmux_session // ""),
      (.value.tmux_pane // ""),
      (.value.updated_at | tostring),
      (.value.reason // ""),
      (.value.cwd // "")
    ]
  | @tsv
')"

if [ -z "$rows" ]; then
    if command -v fzf >/dev/null 2>&1 && [ "${AMUX_PLAIN:-0}" != "1" ]; then
        printf 'No agent state yet\tStart Codex, Claude, Pi, or opencode with amux hooks installed\n' |
            fzf --disabled --reverse \
                --delimiter=$'\t' \
                --with-nth=1,2 \
                --header='amux   no tracked agents yet   press esc to close' >/dev/null || true
    else
        printf 'amux: no agent state yet\n\nPress any key to close.'
        IFS= read -r -n 1 _ 2>/dev/null || true
        printf '\n'
    fi
    exit 0
fi

display_rows="$(
    printf '%s\n' "$rows" |
        while IFS=$'\t' read -r key agent status session pane updated reason cwd; do
            case "$status" in
                attention) icon="▲" ;;
                running) icon="◐" ;;
                done) icon="●" ;;
                *) icon="·" ;;
            esac
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "$key" "$icon" "$agent" "$session" "$pane" "$(age "$updated")" "$reason" "$cwd"
        done
)"

if command -v fzf >/dev/null 2>&1 && [ "${AMUX_PLAIN:-0}" != "1" ]; then
    selected="$(
        printf '%s\n' "$display_rows" |
            fzf --ansi --reverse \
                --with-nth=2.. \
                --delimiter=$'\t' \
                --header='amux   ▲ attention  ◐ running  ● done' \
                --preview='printf "%s\n" {} | awk -F "\t" "{print \"agent: \" \$3 \"\nstatus: \" \$2 \" \" \$4 \"\nsession: \" \$5 \"\npane: \" \$6 \"\nage: \" \$7 \"\nreason: \" \$8 \"\ncwd: \" \$9}"'
    )" || exit 0
else
    printf '%s\n' "$display_rows" | cut -f2-
    exit 0
fi

[ -n "${selected:-}" ] || exit 0

session="$(printf '%s' "$selected" | cut -f4)"
pane="$(printf '%s' "$selected" | cut -f5)"

if [ -n "$pane" ]; then
    tmux select-pane -t "$pane" 2>/dev/null || true
fi

if [ -n "$session" ]; then
    tmux switch-client -t "$session" 2>/dev/null || true
fi
