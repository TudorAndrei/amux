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

Runtime requirements are tmux 3.2 or newer and a supported macOS or Linux
release archive. Archives contain the native `amux-rs` binary, the `bin/amux`
launcher, tmux plugin entry point, and hook templates; they do not require
Rust or `jq` after installation. When `fzf` is installed, amux uses the
classic reverse-list picker; otherwise it uses the built-in native picker.

For a release archive, unpack it and reference the extracted directory from
TPM or with `run-shell`:

```bash
tar -xzf amux-<version>-<target>.tar.gz
tmux run-shell /path/to/amux-<version>-<target>/amux.tmux
```

Contributors can build from source with Rust 1.96 or newer:

```bash
cargo build --release --bin amux-rs
make check
make package
```

The tmux plugin deliberately does not compile Rust on demand. Source checkouts
must be built before loading the plugin; end users should install a release
archive.

Releases use [Cocogitto](https://github.com/cocogitto/cocogitto): commits after
the `v0.0.0` compatibility baseline must follow Conventional Commits, and the
release workflow derives the next version, changelog, tag, and archives from
that history. Run `make cog-check` to verify the local range.

`make package` writes a deployable archive under `dist/` for the current host
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
make check
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

The Rust installer writes timestamped backups before replacing existing global
config files. Hook assets live under `hooks/` and are rendered with the absolute
path to `bin/amux`; neither installation nor normal amux use requires `jq`.

## tmux

Install with TPM:

```tmux
set -g @plugin 'TudorAndrei/amux'
set -g @amux-status on
```

Then reload tmux and run TPM install (`prefix + I`).

To migrate an existing shell-based installation to a release archive, unpack
the archive to its final path, then reload the plugin and rewrite hooks from
that path:

```bash
tmux run-shell /path/to/amux-<version>-<target>/amux.tmux
/path/to/amux-<version>-<target>/bin/amux install-hooks --write
tmux refresh-client -S
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
multiple agents, the row targets the highest-priority agent pane while the
session name remains the searchable field. It receives daemon updates while
keyboard input remains responsive; `Ctrl-R` requests an immediate tmux redraw.
