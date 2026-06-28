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
            key: ([$agent, raw_session, $tmux_session, $tmux_pane, raw_cwd] | map(select(. != "")) | join(":")),
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
    local state_file events_file tmp record

    amux_mkdir_state
    state_file="$(amux_state_file)"
    events_file="$(amux_events_file)"
    record="$(amux_normalize_event "$agent" "$event" "$status" "$attention" "$reason" "$raw")"

    printf '%s\n' "$record" >> "$events_file"
    tmp="${state_file}.$$"
    amux_state_json |
        jq --argjson record "$record" '
          .version = 1
          | .records[$record.key] = ($record | del(.key))
        ' > "$tmp"
    mv "$tmp" "$state_file"
}

amux_records_jq() {
    local filter="$1"
    amux_state_json | jq -r "$filter"
}

amux_list() {
    amux_records_jq '
      .records
      | to_entries
      | sort_by([(.value.attention | not), .value.agent, .value.updated_at])
      | reverse
      | .[]
      | [.value.agent, .value.status, (.value.tmux_session // "-"), (.value.tmux_pane // "-"), (.value.reason // "-"), (.value.cwd // "-")]
      | @tsv
    '
}

amux_status() {
    amux_records_jq '
      (.records | to_entries | map(.value)) as $records
      | ($records | map(select(.attention == true)) | length) as $attention
      | ($records | map(select(.status == "running")) | length) as $running
      | if $attention > 0 then "▲ \($attention)"
        elif $running > 0 then "◐ \($running)"
        elif ($records | length) > 0 then "●"
        else ""
        end
    '
}
