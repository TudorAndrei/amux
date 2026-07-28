# Plan: Remove the status-line integration and clear the deferred-work backlog

## Goal

Remove the tmux status-line integration, which is unused, and close the eight
findings recorded under "Deferred work" in `ISSUES.md` — the confirmed results of
the `improve` audit at `8cf1622` that were held back from plans 001–005. The
findings are unrelated to each other, so this is a sequence of small independent
phases rather than one design. Each was re-verified against the tree at
`ffc6e63`; three have a sharper failure mode than the backlog entry described,
and one may end in a documented rejection.

## Approach

The status-line removal goes **first** — not because it is the most urgent, but
because it deletes code that later phases would otherwise harden, test, and
document. Phase 3's escaping work in particular shrinks once `render::status` is
gone. The remaining phases are ordered by how badly each finding bites:

1. **Remove the status-line integration** — unused; deleting it first shrinks
   everything after.
2. **Recoverable state lock** — can wedge every writer permanently. More exposed
   than when filed, because event-log compaction takes the same lock.
3. **Atomic hook-config writes** — can corrupt files amux does not own.
4. **Escape untrusted rendered text** — hook- and tmux-supplied strings reach a
   terminal unescaped.
5. **Picker terminal restore** — a draw error leaves the terminal in raw mode.
6. **Bound daemon requests** — unbounded read into memory from the socket.
7. **Validate configuration ranges** — a negative `AMUX_STALE_SECONDS` discards
   all state.
8. **Align the conventional-commit gate** — the gate does not run on this
   repo's actual merge path.
9. **Projection indexing** — measure first; likely rejected as premature.

Each phase stands alone and can be dropped without affecting the others.

### What the status-line integration spans

Wider than the `amux status` command alone:

- `amux.tmux` — `@amux-status`, `@amux-status-command`, and the `status-right`
  splice/unsplice logic, including the "remove the previously registered
  segment" handling on reload.
- `src/main.rs` — the `Status` subcommand and `cmd_status`.
- `src/render.rs` — `render::status`, roughly half the module.
- `src/daemon.rs` — `Shared.status`, `Request::Status`, `cached_status`, and
  five sites that recompute the segment on every state or topology change.
- `src/ipc.rs` — `Response.status` and the `Response::status` constructor;
  `Response::state` and `Response::health` also carry the field.
- `tests/rust_smoke.rs:1228-1316` — the plugin status-wiring integration test.
- `README.md` — the `@amux-status` option and the manual `status-right` recipe.

**`schedule_refresh` (src/daemon.rs:367) goes with it.** Its `tmux
refresh-client -S` exists only to redraw the status line; the picker receives
updates over its daemon subscription and `next-attention` switches clients
directly. `refresh_tmux` in `src/main.rs:117` is the same thing on the
fallback path. Both become dead once the segment is gone — a real simplification,
since the refresh coalescing added in `ffc6e63` was itself only there to make
per-hook status redraws affordable. The unrelated `refresh-client -f no-output`
in `src/tmux.rs:346` is control-mode setup and stays.

**This is a breaking change for anyone else.** `@amux-status` is a documented
option and `#(/path/to/amux/bin/amux status)` a documented manual recipe, so a
third-party config referencing either breaks on upgrade. Since amux ships
release archives through TPM, this wants a `feat!`/`BREAKING CHANGE` trailer so
cocogitto bumps accordingly, and a README note. If keeping `amux status` as a
manual-only command is preferable to a hard removal, that is a smaller change —
drop only the `amux.tmux` wiring and the daemon caching, keep the subcommand.
This plan assumes full removal as asked.

### Findings that changed on re-verification

- **Hook-config quoting is a correctness bug, not hardening.** `template_json`
  (src/hooks.rs:236) substitutes the launcher path into the template text and
  *then* parses it as JSON. A path containing `"` or `\` produces invalid JSON
  and fails the install outright; a path containing a space produces valid JSON
  whose command string the agent's shell silently mis-parses.
- **The tmux-format injection is latent, not live.** `render::status`
  (src/render.rs:42) emits only an icon, a count, and a style — no untrusted
  text — so `#[...]` injection into the status line is not currently reachable.
  Phase 1 deletes that function outright, which removes the tmux-format sink
  entirely and reduces Phase 4 to the terminal sinks: `render::list`,
  `render::sessions`, `ui::render_rows`, and the ratatui rows.
