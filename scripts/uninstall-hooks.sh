#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "Manual uninstall for now:"
printf '%s\n' "- remove amux hook entries from ~/.codex/hooks.json"
printf '%s\n' "- remove amux hook entries from ~/.claude/settings.json"
printf '%s\n' "- remove ~/.config/opencode/plugins/amux.js"
printf '%s\n' "- remove ~/.pi/agent/extensions/amux.ts and its path from ~/.pi/agent/settings.json"
printf '%s\n' "Backups from install use *.amux.bak.<timestamp>"

