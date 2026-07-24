#!/usr/bin/env bash
set -e

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! "$CURRENT_DIR/scripts/ensure-runtime.sh"; then
	tmux display-message 'amux: unable to install the native release binary; run prefix + I after installing gh'
	exit 0
fi

tmux set-environment -g AMUX_ROOT "$CURRENT_DIR"
tmux set-option -gq @amux-root "$CURRENT_DIR"
previous_status_command="$(tmux show-option -gqv @amux-status-command)"
status_command="$CURRENT_DIR/bin/amux status"
tmux set-option -gq @amux-status-command "$status_command"

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
tmux bind-key "$amux_key" display-popup -w "$popup_width" -h "$popup_height" -E "$CURRENT_DIR/bin/amux picker"

next_key="$(tmux show-option -gqv @amux-next-attention-key)"
if [ -n "$next_key" ]; then
	tmux bind-key "$next_key" run-shell "$CURRENT_DIR/bin/amux next-attention"
fi

status_right="$(tmux show-option -gqv status-right)"
if [ -n "$previous_status_command" ]; then
	previous_status_segment="#($previous_status_command)"
	status_right="${status_right/"$previous_status_segment" /}"
	status_right="${status_right/"$previous_status_segment"/}"
fi
if [ "$(tmux show-option -gqv @amux-status)" = "on" ]; then
	case "$status_right" in
	*"bin/amux status"*) ;;
	*) status_right="#($status_command) $status_right" ;;
	esac
fi
tmux set-option -g status-right "$status_right"
