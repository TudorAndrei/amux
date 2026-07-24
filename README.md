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
State writes are serialized so hooks arriving at the same time cannot overwrite
one another.

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

Inside tmux, the current pane is the agent identity. A restarted agent replaces
the previous record for that pane, while agents in separate panes remain
independent even when they share a tmux session. `amux sessions --json` returns
an `agents` array for each session and a session-level status rolled up as:
`attention`, `running`, `done`, then `offline`.

Initial normalization is conservative:

- permission, approval, notification, idle, ask, and waiting events set
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
subagent metadata such as `agent_id` or `agent_type`, UUID-only records without
a cwd or tmux target, and UUID-named tmux sessions commonly used for spawned
workers. Set `AMUX_HIDE_SUBAGENTS=0` to show tmux-backed subagents again.

## Installation and Requirements

Runtime requirements are tmux 3.2 or newer and GitHub CLI (`gh`). TPM is the
supported installation and update path: it downloads the matching native
release binary automatically. Rust, `jq`, and `fzf` are never required at
runtime. The picker is always the integrated native interface.

Contributors can build from source with Rust 1.96 or newer:

```bash
mise run package
mise run check
```

The tmux plugin deliberately never compiles Rust on demand. TPM downloads a
prebuilt release asset on installation and whenever its checkout version needs
a matching binary.

Releases use [Cocogitto](https://github.com/cocogitto/cocogitto): commits after
the `v0.0.0` compatibility baseline must follow Conventional Commits, and the
release workflow derives the next version, changelog, tag, and archives from
that history. Run `mise run cog-check` to verify the local range.

`mise run package` writes a deployable archive under `dist/` for the current host
target. GitHub Actions also builds archives for macOS arm64 and Linux
x86_64/arm64.

## Daemon Lifecycle and Recovery

The Rust daemon starts lazily after the first hook event or picker launch. It
owns a private `${XDG_STATE_HOME:-$HOME/.local/state}/amux/amux.sock`, writes
the version-one state and event log atomically, and caches session rows for the
tmux status segment. If it is unavailable, hook events use a locked one-shot
write and the next event starts a fresh daemon.

Use `bin/amux doctor` to inspect the selected binary, tmux version, state-file
compatibility, socket permissions, and monitor connection. A stale socket is
recovered automatically when a daemon starts; an invalid state file is reported
by `doctor` without being overwritten.

## Development

```bash
tests/smoke.sh
mise run check
```

Measured native-runtime latency, tmux reconciliation behavior, and the
historical shell comparison are recorded in
[docs/performance.md](docs/performance.md).

## Global Hooks

Preview hook installation:

```bash
bin/amux install-hooks --dry-run
```

Install global hooks:

```bash
bin/amux install-hooks --write
```

amux only installs lifecycle hooks: session start, prompt submission,
permission/notification, and stop. It deliberately does not install
`PreToolUse` or `PostToolUse` hooks, so individual tool calls never add amux
tracking output to an agent conversation.

The Rust installer writes timestamped backups before replacing existing global
config files. Hook assets live under `hooks/` and are rendered with the absolute
path to `bin/amux`; neither installation nor normal amux use requires `jq`.

## tmux

Install with TPM (the normal tmux plugin workflow):

```tmux
set -g @plugin 'TudorAndrei/amux'
```

Press `prefix + I` to install. Press `prefix + U` to update the plugin; the
next TPM reload or amux command fetches the matching release binary
automatically. On Apple Silicon macOS, Linux x86_64, and Linux arm64, there is
no Cargo build and no manual archive step.

After TPM installs amux, write global hooks from its checkout:

```bash
~/.tmux/plugins/amux/bin/amux install-hooks --write
```

Reloading replaces the previously registered amux status command and picker
binding. The native runtime reuses the version-one state file, and starts its
daemon lazily on the next hook event or picker launch.

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

`@amux-status on` prepends the compact `bin/amux status` segment to
`status-right`. If you prefer manual control, leave it unset and add:

```tmux
#(/path/to/amux/bin/amux status)
```

Hook events request an immediate tmux status redraw. The compact indicator
counts agents rather than tmux sessions.

Status indicators are colored by default in tmux and in the picker:

| Status | Indicator |
| --- | --- |
| `attention` | red `▲` |
| `running` | yellow `◐` |
| `done` | green `●` |
| `offline` | gray `○` |

Set `AMUX_COLOR=0`, `AMUX_PLAIN=1`, or `NO_COLOR=1` to use monochrome output.

The native Rust picker keeps one row per tmux session. For sessions with
multiple agents, the row targets the highest-priority agent pane. Search ranks
and filters only the tmux session name; visible status, reason, cwd, pane, age,
and agent details never contribute matches. It receives daemon updates while
keyboard input remains responsive; arrow keys, `j`/`k`, and `Ctrl-N`/`Ctrl-P`
navigate results, while `Ctrl-R` requests an immediate tmux redraw.
