#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export AMUX_ROOT="$ROOT"
TMPDIR="${TMPDIR:-/tmp}"
export AMUX_STATE_DIR
AMUX_STATE_DIR="$(mktemp -d "$TMPDIR/amux-test.XXXXXX")"
trap 'rm -rf "$AMUX_STATE_DIR"' EXIT
unset TMUX TMUX_PANE

"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
"$ROOT/bin/amux" event --agent claude < "$ROOT/tests/fixtures/claude-stop.json"
"$ROOT/bin/amux" event --agent opencode < "$ROOT/tests/fixtures/opencode-idle.json"
"$ROOT/bin/amux" event --agent pi < "$ROOT/tests/fixtures/pi-tool-call.json"
"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-subagent.json"
"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-subagent-empty-cwd.json"
printf '%s\n' '{"session_id":"prompt-session","hook_event_name":"UserPromptSubmit"}' |
    "$ROOT/bin/amux" event --agent codex --event UserPromptSubmit

state="$("$ROOT/bin/amux" list --json)"

printf '%s\n' "$state" | jq -e '.records | length == 7' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].attention] | any' >/dev/null
printf '%s\n' "$state" | jq -e '.records["codex:prompt-session"] | .status == "running" and .attention == false' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].agent] | sort == ["claude","codex","codex","codex","codex","opencode","pi"]' >/dev/null
"$ROOT/bin/amux" sessions --json | jq -e 'all(.session != "/tmp/amux-subagent-project")' >/dev/null
"$ROOT/bin/amux" sessions --json | jq -e 'all(.session != "70309dc1-b9b2-4826-99d8-6d3ff79d2c83")' >/dev/null
AMUX_HIDE_SUBAGENTS=0 "$ROOT/bin/amux" sessions --json | jq -e 'all(.session != "/tmp/amux-subagent-project")' >/dev/null
AMUX_HIDE_SUBAGENTS=0 "$ROOT/bin/amux" sessions --json | jq -e 'all(.session != "70309dc1-b9b2-4826-99d8-6d3ff79d2c83")' >/dev/null
test -z "$(AMUX_COLOR=0 "$ROOT/bin/amux" status)"

FAKE_BIN="$AMUX_STATE_DIR/bin"
mkdir -p "$FAKE_BIN"
cat > "$FAKE_BIN/tmux" <<'SH'
#!/usr/bin/env bash
case "$1" in
    display-message)
        format="${!#}"
        case "$format" in
            '#{session_name}') printf '%s\n' "${AMUX_FAKE_SESSION:-codex-tmux}" ;;
            '#{window_id}') printf '@1\n' ;;
            '#{pane_id}') printf '%s\n' "${AMUX_FAKE_PANE:-%1}" ;;
        esac
        ;;
    list-sessions)
        if [ "${AMUX_FAKE_MULTI:-0}" = "1" ]; then
            printf '500|multi-agent|0\n'
        elif [ "${AMUX_FAKE_STATUS_SESSIONS:-0}" = "1" ]; then
            printf '400|attention-session|0\n'
            printf '300|done-session|0\n'
            printf '200|running-session|0\n'
            printf '100|offline-session|0\n'
        else
            printf '100|codex-tmux|0\n'
        fi
        if [ "${AMUX_FAKE_EXTRA_SESSION:-0}" = "1" ]; then
            printf '50|codex-offline|0\n'
        fi
        ;;
    list-panes)
        if [ "${AMUX_FAKE_MULTI:-0}" = "1" ]; then
            printf 'multi-agent|%%20|codex|500|codex|/tmp/multi-codex\n'
            printf 'multi-agent|%%21|claude|501|claude|/tmp/multi-claude\n'
        elif [ "${AMUX_FAKE_STATUS_SESSIONS:-0}" = "1" ]; then
            printf 'attention-session|%%10|codex|400|codex|/tmp/attention\n'
            printf 'done-session|%%11|codex|300|codex|/tmp/done\n'
            printf 'running-session|%%12|codex|200|codex|/tmp/running\n'
            printf 'offline-session|%%13|zsh|100|shell|/tmp/offline\n'
        elif [ "${AMUX_FAKE_AGENT_LIVE:-0}" = "1" ]; then
            printf 'codex-tmux|%%1|codex|200|codex|/tmp/codex\n'
        else
            printf 'codex-tmux|%%2|zsh|200|shell|/tmp/shell\n'
        fi
        if [ "${AMUX_FAKE_EXTRA_SESSION:-0}" = "1" ]; then
            printf 'codex-offline|%%4|zsh|201|shell|/tmp/offline\n'
        fi
        ;;
    refresh-client)
        ;;
esac
SH
chmod +x "$FAKE_BIN/tmux"

export PATH="$FAKE_BIN:$PATH"
export TMUX="fake"
"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
unset TMUX
"$ROOT/bin/amux" event --agent claude < "$ROOT/tests/fixtures/claude-stop.json"

sessions="$("$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e 'length == 1' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].session == "codex-tmux"' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].status == "offline"' >/dev/null
printf '%s\n' "$sessions" | jq -e 'all(.session != "/tmp/amux-claude")' >/dev/null

export TMUX="fake"
AMUX_FAKE_AGENT_LIVE=1 "$ROOT/bin/amux" event --agent codex --event Stop < "$ROOT/tests/fixtures/codex-permission.json"
unset TMUX

sessions="$(AMUX_FAKE_AGENT_LIVE=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e 'length == 1' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].session == "codex-tmux"' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].status == "done"' >/dev/null

