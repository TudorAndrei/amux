# Plan 001: Make clear a coherent daemon state transition

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report; do not improvise. Update this plan's
> row in `plans/README.md` only after the commit succeeds.
>
> **Drift check (run first)**:
> `git diff --stat 8cf1622..HEAD -- src/ipc.rs src/daemon.rs src/state.rs`
> `src/main.rs tests/rust_smoke.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `8cf1622`, 2026-07-24

## Why this matters

`amux clear` currently unlinks persisted files directly while the daemon keeps
an in-memory `Shared` cache. Immediately afterward, `list --json` is empty but
the daemon-backed `status`, `sessions`, and picker continue to show the old
records. Direct fallback event writes can also interleave with the unlocked
clear and recreate only part of the state/log pair.

## Current state

- `src/main.rs` dispatches CLI commands; `src/ipc.rs` defines newline-delimited
  daemon requests and responses.
- `src/daemon.rs` owns the cached `State`, session views, status string, and
  revision.
- `src/state.rs` owns state-file locking and persistence.

`src/main.rs:369` currently bypasses the daemon:

```rust
Commands::Clear => state::clear(&config).map(|_| 0),
```

`src/state.rs:48-49` locks writes, while `src/state.rs:73-79` does not lock
`clear`. `src/daemon.rs:215-223` shows the established event pattern: mutate
durable state, reload it, rebuild views/status, increment `revision`, then
reply. Match that pattern for clearing.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Full check | `mise run check` | exit 0 |
| Package check | `mise run package-check` | exit 0 |
| Focused integration | `cargo test --test rust_smoke` | all tests pass |

## Scope

**In scope**: `src/ipc.rs`, `src/daemon.rs`, `src/state.rs`, `src/main.rs`,
`tests/rust_smoke.rs`, `plans/README.md`.

**Out of scope**: changing the persisted `State` schema, adding a history
command, changing `clear`'s CLI spelling, or changing tmux topology parsing.

## Git workflow

- Branch: `advisor/001-coordinate-clear`
- Commit message: `fix(state): coordinate clear with daemon state`
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Define clear semantics and lock direct clears

Add a `Clear` request variant in `src/ipc.rs`. Refactor `state::clear` so it
uses the same exclusive guard as `write_event`; the lock should cover checking
and unlinking both `state.json` and `events.jsonl`. Keep a missing file
successful. Add a small daemon-client helper beside `send_event` for sending
the clear request.

**Verify**: `cargo test --test rust_smoke
cli_clear_doctor_and_option_contracts_are_preserved` → passes.

### Step 2: Clear through the daemon whenever it is reachable

In `src/daemon.rs::handle`, process `Request::Clear` by clearing durable files,
resetting `Shared.state` to `State::initial()`, recomputing views/status from
the current topology, incrementing `revision`, and replying only after the
cache is coherent. In `src/main.rs`, try the daemon helper first and use the
locked direct clear only when the socket is unavailable. Do not shut down the
daemon as part of clear.

**Verify**: `cargo test --test rust_smoke` → all existing and new integration
tests pass.

### Step 3: Characterize both clear paths

Extend `tests/rust_smoke.rs` using its existing `amux`, `daemon_request`, and
temporary-state helpers. Cover: (1) daemon-backed clear after an event leaves
`list --json`, `status`, `sessions --json`, and `picker --rows` empty; (2) a
subsequent event recreates matching state/log entries; and (3) direct-mode
clear cannot interleave with a controlled concurrent event to leave only one
of the two durable files populated. Match existing cleanup conventions.

**Verify**: `cargo test --test rust_smoke` → new clear cases pass reliably.

## Test plan

- Model process setup/cleanup on `tests/rust_smoke.rs:131-168` and daemon setup
  on `tests/rust_smoke.rs:449-558`.
- Assert behavior, not timing: all daemon-facing views must be empty before a
  new event, then state and log must agree after it.

## Done criteria

- [ ] `cargo test --test rust_smoke` exits 0 with daemon and direct clear tests.
- [ ] `mise run check` and `mise run package-check` exit 0.
- [ ] `clear` never bypasses the state lock.
- [ ] The daemon revision changes when clear succeeds.
- [ ] Only in-scope files are modified, aside from the plan index status.

## STOP conditions

- The existing `clear` command is documented as intentionally leaving daemon
  cache intact.
- Achieving atomic state/log semantics requires changing the persisted state
  schema or CLI contract.
- Any existing `sessions --json` or picker-row assertion changes unexpectedly.

## Maintenance notes

Future state-mutating commands must be added to both the direct locked path and
the daemon cache transition. Reviewers should ensure the clear reply is sent
only after the new cached revision is fully assembled.
