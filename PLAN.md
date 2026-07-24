# Plan: Rust Event-Driven amux

## Goal

Replace amux's shell and `jq` state-processing hot path with a Rust runtime that
preserves the existing CLI, hook payloads, `state.json` schema, session-only
picker rows, multi-agent session aggregation, and tmux plugin behavior. The new
runtime will use one persistent tmux control-mode connection and push state
changes to a native TUI so topology refreshes and hook updates never block
keyboard input.

## Approach

The migration will be incremental so every phase can be committed with a
working repository. The current entry points in `bin/amux` and `amux.tmux` will
remain active until their Rust replacements have behavior-parity tests.

A new Cargo binary will separate the existing responsibilities currently
combined in `lib/state.sh`:

- `src/model.rs` will define version-one persisted records and derived
  `AgentView` and `SessionView` values.
- `src/event.rs` will port `amux_normalize_event`, including explicit overrides,
  conservative attention inference, pane-based identity, stale-record pruning,
  and subagent metadata handling.
- `src/state.rs` will load the existing `state.json`, append compatible
  `events.jsonl` entries, and atomically persist updates.
- `src/sessions.rs` will port `amux_sessions` and `amux_status`, preserving the
  distinct session and target-agent priority orders, one output row per tmux
  session, agent counts, and the `agents` array.
- `src/tmux/` will own tmux identifiers, snapshots, commands, and monitoring.
  It will pin `tmuxctl` and use its Tokio protocol support rather than importing
  `tmuxpulse`'s TUI and nested snapshot implementation.
- `src/daemon.rs` and `src/ipc.rs` will provide a single-writer service over a
  user-only Unix socket. Hook invocations will send normalized event requests,
  while picker clients subscribe to state revisions.
- `src/ui.rs` will implement the picker with Ratatui and Crossterm. Keyboard
  input, state subscriptions, and tmux reconciliation will run independently.

The tmux monitor will open one `tmux -C` connection for the active tmux server,
immediately issue `refresh-client -f no-output`, and use asynchronous control
notifications only as topology-invalidated signals. After a 10–25 ms debounce,
it will issue flat `list-sessions -F ...` and `list-panes -a -F ...` commands
through the same connection. It will also reconcile periodically and after a
reconnect because control notifications do not describe every topology change
completely. No nested session/window/pane subprocess loop will be introduced.

Internally, topology and agents will be keyed by stable tmux IDs and pane IDs.
The public JSON will retain the existing field names and session names. Multiple
agent panes will therefore remain independent, while `sessions --json`, the
status segment, and the picker continue to emit exactly one aggregate per
session. A changed search query will select the first matching session; passive
state refreshes will preserve the selected session when it still exists.

The daemon will be started lazily by the Rust CLI. `event` will retry a daemon
connection briefly and then use a locked, atomic one-shot write if the service
cannot start, so agent hooks never lose an event merely because the daemon is
down. The socket directory will be private to the current user. The existing
version-one files remain the durable recovery format and debugging log.

After parity is established, `bin/amux` will become a thin launcher for the Rust
binary and `amux.tmux` will invoke Rust subcommands directly. `amux.tmux` itself
and the JavaScript/TypeScript hook adapters must remain in their host languages.
The Rust hook installer replaces the former shell JSON merge work, allowing
`jq` and `fzf` to stop being runtime dependencies.

Out of scope for this migration are pane-output capture, terminal emulation,
remote hosts, replacing tmux itself, changing the hook semantics documented in
`docs/events.md`, and supporting more agent products.

## Implementation Phases

### Phase 1: Rust core with behavior parity

- Add `Cargo.toml`, `Cargo.lock`, `src/main.rs`, and the model, configuration,
  event, state, session aggregation, and rendering modules described above.
- Keep `bin/amux` as the active user-facing command while exposing the Rust
  implementation separately for compatibility testing.
- Read and write the existing version-one `state.json` and `events.jsonl`
  formats, honoring `AMUX_STATE_DIR`, `XDG_STATE_HOME`, `AMUX_STALE_SECONDS`,
  `AMUX_HIDE_SUBAGENTS`, `AMUX_COLOR`, `AMUX_PLAIN`, and `NO_COLOR`.
- Port all normalization and aggregation cases covered by
  `tests/fixtures/*.json` and `tests/smoke.sh`, including concurrent event
  writes and the special `UserPromptSubmit` behavior.
- Add Rust unit and integration fixtures that assert JSON and text output
  compatibility with the current `event`, `list`, `sessions`, `status`, `clear`,
  and `doctor` commands.

  **Commit:** `feat(core): add the Rust state and session engine`

### Phase 2: Persistent daemon and tmux monitor

- Add the Unix-socket request, response, and subscription protocol in
  `src/ipc.rs`, including socket ownership checks, lazy startup, clean shutdown,
  and stale-socket recovery.
- Add `src/daemon.rs` as the only normal-path state writer and publish monotonic
  state revisions to connected clients.
- Add a tmux provider abstraction and test double, then implement
  `src/tmux/control.rs` with pinned `tmuxctl` Tokio support.
