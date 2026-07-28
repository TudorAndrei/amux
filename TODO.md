# TODO: Remove the status-line integration and clear the deferred-work backlog

## Phase 1: Remove the tmux status-line integration

- [x] `amux.tmux`: delete `@amux-status`, `@amux-status-command`, and all
      `status-right` splice/unsplice logic including the previous-segment
      cleanup on reload. Keep the picker binding, `next-attention` binding, and
      the `AMUX_ROOT` export.
- [x] `src/main.rs`: remove the `Status` subcommand, `cmd_status`, and
      `refresh_tmux` plus its call site in `cmd_event`.
- [x] `src/render.rs`: delete `render::status`; keep `list` and `sessions`.
- [x] `src/daemon.rs`: remove `Shared.status`, `Shared.refresh_pending`,
      `Request::Status`, `cached_status`, `schedule_refresh`, and all five
      status-recomputation sites.
- [x] `src/ipc.rs`: remove `Response.status` and `Response::status`; drop the
      field from `state` and `health`, and update `daemon::health`'s return
      tuple and its callers (`cached_views`).
- [x] `tests/rust_smoke.rs`: delete the status-wiring test at 1228-1316; keep
      the surrounding plugin-reload coverage and flip it to assert the status
      options are *not* set.
- [x] `README.md`: remove the `@amux-status` option and the manual
      `status-right` recipe; add upgrade guidance for both.
- [x] Confirm nothing else shells out to `bin/amux status`
      (`grep -rn "amux status"` across the repo, including `scripts/`).
- [x] Commit with a `BREAKING CHANGE` trailer:
      `feat!: remove the tmux status-line integration`

## Phase 2: Make the state lock recoverable

- [x] `src/config.rs`: add `lock_timeout_seconds` from
      `AMUX_LOCK_TIMEOUT_SECONDS`, default 30.
- [x] `src/state.rs::acquire`: on `AlreadyExists`, take over the lock directory
      when its mtime exceeds the takeover timeout, then retry the create once;
      keep looping if another process wins the race.
- [x] Refresh the lock directory's mtime on acquisition.
- [x] Keep the existing 5 s acquisition budget unchanged.
- [x] Test: a stale lock directory is taken over and `write_event` succeeds.
- [x] Test: a fresh lock directory is respected and the writer still reports
      "timed out waiting for state lock".
- [x] Test: two concurrent takeovers leave exactly one winner and valid
      `state.json`.
- [x] Commit: `fix(state): take over an abandoned state lock`

## Phase 3: Write hook configuration atomically and quote the launcher path

- [x] `src/hooks.rs::write_text`: temp file beside the destination, fsync,
      rename over; keep the existing `backup()` call.
- [x] Substitute `__AMUX_BIN__` as a JSON-encoded, shell-quoted value in both
      `template_json` (src/hooks.rs:236) and `template_text` (src/hooks.rs:242).
- [x] Confirm `remove_matching`'s `bin/amux event --agent` predicate still
      matches quoted commands — if not, upgrades duplicate entries instead of
      replacing them.
- [x] Test: a launcher path containing a space renders a command the shell
      parses as a single argument.
