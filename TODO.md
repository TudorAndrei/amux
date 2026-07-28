# TODO: Fix stuck Codex status and bound the event log

## Phase 1: Bound the event log

- [x] `src/config.rs`: add `events_per_session` (`AMUX_EVENTS_PER_SESSION`,
      default 200, `0` disables) and `events_compact_bytes`
      (`AMUX_EVENTS_COMPACT_BYTES`, default 8 MiB).
- [x] `src/event.rs`: add `compact_raw` — allowlisted strings ≤ 1 KiB, other
      strings ≤ 256 B, ≤ 32 entries per container, nesting ≤ 3, total serialized
      ≤ 4 KiB. Allowlist: `session_id`, `cwd`, `hook_event_name`, `source`,
      `turn_id`, `tool_name`, `permission_mode`, `reason`, `agent_id`,
      `agent_type`, `parent_agent_id`, `parent_session_id`, `is_subagent`.
- [x] Call `compact_raw` from `normalize_at`; cap `Record.reason` at 256 B and
      `Record.cwd` at 1 KiB there so the outer record is bounded too.
- [x] `src/state.rs`: add `compact_events(config, retain_keys)` — (1) rename to
      `.compacting` under the lock and release, (2) stream off-lock into per-key
      ring buffers carrying original ordinals, (3) sort retained by ordinal and
      write `.retained`, (4) re-acquire briefly to concatenate retained + live
      log, fsync, rename, fsync dir, unlink `.compacting`.
- [x] Drop lines whose key is absent from `state.records`, plus key-less and
      unparseable lines.
- [x] `src/state.rs`: add `adopt_orphaned_log` for a `.compacting` file left
      by a crash.
- [x] `src/daemon.rs`: call `adopt_orphaned_log` then `compact_events` in
      `run` before serving; a single-flight maintenance thread compacts when a
      write leaves the log over `events_compact_bytes`. Never compact on a
      connection thread, never on the `AMUX_NO_DAEMON` path.
- [x] Test `compact_raw`: allowlisted keys within budget byte-identical, a 10 KB
      `last_assistant_message` truncated to 256 B, a 100-key object trimmed to
      32, total serialized ≤ 4 KiB, `agent_id`/`agent_type` still readable by
      `sessions.rs::subagent_record`.
- [x] Test `compact_events`: per-key cap honoured; 500 events on key A do not
      evict key B; lines for a key absent from `state.records` dropped;
      cross-pane chronological order preserved (interleave two keys, assert
      ordinals ascending after rewrite); malformed and key-less lines dropped;
      output mode `0600`.
- [x] Test crash safety: leave a `.compacting` file, start the daemon, assert its
      events are adopted and the live log is intact.
- [x] Test `AMUX_EVENTS_PER_SESSION=0`: log grows, no compaction runs.
- [x] Commit: retention implementation and follow-up coverage are committed in
      `9161f2c`, `8c62a51`, and `469a830`.

## Phase 2: Report hook drift from `amux doctor`

- [x] `src/hooks.rs`: `pub fn drift()` + private `drift_at(paths)`, mirroring
      `install` / `install_at` (`Paths` is private).
- [x] Report missing template events, stale amux entries for unshipped events,
      matcher differences, and argument differences.
- [x] Compare **only the arguments after `event`**, never the full command
      string, so a differing launcher path (source vs TPM vs release archive) is
      not drift.
- [x] Reuse the `bin/amux event --agent` predicate from `remove_matching` so
      third-party hooks on the same events are never reported.
- [x] Limit scope to the JSON-merged integrations (Codex, Claude); do not claim
      Pi/opencode text templates are covered.
- [x] Wire into `Doctor` in `src/main.rs` with `amux install-hooks --write` as
      the printed remedy; exit code unchanged.
- [x] Test fixture homes: current install → no drift; install missing
      `PreToolUse` → reports exactly that; install with a *different launcher
      path* but identical arguments → no drift; old flag-less `Stop` command →
      argument drift; changed matcher → matcher drift; foreign hook on `Stop` →
      not reported.
- [x] Run `amux doctor` against the real `~/.codex/hooks.json` before
      re-installing; confirm it names the four stale entries.
- [x] Commit: doctor drift support is included in `9161f2c`.

## Phase 3: Re-model the Codex hook set, with template-level tests

- [x] Confirm with codex-cli whether `PreCompact`, `PostCompact`, and
      `SessionEnd` accept or require a `matcher` **before** writing the template.
- [x] Rewrite `hooks/codex/hooks.json`: nine events, explicit `--status` /
      `--attention` on every command, `SessionStart` matcher narrowed to
      `startup|resume|clear`.
- [x] `src/event.rs`: add the `sessionend` / `session_end` → `offline` branch
      **ahead of** the `["stop","end","idle","done","complete"] -> done` rule.
- [x] `src/sessions.rs`: delete the `UserPromptSubmit -> running` fixup in
      `views_from`.
