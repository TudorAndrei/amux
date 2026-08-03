#!/usr/bin/env bash
set -e

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! "$CURRENT_DIR/scripts/ensure-runtime.sh"; then
	tmux display-message 'amux: unable to install the native release binary; run prefix + I after installing gh'
	exit 0
fi

tmux set-environment -g AMUX_ROOT "$CURRENT_DIR"
tmux set-option -gq @amux-root "$CURRENT_DIR"

amux_key="$(tmux show-option -gqv @amux-picker-key)"
amux_key="${amux_key:-A}"
popup_width="$(tmux show-option -gqv @amux-popup-width)"
popup_width="${popup_width:-90%}"
popup_height="$(tmux show-option -gqv @amux-popup-height)"
popup_height="${popup_height:-80%}"
if [ "$amux_key" != "A" ]; then
	existing_a_binding="$(tmux list-keys -T prefix A 2>/dev/null || true)"
	case "$existing_a_binding" in
	*"$CURRENT_DIR/bin/amux picker"* | *"$CURRENT_DIR/scripts/picker.sh"*) tmux unbind-key A ;;
	esac
fi
# `display-popup -e` passes formats through literally. `run-shell` expands
# formats in the keypress client's context, so capture its TTY before opening
# the popup and give the picker an unambiguous `switch-client -c` target.
tmux bind-key "$amux_key" run-shell \
	"tmux display-popup -w '$popup_width' -h '$popup_height' -E 'AMUX_TMUX_CLIENT=#{client_tty} exec \"$CURRENT_DIR/bin/amux\" picker'"

next_key="$(tmux show-option -gqv @amux-next-attention-key)"
if [ -n "$next_key" ]; then
	tmux bind-key "$next_key" run-shell "$CURRENT_DIR/bin/amux next-attention"
fi
