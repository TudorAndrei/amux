#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
export AMUX_STATE_DIR
AMUX_STATE_DIR="$(mktemp -d "$TMPDIR/amux-test.XXXXXX")"
trap 'rm -rf "$AMUX_STATE_DIR"' EXIT

"$ROOT/bin/amux" event --agent codex < "$ROOT/tests/fixtures/codex-permission.json"
"$ROOT/bin/amux" event --agent claude < "$ROOT/tests/fixtures/claude-stop.json"
"$ROOT/bin/amux" event --agent opencode < "$ROOT/tests/fixtures/opencode-idle.json"
"$ROOT/bin/amux" event --agent pi < "$ROOT/tests/fixtures/pi-tool-call.json"

state="$("$ROOT/bin/amux" list --json)"

printf '%s\n' "$state" | jq -e '.records | length == 4' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].attention] | any' >/dev/null
printf '%s\n' "$state" | jq -e '[.records[].agent] | sort == ["claude","codex","opencode","pi"]' >/dev/null
test "$("$ROOT/bin/amux" status)" = "▲ 2"

printf 'ok\n'

