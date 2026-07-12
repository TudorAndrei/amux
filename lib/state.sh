#!/usr/bin/env bash

amux_initial_state() {
    printf '{"version":1,"records":{}}\n'
}

amux_state_json() {
    local state_file
    state_file="$(amux_state_file)"
    if [ -f "$state_file" ]; then
        cat "$state_file"
    else
        amux_initial_state
    fi
}

amux_normalize_event() {
    local agent="$1" event="$2" status="$3" attention="$4" reason="$5" raw="$6"
    local now iso tmux_session tmux_window tmux_pane cwd

    now="$(amux_now)"
    iso="$(amux_iso_now)"
    tmux_session="$(amux_tmux_value '#{session_name}')"
    tmux_window="$(amux_tmux_value '#{window_id}')"
    tmux_pane="$(amux_tmux_value '#{pane_id}')"
    cwd="$(pwd)"

    jq -cn \
        --arg agent "$agent" \
        --arg event "$event" \
        --arg status_override "$status" \
        --arg attention_override "$attention" \
        --arg reason_override "$reason" \
        --argjson raw "$raw" \
        --arg now "$now" \
        --arg iso "$iso" \
        --arg tmux_session "$tmux_session" \
        --arg tmux_window "$tmux_window" \
        --arg tmux_pane "$tmux_pane" \
        --arg cwd "$cwd" '
        def raw_event:
          if $event != "" then $event
          else ($raw.hook_event_name // $raw.event.type // $raw.type // "activity")
          end;

        def raw_session:
          $raw.session_id // $raw.sessionID // $raw.sessionId // $raw.session.id // $raw.id // "";

        def raw_cwd:
          $raw.cwd // $raw.directory // $raw.project.directory // $cwd;

        def inferred_attention($ev):
          ($ev | ascii_downcase) as $lower
          | ($lower | test("permission|approval|notification|idle|ask|prompt|waiting"));

        def inferred_status($ev; $attn):
          if $status_override != "" then $status_override
          elif $attn then "attention"
          elif ($ev | ascii_downcase | test("stop|end|idle|done|complete")) then "done"
          else "running"
          end;

        raw_event as $ev
        | (if $attention_override == "" then inferred_attention($ev)
           else ($attention_override == "1" or ($attention_override | ascii_downcase) == "true")
           end) as $attn
        | inferred_status($ev; $attn) as $st
        | {
            key: (if raw_session != "" or $tmux_session != "" or $tmux_pane != ""
                  then [$agent, raw_session, $tmux_session, $tmux_pane]
                  else [$agent, raw_cwd]
                  end | map(select(. != "")) | join(":")),
            agent: $agent,
            agent_session_id: raw_session,
            tmux_session: $tmux_session,
            tmux_window: $tmux_window,
            tmux_pane: $tmux_pane,
            cwd: raw_cwd,
            status: $st,
            attention: $attn,
            reason: (if $reason_override != "" then $reason_override
                     else ($raw.reason // $raw.message // $raw.notificationType // $ev)
                     end),
            last_event: $ev,
            updated_at: ($now | tonumber),
            updated_at_iso: $iso,
            raw: $raw
          }'
}

amux_write_event() {
    local agent="$1" event="$2" status="$3" attention="$4" reason="$5" raw="$6"
    local state_file events_file tmp record cutoff

    amux_mkdir_state
    state_file="$(amux_state_file)"
    events_file="$(amux_events_file)"
    record="$(amux_normalize_event "$agent" "$event" "$status" "$attention" "$reason" "$raw")"
    cutoff="$(($(amux_now) - $(amux_stale_seconds)))"

    printf '%s\n' "$record" >> "$events_file"
    tmp="${state_file}.$$"
    amux_state_json |
        jq --argjson record "$record" --argjson cutoff "$cutoff" '
          .version = 1
          | .records[$record.key] = ($record | del(.key))
          | .records |= with_entries(select((.value.updated_at // 0) >= $cutoff))
        ' > "$tmp"
    mv "$tmp" "$state_file"
}

amux_records_jq() {
    local filter="$1"
    amux_state_json | jq -r "$filter"
}

amux_list() {
    local cutoff
    cutoff="$(($(amux_now) - $(amux_stale_seconds)))"
    amux_state_json | jq -r --argjson cutoff "$cutoff" '
      .records
      | to_entries
      | map(select((.value.updated_at // 0) >= $cutoff))
      | sort_by([(.value.attention | not), .value.agent, .value.updated_at])
      | reverse
      | .[]
      | [.value.agent, .value.status, (.value.tmux_session // "-"), (.value.tmux_pane // "-"), (.value.reason // "-"), (.value.cwd // "-")]
      | @tsv
    '
}

amux_sessions() {
    local cutoff hide_subagents state sessions panes
    cutoff="$(($(amux_now) - $(amux_stale_seconds)))"
    hide_subagents="$(amux_hide_subagents)"
    state="$(amux_state_json)"
    sessions="$(
        if command -v tmux >/dev/null 2>&1; then
            tmux list-sessions -F '#{session_last_attached}|#{?session_attached,,#{session_name}}|#{session_attached}' 2>/dev/null || true
        fi
    )"
    panes="$(
        if command -v tmux >/dev/null 2>&1; then
            tmux list-panes -a -F '#{session_name}|#{pane_id}|#{pane_current_command}|#{pane_pid}|#{pane_title}' 2>/dev/null || true
        fi
    )"

    jq -n \
        --slurpfile state <(printf '%s' "$state") \
        --arg sessions "$sessions" \
        --arg panes "$panes" \
        --argjson cutoff "$cutoff" \
        --argjson hide_subagents "$hide_subagents" '
        $state[0] as $state
        |
        def uuid_like:
          test("^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$");

        def subagent_session:
          if $hide_subagents then
            ((.session // "") | uuid_like)
          else
            false
          end;

        def subagent_record:
          if $hide_subagents then
            (((.tmux_session // "") | uuid_like)
              or ((.raw.agent_id // "") != "")
              or ((.raw.agent_type // "") != "")
              or ((.raw.parent_agent_id // "") != "")
              or ((.raw.parent_session_id // "") != "")
              or (.raw.is_subagent == true)
              or (((.tmux_session // "") == "")
                and ((.cwd // "") == "")
                and ((.agent_session_id // "") | uuid_like)))
          else
            false
          end;

        def session_rows:
          $sessions
          | split("\n")
          | map(select(length > 0))
          | map(split("|") as $p | {
              session: $p[1],
              last_attached: (($p[0] // "0") | tonumber),
              attached: (($p[2] // "0") == "1")
            })
          | map(select(.session != ""))
          | map(select(subagent_session | not));

        def pane_rows:
          $panes
          | split("\n")
          | map(select(length > 0))
          | map(split("|") as $p | {
              session: $p[0],
              pane: $p[1],
              command: $p[2],
              pid: $p[3],
              title: $p[4]
            });

        def is_agent:
          (.command // "") as $cmd
          | ($cmd | test("^(claude|codex.*|pi|opencode)$"));

        def agent_name:
          (.command // "") as $cmd
          | if $cmd | test("^codex") then "codex"
            elif $cmd == "claude" then "claude"
            elif $cmd == "pi" then "pi"
            elif $cmd == "opencode" then "opencode"
            else $cmd
            end;

        def hook_records:
          $state.records
          | to_entries
          | map(.value)
          | map(select((.updated_at // 0) >= $cutoff))
          | map(select((.tmux_session // "") != ""))
          | map(select(subagent_record | not));

        def rank($status):
          if $status == "attention" then 3
          elif $status == "done" then 2
          elif $status == "running" then 1
          elif $status == "offline" then 0
          else -1
          end;

        def record_group_key:
          if (.tmux_session // "") != "" then .tmux_session
          elif (.cwd // "") != "" then .cwd
          elif (.agent_session_id // "") != "" then .agent_session_id
          else .agent
          end;

        hook_records as $records
        | session_rows as $tmux_sessions
        | pane_rows as $panes
        | $tmux_sessions as $sessions
        | ($panes | map(select(is_agent))) as $agent_panes
        | ($agent_panes | map(.pane)) as $live_panes
        | $sessions
        | map(. as $session
          | ($agent_panes | map(select(.session == $session.session))) as $session_agent_panes
          | ($records | map(select(record_group_key == $session.session))) as $session_records
          | ($session_records
              | map(if (.tmux_pane // "") != "" and ((.tmux_pane as $pane | $live_panes | index($pane)) | not)
                    then . + {derived_status: "offline"}
                    else . + {derived_status: .status}
                    end)) as $derived_records
          | ($derived_records | map(select(.derived_status == "attention")) | length) as $attention_count
          | ($derived_records | map(select(.derived_status == "running")) | length) as $running_count
          | ($derived_records | map(select(.derived_status == "done")) | length) as $done_count
          | ($derived_records | map(select(.derived_status == "offline")) | length) as $offline_count
          | ($session_agent_panes | length) as $live_count
          | (if $attention_count > 0 then "attention"
             elif $running_count > 0 then "running"
             elif $done_count > 0 then "done"
             elif $live_count > 0 then "running"
             elif $offline_count > 0 then "offline"
             else "none"
             end) as $status
          | ($derived_records | sort_by(.updated_at // 0) | reverse | .[0]) as $latest_record
          | ($session_agent_panes | .[0]) as $first_live
          | $session + {
              status: $status,
              attention: ($status == "attention"),
              agent_count: $live_count,
              pane: (($first_live.pane // $latest_record.tmux_pane) // ""),
              reason: (if $status == "offline" then "offline" else (($latest_record.reason // $first_live.title) // "") end),
              cwd: (($latest_record.cwd // "") ),
              updated_at: (($latest_record.updated_at // .last_attached) | tonumber)
            })
        | sort_by([-(rank(.status)), -(.last_attached // 0)])
    '
}

amux_status() {
    local cutoff output
    cutoff="$(($(amux_now) - $(amux_stale_seconds)))"
    output="$(amux_sessions | jq -r '
      def tmux_color($use_color; $style; $text):
        if $use_color then "#[\($style)]\($text)#[default]" else $text end;

      (map(select(.status == "attention")) | length) as $attention
      | (map(select(.status == "running")) | length) as $running
      | (map(select(.status == "offline")) | length) as $offline
      | (env.AMUX_COLOR // "1") as $color
      | (env.AMUX_PLAIN // "0") as $plain
      | (env.NO_COLOR // "") as $no_color
      | ($color != "0" and $plain != "1" and $no_color == "") as $use_color
      | if $attention > 0 then "\(tmux_color($use_color; "fg=red,bold"; "▲")) \($attention)"
        elif $running > 0 then "\(tmux_color($use_color; "fg=yellow"; "◐")) \($running)"
        elif $offline > 0 then "\(tmux_color($use_color; "fg=colour244"; "○")) \($offline)"
        elif length > 0 then tmux_color($use_color; "fg=green"; "●")
        else ""
        end
    ')"
    [ -n "$output" ] && printf '%s' "$output"
    return 0
}
