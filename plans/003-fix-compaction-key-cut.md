<!-- markdownlint-disable MD013 -->

# Plan 003: Make event-log compaction use one current key cut

> **Executor instructions**: Follow this plan in order. Run every check. Stop
> on a STOP condition. Update `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat 4493b48..HEAD -- src/state.rs src/daemon.rs src/daemon/maintenance.rs tests/rust_smoke.rs`
> Plan 002 is an expected change. If its final mutation boundary differs from
> Plan 002, stop and report.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/002-serialize-daemon-mutations.md`
- **Category**: bug
- **Planned at**: commit `4493b48`, 2026-08-06

## Why this matters

The daemon captures state keys before a background worker rotates the log. A
new event can enter the old log after that capture. Its maintenance request is
dropped because a worker is active, and compaction removes the valid event
because its key is not in the old set.

## Current state

- `src/daemon.rs:263-267` captures `retain_keys` and passes them to maintenance.
- `src/daemon/maintenance.rs:39-46` returns without recording a second request
  when maintenance is active.
- `src/state.rs:345-355` rotates the live log later and then uses the earlier
  key set.
- `src/state.rs:455-459` drops lines for keys outside that set.
- `src/state.rs:282-414` contains the crash-recovery order. Preserve it.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| State tests | `cargo test state::tests --all-features` | all pass |
| Maintenance tests | `cargo test daemon::maintenance --all-features` | all pass |
| Full tests | `cargo test --all-features` | all pass |
| Smoke | `bash tests/smoke.sh` | prints `ok` |

## Scope

**In scope**:

- `src/state.rs`
- `src/daemon.rs`
- `src/daemon/maintenance.rs`
- `tests/rust_smoke.rs` if needed for one process-level check
- `plans/README.md`

**Out of scope**:

- Retention count meaning and `AMUX_EVENTS_PER_SESSION=0`.
- History JSON format and event filtering.
- A new database or journal format.
- Changes to state-v1 records.

## Git workflow

- Branch: `advisor/003-fix-compaction-key-cut`
- Commit message: `fix(state): compact from a coherent state cut`
- Do not push or open a pull request unless asked.

## Steps

### Step 1: Move key selection to log rotation

Remove `retain_keys` from the daemon-to-maintenance public call. In the state
compaction code, acquire the normal state lock, load the current version-one
state, derive its keys, and rename `events.jsonl` to the compacting path while
the same lock is held. The key set and rotated file must describe one durable
point.

Do not hold the lock during the long streaming pass.

**Verify**: `cargo test state::tests --all-features` → all pass.

### Step 2: Coalesce maintenance requests

Extend `maintenance::Status` with `pending: bool`. If `schedule` runs while a
worker is active, set `pending` instead of dropping the request. After one pass,
the worker must run one more pass if pending was set. More requests can merge
into that one pass. `wait_idle` must wait until both active and pending are
false.

**Verify**: add and run a unit test that schedules during active work and sees
exactly one later pass.

### Step 3: Add the exact race test

Use barriers at the pre-rename checkpoint:

1. schedule compaction;
2. stop it before the state lock and rename;
3. commit an event for a new key;
4. release compaction;
5. wait for maintenance;
6. assert the new key's event remains in `events.jsonl`.

Also assert that a request received during active maintenance causes the
coalesced pass.

**Verify**: run the new test 50 times; every run passes without sleeps.

### Step 4: Re-run crash recovery tests

Keep all current failure stages: Renamed, Retained, RetainedWritten, Composed,
Installed, and Synced. Update only their setup for the new current-state key
selection.

**Verify**: `cargo test state::tests::every_post_rename_failure --all-features`
→ pass.

### Step 5: Run full verification

Run the command table.

## Test plan

- Use the existing checkpoint and barrier style in `src/state.rs` and
  `src/daemon/maintenance.rs`.
- Cover a new key before rotation, a new key after rotation, a pending request,
  clear waiting for both passes, and every crash-recovery stage.
- Never use elapsed time as the main order assertion.

## Done criteria

- [ ] Key selection and log rename occur under one state lock.
- [ ] Active maintenance records one pending pass.
- [ ] A new valid history event cannot be removed by an old key set.
- [ ] Clear waits for active and pending maintenance.
- [ ] All recovery-stage tests pass.
- [ ] Full Rust tests and smoke tests pass.
- [ ] Only in-scope files changed.
- [ ] The index status is `DONE`.

## STOP conditions

- Plan 002 is not complete.
- The change needs to hold the state lock during the streaming pass.
- A crash stage can leave chronology different from the current contract.
- The fix changes retention count or history format.

## Maintenance notes

Any future compaction input that controls filtering must be captured at the
same durable point as log rotation. Do not pass a state snapshot from an
earlier daemon revision into background maintenance.