- [x] Test: a launcher path containing `"` and `\` renders valid JSON and
      installs cleanly (currently fails to parse).
- [x] Test: an interrupted write leaves the original file intact and readable.
- [x] Test: `amux doctor` reports the quoting change as drift for an install
      made by the previous version.
- [x] Commit:
      `fix(hooks): write agent configuration atomically with a quoted launcher path`

## Phase 4: Escape untrusted text before rendering

- [x] Add a `sanitize` helper that rewrites C0/C1 control characters in caret
      notation (`\x1b` → `^[`) rather than stripping them.
- [x] Apply in `render::list`, `render::sessions`, `ui::render_rows`,
      `ui::row_line`, and `ui::detail_text`.
- [x] Test: a `reason` containing `\x1b[2J` and a raw newline renders inert and
      visible (`^[[2J`, `^J`) in each sink.
- [x] Test: ordinary UTF-8 — CJK, combining marks, emoji — passes through
      unchanged.
- [x] Update `picker --rows` fixtures if the escaping changes their output.
- [x] Commit: `fix(render): neutralise control characters in agent-supplied text`

## Phase 5: Restore the terminal on every picker exit

- [x] Replace the bare `ratatui::init()` / `ratatui::restore()` pair in
      `ui::run_native` with an RAII guard that restores on drop.
- [x] Confirm every `?` in the loop — `terminal.draw`, `event::poll`,
      `event::read` — now unwinds through the guard.
- [x] Test: a draw error propagates and the restore still runs.
- [x] Manual: kill the picker mid-draw and confirm the shell is left usable.
- [x] Commit: `fix(picker): restore the terminal when the picker exits with an error`

## Phase 6: Bound daemon request size

- [x] `daemon::handle`: wrap the reader in `.take(MAX_REQUEST_BYTES)` (1 MiB)
      and reject a request that reaches the limit without a newline.
- [x] Reply with a structured error instead of closing the connection silently.
- [x] Test: an oversized request is rejected, the daemon stays up, and the next
      request is served normally.
- [x] Test: a normal-sized request is unaffected.
- [x] Commit: `fix(daemon): reject oversized requests before deserialization`

## Phase 7: Validate configuration ranges

- [x] `src/config.rs`: reject `AMUX_STALE_SECONDS <= 0` and
      `AMUX_EVENTS_COMPACT_BYTES == 0`, falling back to the documented defaults.
- [x] Leave `AMUX_EVENTS_PER_SESSION == 0` meaningful — it disables compaction.
- [x] Surface effective values and any rejected override in `amux doctor`; keep
      the hook path silent on stderr.
- [x] Test: each invalid value falls back to its default.
- [x] Test: `doctor` names the rejected override.
- [x] Test: `AMUX_EVENTS_PER_SESSION=0` is not reported as invalid.
- [x] Commit: `fix(config): reject out-of-range environment overrides`

## Phase 8: Run the conventional-commit gate on the real merge path

- [x] Add `cog-check` to the `check` task in `mise.toml` (defined at
      mise.toml:63 but never invoked by `check`).
- [x] Add a `cog check --from-latest-tag` step to `prepare-release` in
      `.github/workflows/ci.yml`, before `cog bump --auto` (ci.yml:66).
- [x] Keep the existing pull-request step at ci.yml:31.
- [x] Verify on a scratch branch that a non-conforming commit fails the gate
      locally, and that the added CI step is one cheap step on a job that
      already runs.
- [x] Commit: `ci: check conventional commits on the release path`

## Phase 9: Measure projection cost, then index or reject

- [x] Add a measurement test for `sessions::views_from` at 100 sessions, 200
      panes, 200 records (~3× the real deployment measured on 2026-07-28: 33
      sessions, 58 panes, 33 records); record the rebuild time.
- [x] If above ~5 ms: group panes and records by session name in one pass and
      index per session, then re-measure and record the improvement.
- [x] If below: do not implement. Record the measurement in `ISSUES.md` and mark
      the finding REJECTED as premature.
- [x] Commit (implemented): `perf(sessions): index panes and records by session`
- [x] Commit (rejected):
      `docs(issues): record projection cost measurement and close the finding`

## Verification

- [x] `mise run check` passes after every phase (build, `cargo fmt --check`,
      clippy `-D warnings`, `cargo test --all-features`, `shellcheck`, `bash -n`,
      `tests/smoke.sh`).
- [x] `mise run package-check` passes.
- [x] Manual: reload the tmux plugin and confirm `status-right` is left exactly
      as the user configured it, with no amux segment added and no leftover
      `@amux-status-command` option.
- [x] Manual: confirm the picker still updates live after `schedule_refresh` and
      `refresh_tmux` are gone — it must, since it uses the daemon subscription.
- [x] Manual: kill a process holding `state.lock`, then confirm the next hook
      recovers within the takeover timeout instead of wedging.
- [x] Manual: `amux install-hooks --write` from a path containing a space
      produces hooks Codex actually executes — verify a real event lands in
      `state.json`.
- [x] Manual: feed a hook payload whose `reason` contains an ANSI clear-screen
      sequence; confirm `amux list`, `amux sessions`, and the picker render it
      inert.
- [x] Manual: interrupt the picker mid-draw; confirm the terminal is usable.
- [x] Edge cases: `AMUX_STALE_SECONDS=-1` no longer discards state;
      `AMUX_EVENTS_PER_SESSION=0` still disables compaction; a lock taken over
      mid-write leaves `state.json` parseable.
- [x] No regressions in the Codex retention work from `ffc6e63`: event
      compaction, doctor drift, and the nine-event mapping still behave as
      `docs/events.md` describes.
- [x] `ISSUES.md` "Deferred work" updated — each item removed as it lands, or
      annotated with its resolution.

## Review

- [x] Code reviewed
- [x] PLAN.md updated if the approach changed during implementation
- [x] Open Questions stay resolved as recorded in PLAN.md (hard removal; 30 s
      takeover; caret-notation escaping; 100/200/200 measurement ceiling)
- [x] Phase 1 commit carries a `BREAKING CHANGE` trailer and cocogitto bumps
      accordingly
- [x] All phase commits are clean, conventional, and leave a working build
- [x] TODO.md items all checked off
