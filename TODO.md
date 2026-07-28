# TODO: Close the six minor review items

## Phase 1: Replace the mkdir lease with advisory file locking

- [x] `src/config.rs`: delete `lock_timeout_seconds` and
      `AMUX_LOCK_TIMEOUT_SECONDS`; keep `lock_acquire_timeout_ms`.
- [x] `Config::lock_dir()` → `lock_file()` at the same `state.lock` path; the
      file is created once and **never deleted** on release.
- [x] `ensure_private_dir`: remove `state.lock` when it is a directory (a
      leftover of the mkdir scheme) so the path can be opened as a file.
- [x] `src/state.rs`: merge `acquire` and `Lock::new` into one function that
      opens the lock file, retries `try_lock` on the `lock_acquire_timeout_ms`
      budget, and returns a guard **owning that `File`**.
- [x] Delete the heartbeat thread, its token, the marker file, and the
      stale-takeover branch in `acquire`.
- [x] Comment at the acquisition site: the lock belongs to the open file
      description, so every acquisition must open a fresh `File`; re-locking a
      shared or cloned handle succeeds silently and gives no exclusion.
- [x] Treat `ENOTSUP`, `EOPNOTSUPP`, and `ENOSYS` from `try_lock` as acquired
      and proceed unlocked, so an exotic filesystem degrades to today's
      behaviour instead of failing every hook (cargo's posture).
- [x] Match **both** `io::ErrorKind::Unsupported` and `cfg`-gated
      `raw_os_error()` values (45 on macOS, 95 on Linux): measured on macOS,
      `ENOSYS` maps to `Unsupported` but `ENOTSUP`/`EOPNOTSUPP` maps to
      `Uncategorized`, so matching on `ErrorKind` alone misses the main case.
- [x] Verify the Linux errno→`ErrorKind` mapping on CI rather than assuming it
      matches macOS.
- [x] Add `AMUX_LOCK=0` to disable locking outright; report the effective state
      in `amux doctor`.
- [x] Document the NFS limitation in `README.md`: `flock` can block forever on
      NFS even non-blocking, so `AMUX_STATE_DIR` on a network mount needs
      `AMUX_LOCK=0`.
- [x] Test: `AMUX_LOCK=0` skips locking and still writes valid state.
- [x] Test: an unsupported-filesystem errno is treated as acquired rather than
      propagated (inject at the helper boundary rather than requiring an exotic
      mount).
- [x] Update the `Config` fixtures in `src/daemon.rs` and `src/sessions.rs`
      tests for the removed field.
- [x] Test: same-process contention — two threads, each independently opening
      the lock file, never overlap inside the critical section.
- [x] Test: cross-process release — a spawned child that exits while holding the
      lock leaves it acquirable.
- [x] Test: the acquisition budget still errors with
      "timed out waiting for state lock" against a held lock (adapt
      `fresh_lock_is_respected_until_the_acquisition_budget_expires`).
- [x] Test: migration — a pre-existing `state.lock` **directory** is removed and
      acquisition succeeds.
- [x] Delete `dispossessed_owner_cannot_delete_a_successor_lock`,
      `lock_heartbeat_refreshes_a_long_hold`, and
      `stale_lock_is_taken_over_before_writing`.
- [x] Commit body names the mixed-version window (old mkdir binary and new flock
      binary do not exclude each other) and the loss of takeover from a wedged
      holder.
- [x] Commit: `feat!: replace the state lock lease with advisory file locking`

## Phase 2: Name quoting drift for what it is

- [x] `src/hooks.rs`: add `OwnedCommand { arguments: String, quoted: bool }`;
      change `OwnedHook.arguments` to `commands: Vec<OwnedCommand>` sorted by
      arguments, preserving multiplicity.
- [x] `command_arguments` returns plain trimmed arguments with no
      `quoted:`/`unquoted:` prefix; derive `quoted` per command from whether the
      launcher segment preceding ` event ` ends in `'`.
- [x] `drift_document`: group-coherent matching — whole-group match → no drift;
      arguments match ignoring quoting → `launcher quoting drift`; matcher
      differs → `matcher drift`; otherwise → `argument drift`.
- [x] Test: **one group containing both a quoted and an unquoted amux command**
      — the case a per-group bool cannot represent.
- [x] Test: two amux groups on the same event, one matching arguments only and
      one matching quoting only, reports drift (independent `any()` passes would
      wrongly report none).
- [x] Test: duplicate stale commands in a group are not hidden by sorting.
- [x] Rename `drift_reports_the_pre_quoting_command_as_an_argument_change` and
      assert the quoting message.
- [x] Confirm `drift_ignores_launcher_paths_and_foreign_hooks_but_reports_real_changes`
      still passes — a different launcher *path* with the same quoting is not
      drift.
- [x] Commit: `fix(doctor): distinguish launcher quoting drift from argument drift`

## Phase 3: Preserve modes and symlinks, and fsync hook-config writes

- [x] Add `src/fsutil.rs` with `pub(crate) fn sync_dir(path: &Path)`; move the
      body from `src/state.rs:347`, re-point callers, register the module.
- [x] `write_text`: resolve the destination through any symlink and target the
      resolved path, creating the temp file in **that** directory.
- [x] Read the existing mode when present, masked with `0o777` so file-type and
      setuid/setgid/sticky bits are not propagated; default `0o600` when
      creating.
- [x] Order: create temp at `0o600` → write → `set_permissions(final)` →
      `sync_all` → `rename` → `sync_dir(parent)`. Permissions must be set
      **before** `sync_all`.
- [x] Use `set_permissions` rather than `OpenOptions::mode` for the final mode so
      the result is not umask-filtered.
- [x] Test: a destination pre-created at **`0o664`** (not `0o644`, which umask
      022 would leave unchanged and make the test vacuous) still has `0o664`
      afterwards.
- [x] Test: a destination that did not exist is created at exactly `0o600` under
      a umask that would otherwise strip bits.
- [x] Test: a symlinked destination keeps its symlink and its target receives the
      new contents.
- [x] Test: setuid/setgid/sticky bits on a destination are not carried over.
- [x] Confirm `failed_atomic_write_keeps_the_original_configuration` still
      passes.
- [x] Commit: `fix(hooks): preserve configuration modes and symlinks on atomic write`

## Phase 4: Make the restore-guard test truthful

- [x] Rewrite the test around a small fallible helper that early-returns through
      `?` while a `RestoreGuard` is live — the current version exits a block
      normally carrying an `Err` and never uses `?`.
- [x] Rename it `restore_guard_runs_on_early_return`.
- [x] Do **not** extract `drive()` from `run_native`; the extraction was dropped.
- [x] Commit: `test(picker): prove the restore guard runs on early return`

## Phase 5: Report an unparseable events-per-session override

- [x] `src/config.rs:41`: reject an unparseable `AMUX_EVENTS_PER_SESSION` and
      push it onto `rejected_overrides`; keep `0` meaningful as "disable".
- [x] Extend the `doctor` assertions in
      `tests/rust_smoke.rs::cli_clear_doctor_and_option_contracts_are_preserved`:
      unparseable is reported, `0` is not, and a valid value prints its effective
      line rather than merely going unreported.
- [x] Commit: `fix(config): report an unparseable events-per-session override`

## Verification

- [x] `mise run check` passes after every phase (build, `cargo fmt --check`,
      clippy `-D warnings`, unit and integration tests, shellcheck, `bash -n`,
      `tests/smoke.sh`, `cog check`).
- [x] `mise run package-check` passes.
- [x] Manual: with a `state.lock` **directory** left behind by the current
      binary, the new binary starts, removes it, and records a hook event.
- [x] Manual: hold the lock from one process, run a hook from another, confirm it
      errors after the acquisition budget rather than hanging or taking over.
- [x] Manual: kill -9 a process holding the lock; confirm the next hook acquires
      immediately (the kernel released it) with no takeover logic involved.
- [x] Manual: run the daemon under concurrent hook events from several panes and
      confirm `state.json` stays parseable — this is the in-process thread case
      flock must cover.
- [x] Manual: `amux doctor` against an install whose launcher is unquoted reports
      quoting drift, not argument drift.
- [x] Manual: symlink a fixture `~/.claude/settings.json` into a dotfiles
      directory, run `install-hooks --write`, confirm the symlink survives, the
      target has the new contents, and its mode is unchanged.
- [x] Edge cases: `AMUX_EVENTS_PER_SESSION` unparseable vs `0` vs valid; a hook
      file that does not exist yet gets `0600`; one hook group holding both a
      quoted and an unquoted amux command.
- [x] No regressions in event retention: `compact_events`, `adopt_orphaned_log`,
      and `clear` all still work through the new lock guard.
- [x] No behaviour change in Phase 4 — test-only.

## Review

- [x] Code reviewed
- [x] PLAN.md updated if the approach changed during implementation
- [x] Remaining open question answered: remove
      `AMUX_LOCK_TIMEOUT_SECONDS`; `AMUX_LOCK=0` is the documented escape hatch
- [x] README and `docs/` updated for the removed `AMUX_LOCK_TIMEOUT_SECONDS` and
      the new locking behaviour
- [x] All phase commits are clean, conventional, and leave a working build
- [x] TODO.md items all checked off