- **Stale-lock detection has to be time-based.** There is no `libc` or `nix`
  dependency (`Cargo.toml`), so liveness cannot be checked with `kill(pid, 0)`
  without adding one. Not worth a dependency; a takeover timeout on the lock's
  own mtime is sufficient and portable.
- **The commit gate never runs here.** `.github/workflows/ci.yml:31` runs
  `cog check` only `if: github.event_name == 'pull_request'`, and `mise run
  check` (mise.toml:51) does not include the `cog-check` task defined at
  mise.toml:63. This repo merges straight to `main` without PRs, so nothing
  validates the commits that `cog bump --auto` (ci.yml:66) derives a version and
  changelog from.

### Out of scope

- Anything already shipped by the Codex retention plan (`ffc6e63`).
- Daemon/fallback cache coherence — still open, still orthogonal.
- Adding a `libc`/`nix` dependency.

## Implementation Phases

### Phase 1: Remove the tmux status-line integration

- `amux.tmux`: delete `@amux-status`, `@amux-status-command`, and all
  `status-right` manipulation, including the previous-segment cleanup on reload.
  Keep the picker binding, `next-attention` binding, and `AMUX_ROOT` export.
- `src/main.rs`: remove the `Status` subcommand, `cmd_status`, and
  `refresh_tmux` plus its call site in `cmd_event`.
- `src/render.rs`: delete `render::status`; keep `list` and `sessions`.
- `src/daemon.rs`: remove `Shared.status`, `Request::Status`, `cached_status`,
  `schedule_refresh`, `Shared.refresh_pending`, and every recomputation site.
- `src/ipc.rs`: remove `Response.status` and `Response::status`; drop the field
  from `state` and `health`, and update `daemon::health`'s tuple accordingly.
- `tests/rust_smoke.rs`: delete the status-wiring test at 1228-1316; keep the
  surrounding plugin-reload coverage and adjust it to assert the status options
  are *not* set.
- `README.md`: remove the `@amux-status` option and the manual `status-right`
  recipe; note the removal in the upgrade guidance.
- Use a `feat!` commit with a `BREAKING CHANGE` trailer so cocogitto bumps the
  major-ish version and the changelog records it.
  **Commit:** `feat!: remove the tmux status-line integration`

### Phase 2: Make the state lock recoverable

- `src/config.rs`: add `lock_timeout_seconds` from `AMUX_LOCK_TIMEOUT_SECONDS`,
  default 30.
- `src/state.rs::acquire`: on `AlreadyExists`, take over the lock directory when
  its mtime is older than the takeover timeout, then retry the create once;
  keep looping if another process wins the race.
- Refresh the lock's mtime on acquisition so a long legitimate hold is never
  mistaken for abandonment. Compaction from `ffc6e63` already keeps holds short,
  so 30 s is far above any real hold.
- Keep the existing 5 s acquisition budget; only the abandoned-lock case changes.
  **Commit:** `fix(state): take over an abandoned state lock`

### Phase 3: Write hook configuration atomically and quote the launcher path

- `src/hooks.rs::write_text`: temp file beside the destination, fsync, rename
  over. Keep the existing `backup()` call.
- Substitute the launcher path as a JSON-encoded, shell-quoted value in
  `template_json` and `template_text`, so it is escaped before the document is
  parsed rather than after.
- Confirm `remove_matching`'s `bin/amux event --agent` predicate still matches
  quoted commands — if not, upgrades duplicate entries instead of replacing them.
  **Commit:**
  `fix(hooks): write agent configuration atomically with a quoted launcher path`

### Phase 4: Escape untrusted text before rendering

- Add one `sanitize` helper that rewrites C0/C1 control characters in caret
  notation (`\x1b` → `^[`), so the evidence survives instead of being stripped.
- Apply in `render::list`, `render::sessions`, `ui::render_rows`,
  `ui::row_line`, and `ui::detail_text` — every sink carrying hook `reason`,
  tmux `pane_title`, `cwd`, or a session name.
- The tmux-format (`#[`) case is gone with Phase 1; no tmux-bound sink remains.
  **Commit:** `fix(render): neutralise control characters in agent-supplied text`

### Phase 5: Restore the terminal on every picker exit

- `ui::run_native` calls `ratatui::restore()` only on the success path; the `?`
  on `terminal.draw`, `event::poll`, and `event::read` all return with the
  terminal in raw mode and the alternate screen active.
