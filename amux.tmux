#!/usr/bin/env bash
set -e

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! "$CURRENT_DIR/scripts/ensure-runtime.sh"; then
	tmux display-message 'amux: unable to install the native release binary; run prefix + I after installing gh'
	exit 0
fi

tmux set-environment -g AMUX_ROOT "$CURRENT_DIR"
tmux set-option -gq @amux-root "$CURRENT_DIR"
