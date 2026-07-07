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

Status and list commands ignore records older than
`${AMUX_STALE_SECONDS:-86400}` seconds.

Session views only surface agents associated with tmux sessions. `amux sessions`,
the picker, and the tmux status segment ignore hook records captured outside
tmux; those records remain available through `amux list` for debugging. Session
views also hide spawned subagents by default, including hook records with
subagent metadata such as `agent_id` or `agent_type`, UUID-only records without a
cwd or tmux target, and UUID-named tmux sessions commonly used for spawned
workers. Set `AMUX_HIDE_SUBAGENTS=0` to show tmux-backed subagents again.

## Development

```bash
tests/smoke.sh
make check
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

Install with TPM:

```tmux
set -g @plugin 'TudorAndrei/amux'
set -g @amux-status on
```

Then reload tmux and run TPM install (`prefix + I`).

Load the plugin directly for local development:

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
set -g @amux-popup-width 90%
set -g @amux-popup-height 80%
set -g @amux-next-attention-key C-a
set -g @amux-status on
```

`@amux-status on` prepends the compact `scripts/status.sh` segment to
`status-right`. If you prefer manual control, leave it unset and add:

```tmux
#(/path/to/amux/scripts/status.sh)
```

Status indicators are colored by default in tmux and in the picker:

| Status | Indicator |
| --- | --- |
| `attention` | red `▲` |
| `running` | yellow `◐` |
| `done` | green `●` |
| `offline` | gray `○` |

Set `AMUX_COLOR=0`, `AMUX_PLAIN=1`, or `NO_COLOR=1` to use monochrome output.
