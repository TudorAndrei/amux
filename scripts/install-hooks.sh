#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AMUX_BIN="$ROOT/bin/amux"
MODE="dry-run"

usage() {
    cat <<'USAGE'
Usage: scripts/install-hooks.sh [--dry-run|--write]

Installs global amux hooks for Codex, Claude, opencode, and Pi.
Default mode is --dry-run.
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) MODE="dry-run"; shift ;;
        --write) MODE="write"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 1 ;;
    esac
done

need() {
    command -v "$1" >/dev/null 2>&1 || {
        printf 'missing required command: %s\n' "$1" >&2
        exit 1
    }
}

json_with_amux() {
    sed "s|__AMUX_BIN__|$AMUX_BIN|g" "$1"
}

js_with_amux() {
    sed "s|__AMUX_BIN__|$AMUX_BIN|g" "$1"
}

backup() {
    local path="$1"
    if [ -e "$path" ]; then
        cp "$path" "$path.amux.bak.$(date +%Y%m%d%H%M%S)"
    fi
}

write_json_merge_hooks() {
    local dest="$1" fragment="$2" name="$3"
    local dir tmp
    dir="$(dirname "$dest")"
    tmp="$(mktemp)"

    if [ "$MODE" = "write" ]; then
        mkdir -p "$dir"
    fi
    if [ -f "$dest" ]; then
        jq --argjson fragment "$fragment" '
          .hooks = (.hooks // {})
          | reduce (($fragment.hooks // {}) | keys[]) as $event (.;
              .hooks[$event] = (((.hooks[$event] // []) + $fragment.hooks[$event]) | unique)
            )
        ' "$dest" > "$tmp"
    else
        printf '%s\n' "$fragment" | jq . > "$tmp"
    fi

    if [ "$MODE" = "dry-run" ]; then
        printf 'would update %s: %s\n' "$name" "$dest"
        cat "$tmp"
        rm -f "$tmp"
    else
        backup "$dest"
        mv "$tmp" "$dest"
        printf 'updated %s: %s\n' "$name" "$dest"
    fi
}

write_file() {
    local dest="$1" content="$2" name="$3"
    if [ "$MODE" = "dry-run" ]; then
        printf 'would write %s: %s\n' "$name" "$dest"
        printf '%s\n' "$content"
    else
        mkdir -p "$(dirname "$dest")"
        backup "$dest"
        printf '%s\n' "$content" > "$dest"
        printf 'wrote %s: %s\n' "$name" "$dest"
    fi
}

install_codex() {
    local fragment
    fragment="$(json_with_amux "$ROOT/hooks/codex/hooks.json")"
    write_json_merge_hooks "$HOME/.codex/hooks.json" "$fragment" "Codex hooks"
}

install_claude() {
    local fragment
    fragment="$(json_with_amux "$ROOT/hooks/claude/settings.fragment.json")"
    write_json_merge_hooks "$HOME/.claude/settings.json" "$fragment" "Claude settings hooks"
}

install_opencode() {
    local content
    content="$(js_with_amux "$ROOT/hooks/opencode/amux.js")"
    write_file "$HOME/.config/opencode/plugins/amux.js" "$content" "opencode plugin"
}

install_pi() {
    local extension_dest settings_dest tmp
    extension_dest="$HOME/.pi/agent/extensions/amux.ts"
    settings_dest="$HOME/.pi/agent/settings.json"
    tmp="$(mktemp)"

    write_file "$extension_dest" "$(js_with_amux "$ROOT/hooks/pi/amux.ts")" "Pi extension"

    if [ "$MODE" = "write" ]; then
        mkdir -p "$(dirname "$settings_dest")"
    fi
    if [ -f "$settings_dest" ]; then
        jq --arg extension "$extension_dest" '
          .extensions = (((.extensions // []) + [$extension]) | unique)
        ' "$settings_dest" > "$tmp"
    else
        jq -n --arg extension "$extension_dest" '{extensions: [$extension]}' > "$tmp"
    fi

    if [ "$MODE" = "dry-run" ]; then
        printf 'would update Pi settings: %s\n' "$settings_dest"
        cat "$tmp"
        rm -f "$tmp"
    else
        backup "$settings_dest"
        mv "$tmp" "$settings_dest"
        printf 'updated Pi settings: %s\n' "$settings_dest"
    fi
}

need jq
[ -x "$AMUX_BIN" ] || {
    printf 'amux binary is not executable: %s\n' "$AMUX_BIN" >&2
    exit 1
}

install_codex
install_claude
install_opencode
install_pi