- Replace with an RAII guard whose `Drop` restores, so errors and panics unwind
  cleanly.
  **Commit:** `fix(picker): restore the terminal when the picker exits with an error`

### Phase 6: Bound daemon request size

- `daemon::handle` reads with `read_line` into an unbounded `String`. Wrap the
  reader in `.take(MAX_REQUEST_BYTES)` (1 MiB — generous now that `compact_raw`
  caps payloads at 4 KiB) and reject a request that hits the limit without a
  newline instead of deserializing whatever arrived.
- Reply with a structured error rather than closing the connection silently.
  **Commit:** `fix(daemon): reject oversized requests before deserialization`

### Phase 7: Validate configuration ranges

- `src/config.rs`: reject `AMUX_STALE_SECONDS <= 0` and
  `AMUX_EVENTS_COMPACT_BYTES == 0`, falling back to the documented defaults.
  `AMUX_EVENTS_PER_SESSION == 0` stays meaningful — it disables compaction.
- Surface effective values and any rejected override in `amux doctor`; the hook
  path must stay quiet on stderr.
  **Commit:** `fix(config): reject out-of-range environment overrides`

### Phase 8: Run the conventional-commit gate on the real merge path

- Add `cog-check` to the `check` task in `mise.toml`, where it costs nothing.
- Add a `cog check --from-latest-tag` step to `prepare-release` in
  `.github/workflows/ci.yml`, before `cog bump --auto`.
- Keep the existing pull-request step; this adds the push path rather than
  replacing it.
  **Commit:** `ci: check conventional commits on the release path`

### Phase 9: Measure projection cost, then index or reject

- `sessions::views_from` re-filters every pane and every record per session, so
  it is O(sessions × (panes + records)) on each topology change.
- **Measure first**, at 100 sessions / 200 panes / 200 records — roughly 3× the
  real deployment, which measured 33 sessions, 58 panes, and 33 records on
  2026-07-28. At that real scale the projection is about 3,000 operations per
  rebuild, so REJECTED is the expected outcome and the number is the deliverable.
- Above ~5 ms: group panes and records by session name in one pass and index per
  session. Below: do not implement — record the measurement in `ISSUES.md` and
  mark the finding REJECTED as premature.
  **Commit (implemented):** `perf(sessions): index panes and records by session`
  **Commit (rejected):**
  `docs(issues): record projection cost measurement and close the finding`

## Risks & Tradeoffs

- **Removing `amux status` breaks third-party configs.** Documented option,
  documented manual recipe, shipped through TPM. Mitigated by a `BREAKING
  CHANGE` trailer and a README upgrade note, not eliminated. The narrower
  alternative — keep the subcommand, drop only the wiring — is available if a
  hard removal is not wanted.
- **Dropping `refresh_tmux` changes hook behaviour subtly.** Any user-authored
  `status-right` segment that shells out to amux stops being nudged to redraw. It
  will still refresh on tmux's own status interval. Worth stating in the release
  note.
- **Lock takeover can race a live-but-slow writer.** A process stalled longer
  than the takeover timeout — SIGSTOP, a pathological filesystem — could have its
  lock stolen, allowing two concurrent writers. Mitigated by refreshing the mtime
  on acquisition and a timeout far above any real hold. It trades a rare
  corruption window against a permanent wedge; the wedge is worse.
- **Quoting the launcher path changes every generated command string.** Existing
  installs show as drift in `amux doctor` and need one `install-hooks --write`.
- **Escaping changes visible output**, which will show up as diffs in
  `picker --rows` fixtures. That is the point.
- **Phase 9 may deliver nothing but a number.** Planned for explicitly; the
  alternative is carrying an unmeasured performance finding indefinitely.

## Open Questions

All resolved before implementation.

- Phase 1: **hard removal** of `amux status`, including the subcommand,
  `render::status`, and the daemon plumbing. Carries a `BREAKING CHANGE`
  trailer.
- Phase 2: takeover timeout **30 s**, overridable via
  `AMUX_LOCK_TIMEOUT_SECONDS` — roughly 30× the longest real hold now that
  compaction takes the lock only briefly.
- Phase 4: **escape visibly in caret notation** (`^[`) rather than stripping, so
  a debugging surface like `amux list` still shows that the payload contained
  control characters.
- Phase 9: measured against the real deployment rather than assumed — 33
  sessions, 58 panes, 33 records as of 2026-07-28. The ceiling is set at
  100/200/200, roughly 3× that, and a REJECTED outcome is the expected result.
