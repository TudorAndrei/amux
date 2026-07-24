#!/usr/bin/env bash
set -e

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

tmux set-environment -g AMUX_ROOT "$CURRENT_DIR"
tmux set-option -gq @amux-root "$CURRENT_DIR"
tmux set-option -gq @amux-status-command "$CURRENT_DIR/bin/amux status"

amux_key="$(tmux show-option -gqv @amux-picker-key)"
amux_key="${amux_key:-A}"
popup_width="$(tmux show-option -gqv @amux-popup-width)"
popup_width="${popup_width:-90%}"
popup_height="$(tmux show-option -gqv @amux-popup-height)"
popup_height="${popup_height:-80%}"
if [ "$amux_key" != "A" ]; then
	existing_a_binding="$(tmux list-keys -T prefix A 2>/dev/null || true)"
	case "$existing_a_binding" in
	*"$CURRENT_DIR/bin/amux picker"*) tmux unbind-key A ;;
	esac
fi
tmux bind-key "$amux_key" display-popup -w "$popup_width" -h "$popup_height" -E "$CURRENT_DIR/bin/amux picker"

next_key="$(tmux show-option -gqv @amux-next-attention-key)"
if [ -n "$next_key" ]; then
	tmux bind-key "$next_key" run-shell "$CURRENT_DIR/bin/amux next-attention"
fi

if [ "$(tmux show-option -gqv @amux-status)" = "on" ]; then
	status_right="$(tmux show-option -gqv status-right)"
	case "$status_right" in
	*"bin/amux status"*) ;;
	*) tmux set-option -g status-right "#($CURRENT_DIR/bin/amux status) $status_right" ;;
	esac
fi