export TMUX="fake"
AMUX_FAKE_SESSION=codex-offline AMUX_FAKE_PANE=%3 "$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
unset TMUX

sessions="$(AMUX_FAKE_AGENT_LIVE=1 AMUX_FAKE_EXTRA_SESSION=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e 'length == 2' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].session == "codex-tmux"' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[0].status == "done"' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[1].session == "codex-offline"' >/dev/null
printf '%s\n' "$sessions" | jq -e '.[1].status == "offline"' >/dev/null

export TMUX="fake"
AMUX_FAKE_SESSION=attention-session AMUX_FAKE_PANE=%10 "$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
AMUX_FAKE_SESSION=done-session AMUX_FAKE_PANE=%11 "$ROOT/bin/amux" event --agent codex --event Stop < "$ROOT/tests/fixtures/codex-permission.json"
AMUX_FAKE_SESSION=running-session AMUX_FAKE_PANE=%12 "$ROOT/bin/amux" event --agent codex --event PostToolUse < "$ROOT/tests/fixtures/codex-permission.json"
AMUX_FAKE_SESSION=offline-session AMUX_FAKE_PANE=%13 "$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
unset TMUX

sessions="$(AMUX_FAKE_STATUS_SESSIONS=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e '[.[] | .status] == ["attention", "done", "running", "offline"]' >/dev/null

export TMUX=fake
printf '%s\n' '{"session_id":"codex-old","cwd":"/tmp/multi-codex"}' |
    TMUX_PANE=%20 AMUX_FAKE_SESSION=multi-agent AMUX_FAKE_PANE=%99 \
    "$ROOT/bin/amux" event --agent codex --event PostToolUse
printf '%s\n' '{"session_id":"claude-old","cwd":"/tmp/multi-claude"}' |
    TMUX_PANE=%21 AMUX_FAKE_SESSION=multi-agent \
    "$ROOT/bin/amux" event --agent claude --event Notification --attention 1 --reason notification
unset TMUX

state="$("$ROOT/bin/amux" list --json)"
printf '%s\n' "$state" | jq -e '.records["codex:multi-agent:%20"].tmux_pane == "%20"' >/dev/null

sessions="$(AMUX_FAKE_MULTI=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e '
  length == 1
  and .[0].session == "multi-agent"
  and .[0].status == "attention"
  and .[0].agent_count == 2
  and .[0].live_agent_count == 2
  and .[0].pane == "%21"
  and (.[0].agents | map([.agent, .pane, .status]) | sort)
      == [["claude","%21","attention"],["codex","%20","running"]]
' >/dev/null

export TMUX=fake
printf '%s\n' '{"session_id":"codex-new","cwd":"/tmp/multi-codex"}' |
    TMUX_PANE=%20 AMUX_FAKE_SESSION=multi-agent \
    "$ROOT/bin/amux" event --agent codex --event Stop
printf '%s\n' '{"session_id":"claude-new","cwd":"/tmp/multi-claude"}' |
    TMUX_PANE=%21 AMUX_FAKE_SESSION=multi-agent \
    "$ROOT/bin/amux" event --agent claude --event PostToolUse
unset TMUX

sessions="$(AMUX_FAKE_MULTI=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e '
  .[0].status == "running"
  and .[0].pane == "%21"
  and (.[0].agents | map(select(.agent == "codex")) | .[0].status) == "done"
  and (.[0].agents | map(select(.agent == "codex")) | .[0].agent_session_id) == "codex-new"
' >/dev/null

export TMUX=fake
printf '%s\n' '{"session_id":"claude-new","cwd":"/tmp/multi-claude"}' |
    TMUX_PANE=%21 AMUX_FAKE_SESSION=multi-agent \
    "$ROOT/bin/amux" event --agent claude --event Stop
unset TMUX

sessions="$(AMUX_FAKE_MULTI=1 "$ROOT/bin/amux" sessions --json)"
printf '%s\n' "$sessions" | jq -e '.[0].status == "done" and ([.[0].agents[].status] | all(. == "done"))' >/dev/null
test "$(AMUX_FAKE_MULTI=1 AMUX_COLOR=0 "$ROOT/bin/amux" status)" = "● 2"
picker_rows="$(AMUX_FAKE_MULTI=1 AMUX_COLOR=0 "$ROOT/scripts/picker.sh" --rows)"
printf '%s\n' "$picker_rows" | awk -F '\t' '
  $1 == "multi-agent" && $2 == "%20" && $4 == "codex" { codex = 1 }
  $1 == "multi-agent" && $2 == "%21" && $4 == "claude" { claude = 1 }
  END { exit !(codex && claude && NR == 2) }
'

unset TMUX TMUX_PANE
RACE_STATE_DIR="$(mktemp -d "$TMPDIR/amux-race.XXXXXX")"
for i in $(seq 1 40); do
    (
        printf '{"session_id":"race-%s"}\n' "$i" |
            AMUX_STATE_DIR="$RACE_STATE_DIR" "$ROOT/bin/amux" event --agent codex --event PostToolUse
    ) &
done
wait
jq -e '.records | length == 40' "$RACE_STATE_DIR/state.json" >/dev/null
test "$(wc -l < "$RACE_STATE_DIR/events.jsonl" | tr -d ' ')" = "40"
test ! -e "$RACE_STATE_DIR/state.lock"
rm -rf "$RACE_STATE_DIR"

printf 'ok\n'
