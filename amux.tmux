#!/usr/bin/env bash
CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

tmux set-environment -g AMUX_ROOT "$CURRENT_DIR"
tmux bind-key A display-popup -E "$CURRENT_DIR/scripts/picker.sh"

