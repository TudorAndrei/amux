# TODO: Rust Event-Driven amux

## Phase 1: Rust core with behavior parity

- [x] Add the Cargo project and Rust CLI alongside the active `bin/amux`.
- [x] Implement version-one agent records, configuration, normalization,
      persistence, session aggregation, and text/JSON rendering.
- [x] Preserve all current `AMUX_*`, XDG, color, and stale-state environment
      behavior.
- [x] Port `tests/fixtures/*.json` cases and the `tests/smoke.sh` command
      contracts to Rust tests.
- [x] Test concurrent writes, `UserPromptSubmit`, subagent filtering, stale
      records, status counts, and multi-agent session aggregation.
- [ ] Commit: `feat(core): add the Rust state and session engine`

## Phase 2: Persistent daemon and tmux monitor

- [x] Implement the private Unix-socket IPC protocol, lazy daemon startup,
      subscriptions, shutdown, and stale-socket recovery.
- [x] Make the daemon the normal-path state writer while retaining atomic
      version-one state and event-log persistence.
- [x] Add the tmux provider abstraction and pinned `tmuxctl` Tokio integration.
- [x] Disable pane output and reconcile topology with flat `list-sessions` and
      `list-panes -a` commands after debounced control notifications.
- [x] Add periodic reconciliation, reconnect backoff, and one-shot fallback
      paths.
- [x] Resolve hook events by tmux server and pane without per-event
      `display-message` subprocesses.
- [x] Test isolated tmux session create, rename, close, reconnect, linked
      topology, and multiple agents in one session.
- [ ] Commit: `feat(daemon): add event-driven tmux and agent state monitoring`

## Phase 3: Non-blocking native picker

- [x] Implement the Ratatui/Crossterm picker with independent input, rendering,
      state-subscription, and topology tasks.
- [x] Render one session row with status, age, reason, target pane, and
      multi-agent detail information.
- [x] Implement session-name-only fuzzy search and select the first result after
      each query change.
- [x] Preserve the selected session across passive refreshes and avoid duplicate
      session rows.
- [x] Implement navigation, Enter, Escape, Ctrl-C, Ctrl-R, `picker --rows`, and
      plain output.
- [x] Test that delayed refreshes do not block keyboard events and that live
      agent updates do not reset a valid selection.
- [ ] Commit: `feat(picker): replace polling reloads with a responsive Rust TUI`

## Phase 4: CLI, hooks, and tmux integration cutover

- [x] Switch `bin/amux` to the Rust command implementation without changing the
      existing CLI and JSON contracts.
- [x] Replace the picker, status, and next-attention shell adapters with direct
      Rust command invocations from `amux.tmux`.
- [x] Update `amux.tmux` to use the native picker and cached status without
      full-model polling.
- [x] Implement hook install, uninstall, dry-run, JSON merge, and backup
      behavior in Rust using the existing `hooks/` templates.
- [x] Extend `doctor` with tmux, daemon, monitor, socket, and state-format
      diagnostics.
- [x] Run `tests/smoke.sh` against Rust, then remove `lib/core.sh` and
      `lib/state.sh`.
- [ ] Commit: `refactor(cli): switch amux integrations to the Rust runtime`

## Phase 5: Packaging, verification, and documentation

- [x] Update `Makefile` with Rust formatting, Clippy, unit, integration, and
      retained shell-adapter checks.
- [x] Add macOS and Linux CI plus release-binary packaging for the agreed target
      architectures.
- [x] Document Rust installation, daemon lifecycle, recovery, compatibility,
      minimum versions, and troubleshooting in `README.md`, `docs/events.md`,
      and `RELEASE.md`.
- [x] Remove `jq` and `fzf` runtime requirements and delete obsolete shell code
      only after parity tests pass.
- [x] Record event-to-render latency, topology reconciliation time, idle
      wakeups, and keyboard responsiveness compared with the shell version.
- [ ] Commit: `chore(release): package and document the Rust implementation`

## Verification

- [x] The original `make check` baseline remains green until the Rust cutover.
- [x] `cargo fmt --check`, `cargo clippy --all-targets --all-features`, and
      `cargo test --all-features` pass.
- [x] `event`, `list`, `sessions`, `status`, `clear`, and `doctor` retain their
      documented arguments, exit behavior, text output, and JSON fields.
- [x] Existing version-one `state.json` files load without migration or data
      loss, and new writes remain valid to older readers until cutover.
- [x] `events.jsonl` contains every accepted hook event during a 40-writer burst,
      with no corrupt state file or abandoned lock/socket.
- [x] A permission event, tool event, prompt submission, stop event, and stale
      record produce the same derived states documented in `docs/events.md`.
- [x] A tmux session with Codex and Claude in separate panes produces one
      session row, two agents, the correct rolled-up status, and the
      highest-priority target pane.
- [x] Creating, renaming, linking, and closing tmux sessions/windows/panes
      updates the model without duplicate sessions.
- [x] Killing and restarting the tmux server or amux daemon reconnects and
      rebuilds a correct snapshot.
- [x] Typing and navigation remain responsive while a mocked tmux snapshot is
      delayed, and a query change selects the first matching session.
- [x] Passive live updates preserve the selected session when it remains
      visible.
- [x] Manual smoke test: load `amux.tmux`, open the picker with `prefix + A`,
      search by session name, receive live multi-agent updates, and switch to
      the selected target pane.
- [x] Manual smoke test: with `@amux-status on`, hook events update the colored
      agent count without waiting for a polling interval.
- [x] Manual smoke test: stop the daemon, send a hook event, and verify fallback
      persistence followed by successful daemon recovery.
- [x] No behavior regression in subagent hiding, stale-record pruning, plain
      output, color controls, hook dry runs, or timestamped backups.
- [ ] Release archives run on every supported target without requiring `jq`,
      `fzf`, or a Rust toolchain.

## Review

- [ ] Code reviewed.
- [x] `PLAN.md` updated if the approach changes during implementation.
- [ ] All phase commits are clean and use the exact planned messages.
- [x] Each phase leaves the repository buildable and its completed tests green.
- [ ] `TODO.md` items are all checked off.
