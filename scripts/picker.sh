#!/usr/bin/env bash
set -euo pipefail

AMUX_ROOT="${AMUX_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AMUX="$AMUX_ROOT/bin/amux"

rows="$("$AMUX" list --json | jq -r '
  .records
  | to_entries
  | sort_by([(.value.attention | not), .value.updated_at])
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

display_rows="$("$AMUX" list --json | jq -r --argjson now "$(date +%s)" '
  def age($updated):
    ($now - $updated) as $delta
    | if $delta < 60 then "\($delta)s"
      elif $delta < 3600 then "\(($delta / 60) | floor)m"
      elif $delta < 86400 then "\(($delta / 3600) | floor)h"
      else "\(($delta / 86400) | floor)d"
      end;
  .records
  | to_entries
  | sort_by([(.value.attention | not), .value.updated_at])
  | reverse
  | .[]
  | (.value.status // "unknown") as $status
  | (if $status == "attention" then "▲"
     elif $status == "running" then "◐"
     elif $status == "done" then "●"
     else "·"
     end) as $icon
  | [
      .key,
      $icon,
      .value.agent,
      (.value.tmux_session // ""),
      (.value.tmux_pane // ""),
      age(.value.updated_at),
      (.value.reason // ""),
      (.value.cwd // "")
    ]
  | @tsv
')"

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
