#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export AMUX_ROOT="$ROOT"
TMPDIR="${TMPDIR:-/tmp}"
export AMUX_STATE_DIR
AMUX_STATE_DIR="$(mktemp -d "$TMPDIR/amux-test.XXXXXX")"
trap 'rm -rf "$AMUX_STATE_DIR"' EXIT

"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
"$ROOT/bin/amux" event --agent claude < "$ROOT/tests/fixtures/claude-stop.json"
"$ROOT/bin/amux" event --agent opencode < "$ROOT/tests/fixtures/opencode-idle.json"
"$ROOT/bin/amux" event --agent pi < "$ROOT/tests/fixtures/pi-tool-call.json"
"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-subagent.json"
"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-subagent-empty-cwd.json"

state="$("$ROOT/bin/amux" list --json)"

printf '%s\n' "$state" | jq -e '.records | length == 6' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].attention] | any' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].agent] | sort == ["claude","codex","codex","codex","opencode","pi"]' >/dev/null
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
        case "${3:-}" in
            '#{session_name}') printf 'codex-tmux\n' ;;
            '#{window_id}') printf '@1\n' ;;
            '#{pane_id}') printf '%%1\n' ;;
        esac
        ;;
    list-sessions)
        printf '100|codex-tmux|0\n'
        ;;
    list-panes)
        if [ "${AMUX_FAKE_AGENT_LIVE:-0}" = "1" ]; then
            printf 'codex-tmux|%%1|codex|200|codex\n'
        else
            printf 'codex-tmux|%%2|zsh|200|shell\n'
        fi
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

printf 'ok\n'
