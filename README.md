# amux

Agent multiplexer for tmux.

Initial scope:

- Collect agent lifecycle events from global hooks.
- Track Codex, Claude, Pi, and opencode sessions.
- Surface agents that need user attention.
- Provide a live tmux picker integration.

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

The cached `state.json` is optimized for picker rendering.
`events.jsonl` is a bounded transition log for debugging hook normalization:
unchanged repeated events are not appended. The daemon retains at most
`${AMUX_EVENTS_PER_SESSION:-200}` events for each
`agent:tmux_session:pane` key, drops keys that have expired from state, and
compacts once it reaches `${AMUX_EVENTS_COMPACT_BYTES:-8388608}` bytes. Set
`AMUX_EVENTS_PER_SESSION=0` to disable daemon compaction. Hook input is capped
at 256 KiB before full allocation or parsing. Persisted raw metadata is a
smaller explicit allowlist for lifecycle classification, session identity,
cwd, reason, and subagent detection; tool input, messages, command text, and
unknown fields are never written.
The state directory is owner-only (`0700`), and both files are owner-readable
and writable (`0600`), including daemon-less fallback writes. State writes use
an advisory `flock` on `state.lock` so hooks arriving at the same time cannot
overwrite one another. On NFS, `flock` may block forever even for a non-blocking
request; if `AMUX_STATE_DIR` is on a network mount, set `AMUX_LOCK=0` to disable
this protection explicitly.

Each state record contains:

| Field | Meaning |
| --- | --- |
| `agent` | `codex`, `claude`, `pi`, or `opencode` |
| `agent_session_id` | Agent session id when the hook exposes one |
| `tmux_session` | tmux session name when the hook runs inside tmux |
| `tmux_pane` | tmux pane id when available |
| `cwd` | Working directory reported by the hook or current process |
| `status` | `running`, `attention`, `done`, `offline`, or `unknown` |
| `attention` | Boolean flag for records that should be surfaced first |
| `reason` | Short reason shown in tmux UI |
| `last_event` | Last normalized hook event name |
| `updated_at` | Unix timestamp for stale-record handling |

Inside tmux, the current pane is the agent identity. A restarted agent replaces
the previous record for that pane, while agents in separate panes remain
independent even when they share a tmux session. `amux sessions --json` returns
an `agents` array for each session and a session-level status rolled up as:
`attention`, `running`, `done`, then `offline`.

Lifecycle classification is centralized in the Rust process rather than
duplicated in installed hook command arguments:

- permission, approval, notification, idle, ask, and waiting events set
  `attention=true`
- Claude `Notification` payloads are more specific: `idle_prompt` and
  `agent_completed` map to `done`, while `permission_prompt`,
  `agent_needs_input`, and unknown types map to `attention`
- session-end events map to `offline`; stop, end, idle, done, and complete
  events map to `done` unless they are also
  attention events
- all other hook activity maps to `running`

Codex's nine installed events use an explicit in-process mapping. Unknown
future agent events remain conservative (`running`, without attention), and
manual `--status`/`--attention` overrides retain precedence.

Status and list commands ignore records older than
`${AMUX_STALE_SECONDS:-86400}` seconds.

### Event history

`amux events` reads the retained transition log without changing it. It returns
the newest 100 matching events by default, capped at 1,000 with `--limit`, and
prints those matches in chronological order. Filters may be combined:

```bash
amux events --agent codex --session work --pane %3 --limit 25
amux events --json --session 0192f8a1-session-id
```

`--session` matches either the tmux session or the agent session id. Plain text
uses stable tab-separated columns: ISO timestamp, agent, status, tmux session,
pane, event, reason, and cwd. Every text field is terminal-sanitized. JSON is a
single bounded object shaped as `{"version":1,"events":[...]}` and contains
only the current minimized record fields and allowlisted raw metadata. Unknown
legacy fields are discarded again while reading. Malformed log lines are
skipped, and a missing or empty log returns no text or an empty JSON array.

History has the same privacy and retention limits as `events.jsonl`; it is not
an audit log. `AMUX_EVENTS_PER_SESSION=0` disables automatic compaction rather
than deleting history, but `amux events` remains capped at 1,000 records per
invocation.

Session views only surface agents associated with tmux sessions. `amux sessions`,
the picker ignores hook records captured outside
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
hk check
```

The tmux plugin deliberately never compiles Rust on demand. TPM downloads a
prebuilt release asset on installation and whenever its checkout version needs
a matching binary.

Releases use [Cocogitto](https://github.com/cocogitto/cocogitto): commits after
the `v0.0.0` compatibility baseline must follow Conventional Commits, and the
release workflow derives the next version, changelog, tag, and archives from
that history. Run `hk check` to verify the local range.

`mise run package` writes a deployable archive under `dist/` for the current host
target. GitHub Actions also builds archives for macOS arm64 and Linux
x86_64/arm64.

## Daemon Lifecycle and Recovery

The Rust daemon starts lazily after the first hook event or picker launch. It
owns a private `${XDG_STATE_HOME:-$HOME/.local/state}/amux/amux.sock`, writes
the version-one state and event log atomically, and caches session rows for the
picker. If it is unavailable, hook events use a locked one-shot
write and the next event starts a fresh daemon.

Use `bin/amux doctor` to inspect the selected binary, tmux version, state-file
compatibility, socket permissions, monitor connection, and integration drift
for Codex, Claude, Pi, and opencode. A stale socket is
recovered automatically when a daemon starts; an invalid state file is reported
by `doctor` without being overwritten. If doctor reports hook drift after an
upgrade, run `bin/amux install-hooks --write` to install the current mappings.

## Development

```bash
tests/smoke.sh
hk check
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

Codex installs explicit activity hooks for session start, prompt submission,
pre/post tool use, pre/post compaction, permission requests, turn stop, and
session end. Its `SessionStart` matcher is `startup|resume|clear`; compaction is
tracked by dedicated hooks instead. `Stop` is turn-scoped: a tool-less next
turn can briefly remain `done` because Codex provides no turn-start hook. Re-run
`bin/amux install-hooks --write` after upgrading; `bin/amux doctor` reports
separate Codex, Claude, Pi, and opencode integration results.

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

Reloading installs the picker binding without modifying `status-right`. The
native runtime reuses the version-one state file and starts its daemon lazily
on the next hook event or picker launch.

Updating the plugin fetches the matching binary and retires the running daemon
automatically, but hook files are never rewritten unasked: run `amux doctor`
after an update and re-run `install-hooks --write` when it reports drift. See
[docs/updating.md](docs/updating.md).

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
```

## Upgrade note

The `amux status` command and `@amux-status`/`status-right` integration were
removed. Remove any `#(.../amux status)` segment from existing tmux configs;
amux no longer changes `status-right`. The picker continues to receive live
daemon updates.

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
