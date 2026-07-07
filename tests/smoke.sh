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

printf 'ok\n'