- [x] Template contract test: parse `hooks/codex/hooks.json`, assert all nine
      events with exact matchers and `--status` / `--attention` flags. Confirm it
      fails on `main`.
- [x] Rendered-install test: `install_at` into a fixture `HOME`, read back
      `~/.codex/hooks.json`, execute the generated command for each of the nine
      events against a temp state dir, assert the resulting record status.
      Confirm it fails on `main` (no `PreToolUse` entry exists to execute).
- [x] Do **not** re-add a hand-fed `UserPromptSubmit → Stop → PreToolUse` replay
      test — verified to pass on the released binary already, since
      `normalize_at` infers `running` from the event name.
- [x] Extend `tests/smoke.sh` with `SessionEnd -> offline`, pinning a live
      `codex` pane so `offline` comes from the record and not the no-agent-panes
      branch in `views_from`.
- [x] Verify `bin/amux install-hooks --dry-run` renders all nine events with the
      absolute launcher path substituted.
- [x] Commit: Codex hook remodelling is included in `9161f2c`.

## Phase 4: Cut the per-hook cost

- [x] Send only `TMUX_PANE` on the daemon path; resolve session and window from
      the daemon's existing topology. Keep the `tmux display-message` lookups in
      `current_tmux_context` for the direct fallback path only.
- [x] Skip the `events.jsonl` append when `status` / `attention` / `reason` are
      all unchanged for that key; still update state.
- [x] Coalesce `tmux refresh-client` in the daemon instead of per hook in
      `cmd_event`.
- [x] Benchmark a tool-heavy Codex turn before and after (process count, wall
      clock per hook, bytes appended); record the numbers in the commit body.
- [x] Test that a status transition is still logged and a no-op repeat is not.
- [x] Commit: daemon-path performance work is committed in `9161f2c` and
      `fb16695`.

## Phase 5: Documentation

- [x] `docs/events.md`: nine-event Codex mapping, turn-scoped `Stop`, no
      `Notification` hook, no inferred status for shipped hooks, and the
      tool-less-turn limitation stated plainly.
- [x] `README.md`: `AMUX_EVENTS_PER_SESSION`, `AMUX_EVENTS_COMPACT_BYTES`,
      per-key retention with dead-key expiry, payload budget, `offline` in the
      status table, the `doctor` drift check, the removed `compact` matcher, and
      (if Phase 4 lands) that `events.jsonl` is a transition log.
- [x] Upgrade note: re-run `amux install-hooks --write`; `amux doctor` says when.
- [x] Commit: documentation is committed in `9161f2c` and `6c8d35d`.

## Verification

- [x] `mise run check` passes (build, `cargo fmt --check`, clippy `-D warnings`,
      `cargo test --all-features`, `shellcheck`, `bash -n`, `tests/smoke.sh`).
- [x] `bin/amux install-hooks --dry-run` against a copy of the real
      `~/.codex/hooks.json` shows the four stale amux entries removed and nine
      added, with no non-amux hook touched.
- [ ] Manual: run `codex` in a tmux pane on a tool-heavy task; confirm the picker
      holds `running` **across tool calls**, flips to `done` at turn end, and
      returns to `running` on the next tool call. A brief `done` between tool
      calls is expected, not a failure.
- [ ] Manual: confirm the known limitation — a tool-less turn after `Stop` still
      shows `done` — and that it is documented rather than silently wrong.
- [ ] Manual: let the session auto-compact; confirm it stays `running` via
      `PreCompact`/`PostCompact` now that `compact` left the `SessionStart`
      matcher.
- [ ] Manual: quit Codex; confirm the row becomes `offline`.
- [x] Manual: confirmed the real 553 MiB log compacted at daemon startup to
      864 KiB in 2.18 s. A malformed-byte line initially exposed a recovery bug;
      `8c5bc7c` now drops such lines, and the retried compaction completed while
      the pre-existing daemon continued serving the state directory.
- [x] Edge cases: `PermissionRequest` between `PreToolUse` and `Stop` still shows
      `attention`, and the following `PostToolUse` clears it; a subagent record
      carrying `agent_id` is still hidden after `compact_raw`; a pane renumbered
      mid-session does not evict the other key's events.
- [x] No regressions for Claude, Pi, opencode: their rows in
      `amux sessions --json` are unchanged before and after, and their hook
      templates are untouched in the diff.
- [x] No regressions in the picker: `bin/amux picker --rows` unchanged for a
      fixture state containing no `offline` records.
- [x] Confirm deleting the `views_from` fixup changes no rendered status for a
      freshly installed hook set (diff `amux sessions --json` before/after on the
      same fixture state).

## Review

- [x] Code reviewed
- [x] PLAN.md updated if the approach changed during implementation
- [x] Both new Open Questions answered (Phase 4 scope; `Stop → done` debounce)
- [x] All phase commits are clean, conventional, and leave a working build
- [ ] TODO.md items all checked off
