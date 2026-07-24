# Plan 003: Enforce private permissions for every state write path

> **Executor instructions**: Follow this plan step by step and update the index
> only after the planned commit succeeds.
>
> **Drift check (run first)**:
> `git diff --stat 8cf1622..HEAD -- src/state.rs src/daemon.rs`
> `tests/rust_smoke.rs README.md`

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `8cf1622`, 2026-07-24

## Why this matters

Raw hook payloads are retained in `events.jsonl`. The normal daemon creates a
private `0700` state directory, but fallback writes create the directory and
files using process defaults. Under a typical permissive umask, other local
users may read diagnostics that the daemon path protects.

## Current state

`src/daemon.rs:31-34` explicitly creates and chmods the directory:

```rust
fs::create_dir_all(&config.state_dir)?;
fs::set_permissions(&config.state_dir, fs::Permissions::from_mode(0o700))?;
```

In contrast, `src/state.rs:20-21` only calls `create_dir_all`;
`src/state.rs:50-70` creates/appends event and state files.
`src/main.rs:150-173` invokes this path
when `AMUX_NO_DAEMON` is set or the daemon cannot be reached. The project is
Unix-only already: daemon IPC uses Unix sockets.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Focused integration | `cargo test --test rust_smoke` | all tests pass |
| Full check | `mise run check` | exit 0 |
| Package check | `mise run package-check` | exit 0 |

## Scope

**In scope**: `src/state.rs`, minimal duplicate permission setup in
`src/daemon.rs` only if extracted, `tests/rust_smoke.rs`, `README.md`, and the
plan index.

**Out of scope**: encrypting event payloads, changing event retention, changing
`AMUX_STATE_DIR`, or adding Windows support.

## Git workflow

- Branch: `advisor/003-private-state-storage`
- Commit message: `fix(state): protect fallback event storage`

## Steps

### Step 1: Centralize private state-directory creation

Add one `state`-module helper that creates the configured directory and enforces
mode `0700`, returning an error if it cannot. Call it before direct locking and
from daemon startup so fallback and daemon paths cannot drift. Do not rely only
on the process umask.

**Verify**: `cargo test --test rust_smoke` → existing tests pass.

### Step 2: Create and remediate persisted files as owner-only

Use Unix creation modes for new `events.jsonl` and temporary/state files, and
apply `0600` to existing persisted files before use so an older permissive file
is remediated. Preserve append ordering, sync behavior, and atomic rename of
`state.json`.

**Verify**: `cargo test --test rust_smoke` → existing persistence tests pass.

### Step 3: Add a fallback permission regression and document it

In `tests/rust_smoke.rs`, invoke the normal CLI with `AMUX_NO_DAEMON=1` and a
deliberately non-private temporary directory. Use `PermissionsExt` to assert
`0700` for the state directory and `0600` for both persisted files after an
event. Extend the README state-model paragraph with the owner-only storage
guarantee.

**Verify**: `mise run check && mise run package-check` → both exit 0.

## Test plan

- Follow the daemon-disabled `amux` helper setup at `tests/rust_smoke.rs:36-43`.
- Test new files and pre-existing overly broad files; do not assert exact modes
  on parent directories owned by the test harness.

## Done criteria

- [ ] Fallback-created directory is mode `0700`.
- [ ] Fallback `state.json` and `events.jsonl` are mode `0600`.
- [ ] Existing persistence and package checks pass.
- [ ] README accurately describes the behavior.

## STOP conditions

- A supported platform lacks the Unix permission APIs used elsewhere in this
  project.
- Applying restrictive permissions breaks a documented shared-state workflow.

## Maintenance notes

Any new file below `Config::state_dir` must be created through the same helper.
Reviewers should verify both daemon and daemon-disabled paths use it.
