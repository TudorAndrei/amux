# amux

Agent multiplexer for tmux.

Initial scope:

- Collect agent lifecycle events from global hooks.
- Track Codex, Claude, Pi, and opencode sessions.
- Surface agents that need user attention.
- Provide tmux status and picker integrations.

## State Model

Agent hooks call:

```bash
amux event --agent codex --event PermissionRequest < hook-input.json
```

`amux` stores derived state in:

```text
${XDG_STATE_HOME:-$HOME/.local/state}/amux/state.json
${XDG_STATE_HOME:-$HOME/.local/state}/amux/events.jsonl
```

The cached `state.json` is optimized for tmux status and picker rendering. The
append-only `events.jsonl` file is for debugging hook input and normalization.

Each state record contains:

| Field | Meaning |
| --- | --- |
| `agent` | `codex`, `claude`, `pi`, or `opencode` |
| `agent_session_id` | Agent session id when the hook exposes one |
| `tmux_session` | tmux session name when the hook runs inside tmux |
| `tmux_pane` | tmux pane id when available |
| `cwd` | Working directory reported by the hook or current process |
| `status` | `running`, `attention`, `done`, or `unknown` |
| `attention` | Boolean flag for records that should be surfaced first |
| `reason` | Short reason shown in tmux UI |
| `last_event` | Last normalized hook event name |
| `updated_at` | Unix timestamp for stale-record handling |

Initial normalization is conservative:

- permission, approval, notification, idle, ask, prompt, and waiting events set
  `attention=true`
- stop, end, idle, done, and complete events map to `done` unless they are also
  attention events
- all other hook activity maps to `running`

## Development

```bash
tests/smoke.sh
```

## Global Hooks

Preview hook installation:

```bash
scripts/install-hooks.sh --dry-run
```

Install global hooks:

```bash
scripts/install-hooks.sh --write
```

The installer writes backups before replacing existing global config files. Hook
assets live under `hooks/` and are rendered with the absolute path to
`bin/amux`.

## tmux

Load the plugin directly:

```tmux
run-shell /path/to/amux/amux.tmux
```

Default bindings:

| Binding | Action |
| --- | --- |
| `prefix + A` | Open the `amux` picker |

Optional settings:

```tmux
set -g @amux-picker-key A
set -g @amux-next-attention-key C-a
set -g @amux-status on
run-shell /path/to/amux/amux.tmux
```

`@amux-status on` prepends the compact `scripts/status.sh` segment to
`status-right`. If you prefer manual control, leave it unset and add:

```tmux
#(/path/to/amux/scripts/status.sh)
```

### Dotfiles-local install

This checkout is intended to be loaded from:

```text
/Users/tudor/dotfiles/configs/tmux/amux
```

The current dotfiles tmux config at `/Users/tudor/dotfiles/configs/tmux/tmux.conf`
loads the plugin with:

```tmux
run-shell /Users/tudor/dotfiles/configs/tmux/amux/amux.tmux
```

It keeps the existing `prefix + l` session picker on
`/Users/tudor/dotfiles/configs/tmux/session-picker.sh` and uses `prefix + A`
for the `amux` picker until the new picker has full session-picker parity.
