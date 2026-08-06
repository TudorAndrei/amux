<!-- markdownlint-disable MD013 -->

# Plan 002: Keep daemon mutations in durable commit order

> **Executor instructions**: Follow this plan step by step. Run every check.
> Stop on a STOP condition. Update this plan's row in `plans/README.md` when
> done.
>
> **Drift check (run first)**:
> `git diff --stat 4493b48..HEAD -- src/daemon.rs tests/rust_smoke.rs`
> Compare changed code with the excerpts below. A mismatch is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `4493b48`, 2026-08-06

## Why this matters

The file lock orders durable event commits. A handler releases that lock before
it gets the separate live-model mutex. Two handlers can commit A then B but
publish B then A. The file then contains B while health, watch, and the picker
show the older A snapshot. Event and clear operations have the same split.

## Current state

- `src/daemon.rs:175-179` starts one handler thread for each client.
- `src/daemon.rs:252-269` persists an event and later locks `Shared` to publish
  the returned full state:

```rust
let commit = intake::persist(config, request, tmux)?;
let mut guard = shared.lock()?;
let revision = guard.model.apply_event_state(config, commit.state);
```

- `src/daemon.rs:271-282` clears disk before it separately clears the model.
- `src/state.rs:143-180` owns the durable file lock and returns an owned state.
- Lock poison errors become `String`; match this pattern.
- Deterministic race tests use barriers and checkpoints in
  `src/daemon/maintenance.rs:91-139` and `src/state.rs` compaction tests.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Focused tests | `cargo test daemon:: --all-features` | all daemon tests pass |
| Full tests | `cargo test --all-features` | all tests pass |
| Smoke | `bash tests/smoke.sh` | prints `ok` |
| Style | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:

- `src/daemon.rs`
- `tests/rust_smoke.rs` only if a process-level test is needed
- `plans/README.md`

**Out of scope**:

- State-v1 JSON shape.
- IPC request or response shape.
- Event classification and compaction policy.
- Any change to the canonical TPM launcher path.

## Git workflow

- Branch: `advisor/002-serialize-daemon-mutations`
- Commit message: `fix(daemon): publish mutations in commit order`
- Do not push or open a pull request unless asked.

## Steps

### Step 1: Add a daemon mutation coordinator

Add a separate `Arc<Mutex<()>>` mutation lock to `Shared`. Do not use the
existing `Shared` mutex as the long file-I/O lock because monitor publication
must stay responsive. Initialize it once in `run` and in every test `Shared`.

Add a small helper that clones and locks this coordinator and converts poison
to `"daemon mutation lock poisoned"`.

**Verify**: `cargo check --all-features` → exit 0.

### Step 2: Put event commit and publication in one ordered section

For `Request::Event`, acquire the mutation lock before `intake::persist`.
Keep it until `apply_event_state` has published the exact returned state and
the compaction decision has captured its keys. Do not hold the live-model
mutex during file I/O.

The required lock order is:

1. mutation coordinator;
2. state file lock inside `intake::persist`;
3. live-model mutex after persistence returns.

No code path can take these in the reverse order.

**Verify**: `cargo test daemon:: --all-features` → all pass.

### Step 3: Put clear in the same ordered section

For `Request::Clear`, acquire the mutation coordinator before maintenance
wait/clear. Keep it through `LiveModel::clear`. Preserve the rule that clear
waits for active maintenance.

**Verify**: `cargo test daemon::maintenance --all-features` → all pass.

### Step 4: Add a deterministic order test

Add a test-only checkpoint immediately after durable event commit and before
live-model publication. Model it on `compact_events_with_checkpoint`; it must
not exist in the release API.

Use two threads and barriers:

- A enters the ordered section and stops after its durable commit.
- B starts and attempts its commit.
- Confirm B cannot pass A while A is stopped.
- Release A, then confirm the disk state and live model both contain A and B.

Add the same order assertion for an event that overlaps clear. The result must
match one valid order: clear then event, or event then clear. Disk and live
model must agree.

**Verify**: run the new test 50 times with a shell loop; every run passes.

### Step 5: Run full verification

Run all commands in the command table.

**Verify**: each command has the stated result.

## Test plan

- Add tests in `src/daemon.rs` beside the existing socket and monitor tests.
- Cover event/event order and event/clear order with barriers.
- Assert both record keys, model revision order, and disk/model agreement.
- Do not use sleeps as the order mechanism.

## Done criteria

- [ ] All durable daemon mutations use one coordinator.
- [ ] No file I/O occurs while the live-model mutex is held.
- [ ] Deterministic event/event and event/clear tests pass 50 times.
- [ ] `cargo test --all-features` passes.
- [ ] `bash tests/smoke.sh` prints `ok`.
- [ ] The state and IPC schemas did not change.
- [ ] Only in-scope files changed.
- [ ] The index status is `DONE`.

## STOP conditions

- The fix requires a new IPC field or state-file field.
- A required path takes the live-model mutex before the mutation coordinator.
- The deterministic test needs timing sleeps to reproduce the bug.
- Clear can return while maintenance still installs an old log.

## Maintenance notes

All future daemon commands that mutate disk and the live model must use this
coordinator. Review lock order carefully. A later batch-event feature must not
bypass it.
