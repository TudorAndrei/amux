#!/usr/bin/env bash
set -euo pipefail

AMUX_ROOT="${AMUX_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
AMUX="$AMUX_ROOT/bin/amux"

rows="$("$AMUX" list --json | jq -r '
  def group_key:
    if (.tmux_session // "") != "" then .tmux_session
    elif (.cwd // "") != "" then .cwd
    elif (.agent_session_id // "") != "" then .agent_session_id
    else .agent
    end;
  .records
  | to_entries
  | map(.value)
  | group_by(group_key)
  | map(max_by(.updated_at))
  | sort_by([(if .attention then 0 elif .status == "running" then 1 else 2 end), -(.updated_at // 0)])
  | .[]
  | [
      (.tmux_session // .cwd // .agent_session_id // .agent),
      (.status // ""),
      (.tmux_session // ""),
      (.tmux_pane // ""),
      (.updated_at | tostring),
      (.reason // ""),
      (.cwd // "")
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
  def group_key:
    if (.tmux_session // "") != "" then .tmux_session
    elif (.cwd // "") != "" then .cwd
    elif (.agent_session_id // "") != "" then .agent_session_id
    else .agent
    end;
  def session_status:
    if any(.attention == true) then "attention"
    elif any(.status == "running") then "running"
    elif length > 0 then "done"
    else "unknown"
    end;
  def age($updated):
    ($now - $updated) as $delta
    | if $delta < 60 then "\($delta)s"
      elif $delta < 3600 then "\(($delta / 60) | floor)m"
      elif $delta < 86400 then "\(($delta / 3600) | floor)h"
      else "\(($delta / 86400) | floor)d"
      end;
  .records
  | to_entries
  | map(.value)
  | group_by(group_key)
  | map(
      . as $group
      | ($group | session_status) as $status
      | (if $status == "attention" then ($group | map(select(.attention == true)) | max_by(.updated_at))
         elif $status == "running" then ($group | map(select(.status == "running")) | max_by(.updated_at))
         else ($group | max_by(.updated_at))
         end) + {session_status: $status}
    )
  | sort_by([(if .session_status == "attention" then 0 elif .session_status == "running" then 1 else 2 end), -(.updated_at // 0)])
  | .[]
  | (.session_status // "unknown") as $status
  | (if $status == "attention" then "▲"
     elif $status == "running" then "◐"
     elif $status == "done" then "●"
     else "·"
     end) as $icon
  | [
      (.tmux_session // .cwd // .agent_session_id // .agent),
      $icon,
      (.tmux_session // .cwd // .agent_session_id // .agent),
      (.tmux_pane // ""),
      age(.updated_at),
      (.reason // ""),
      (.cwd // "")
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
                --preview='printf "%s\n" {} | awk -F "\t" "{print \"session: \" \$3 \"\nstatus: \" \$2 \"\npane: \" \$4 \"\nage: \" \$5 \"\nreason: \" \$6 \"\ncwd: \" \$7}"'
    )" || exit 0
else
    printf '%s\n' "$display_rows" | cut -f2-
    exit 0
fi

[ -n "${selected:-}" ] || exit 0

session="$(printf '%s' "$selected" | cut -f3)"
pane="$(printf '%s' "$selected" | cut -f4)"

if [ -n "$pane" ]; then
    tmux select-pane -t "$pane" 2>/dev/null || true
fi

if [ -n "$session" ]; then
    tmux switch-client -t "$session" 2>/dev/null || true
fi