- Disable pane output, build initial topology with one flat sessions query and
  one flat all-panes query, debounce notification bursts, periodically
  reconcile, and reconnect with bounded backoff.
- Associate incoming hook requests with their `TMUX` server and `TMUX_PANE`
  context without running `display-message` for each event.
- Preserve a safe one-shot file-write and one-shot tmux-query fallback for
  daemon startup or recovery failures.
- Add isolated tmux-server integration tests for create, rename, close,
  reconnect, linked topology, and multiple agent panes in one session.

  **Commit:** `feat(daemon): add event-driven tmux and agent state monitoring`

### Phase 3: Non-blocking native picker

- Add `src/ui.rs` with a Ratatui/Crossterm event loop that selects concurrently
  over keyboard input and daemon revisions; no tmux query or disk operation may
  run on the input/render task.
- Preserve one row per session, status ordering, colors, age display, reason,
  target pane selection, and an agent detail view for multi-agent sessions.
- Preserve session-name-only fuzzy search, reset selection to the first match
  when the query changes, and retain selection by tmux session ID during passive
  refreshes.
- Implement Enter, Escape, Ctrl-C, navigation, and Ctrl-R reconciliation, then
  perform `select-pane` and `switch-client` through the tmux command channel.
- Retain a deterministic `picker --rows`/plain output for scripts and tests.
- Add reducer-level UI tests proving delayed topology refreshes do not delay
  key handling and regression tests for filtering, selection, deduplication,
  and live multi-agent updates.

  **Commit:** `feat(picker): replace polling reloads with a responsive Rust TUI`

### Phase 4: CLI, hooks, and tmux integration cutover

- Switch `bin/amux` to the Rust binary while preserving the current CLI syntax,
  standard output, JSON schemas, exit behavior, and environment overrides.
- Replace shell picker, status, and next-attention adapters with direct Rust
  subcommand invocations from `amux.tmux`.
- Update `amux.tmux` to launch the Rust picker and consume a cached status value
  without polling the complete session model; retain immediate tmux redraws
  after hook events.
- Move hook installation and removal into Rust commands while preserving the
  templates under `hooks/`, dry-run behavior, unique JSON merges, absolute
  command paths, and timestamped backups.
- Extend `doctor` with binary, tmux version, daemon socket, state compatibility,
  and monitor health checks.
- Run the legacy `tests/smoke.sh` contract against the Rust entry point before
  removing `lib/core.sh` and `lib/state.sh`.

  **Commit:** `refactor(cli): switch amux integrations to the Rust runtime`

### Phase 5: Packaging, verification, and documentation

- Update `Makefile` so `make check` runs formatting, Clippy, Rust tests, shell
  syntax for retained adapters, and isolated tmux integration tests where tmux
  is available.
- Add CI for supported macOS and Linux targets and produce release binaries so
  TPM users do not need a Rust toolchain at runtime; keep a documented
  source-build path for contributors.
- Update `README.md`, `docs/events.md`, and `RELEASE.md` with installation,
  daemon lifecycle, recovery, state compatibility, minimum versions, and
  troubleshooting instructions.
- Remove `jq` and `fzf` from runtime requirements and delete obsolete shell
  implementation files only after compatibility and migration tests pass.
- Measure event-to-render latency, idle wakeups, topology reconciliation time,
  and keyboard responsiveness against the shell baseline.

  **Commit:** `chore(release): package and document the Rust implementation`

## Risks & Tradeoffs

- `tmuxctl` is currently a young `0.1.0` crate with a high Rust version
  requirement. Pin the exact release, isolate it behind `src/tmux/`, and vendor
  its protocol core if its API or minimum compiler version becomes a problem.
- A control-mode client is attached to one session and its notifications are
  not a complete normalized server model. Treat every relevant event as an
  invalidation and reconcile the server-wide flat snapshot instead of applying
  incomplete event payloads literally.
- The daemon adds lifecycle and IPC failure modes. Lazy startup, a single-writer
  lock, stale-socket recovery, atomic disk persistence, and a one-shot fallback
  keep hooks functional during failures.
- Tmux windows may be linked into multiple sessions. Snapshot parsing and
  deduplication must use session, window, and pane IDs rather than display names
  and must be exercised against an isolated real tmux server.
- A native picker removes the familiar fzf implementation. Preserve the
  user-visible search and selection contract in tests, while accepting that
  exact fzf ranking and keybindings are not part of compatibility.
- Prebuilt release binaries increase CI and release maintenance, but compiling
  from a TPM plugin load would be slow and would require every user to install
  Rust.

## Open Questions

- Should the first Rust release ship prebuilt binaries for macOS arm64,
  macOS x86_64, Linux x86_64, and Linux arm64, or is a source-build-only first
  milestone acceptable?
- What minimum tmux and Rust versions should be supported? The implementation
  target is tmux 3.2 or newer and Rust 1.96 or newer, required by pinned
  `tmuxctl` 0.1.0.
- Should non-default tmux servers selected with `-L` or `-S` be supported in the
  first release, or should the first daemon monitor only the server identified
  by the invoking client's `TMUX` environment?
