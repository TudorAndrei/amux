# Plan 004: Bound and reclaim daemon subscriptions

> **Executor instructions**: Follow this plan step by step and update the index
> only after the planned commit succeeds.
>
> **Drift check (run first)**:
> `git diff --stat 8cf1622..HEAD -- src/daemon.rs src/ui.rs`
> `tests/rust_smoke.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/001-coordinate-clear-with-daemon-state.md`
- **Category**: perf
- **Planned at**: commit `8cf1622`, 2026-07-24

## Why this matters

Every picker opens a daemon subscription. A subscription handler polls forever
while no revision changes, even after its picker exits; once a revision does
change, it writes the entire response while holding the global `Shared` mutex.
A disconnected or slow reader can therefore accumulate idle threads and delay
event ingestion and status requests.

## Current state

- `src/ui.rs:59-81` opens one background `daemon::subscribe` per native picker.
- `src/daemon.rs:100-106` creates one OS thread for every socket connection.
- `src/daemon.rs:225-263` loops at 50 ms, holds `Shared` while calling `reply`,
  and never observes a client disconnect until a later write fails.

Keep the existing revision-based stream protocol and the `daemon::subscribe`
receiver API used by `ui::updates`.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Daemon tests | `cargo test daemon` | all daemon unit tests pass |
| Integration | `cargo test --test rust_smoke` | all tests pass |
| Full check | `mise run check` | exit 0 |

## Scope

**In scope**: `src/daemon.rs`, daemon tests in that file and/or
`tests/rust_smoke.rs`, `plans/README.md`.

**Out of scope**: changing picker rendering, replacing Unix sockets, adding a
third-party async runtime, or changing event/status request semantics.

## Git workflow

- Branch: `advisor/004-reclaim-subscriptions`
- Commit message: `fix(daemon): reclaim idle subscriptions`

## Steps

### Step 1: Separate snapshotting from socket I/O

In the `Request::Subscribe` branch, clone the revision and response payload
while holding `Shared`, then release the mutex before calling `reply`. Repeat
that pattern for later revision updates. Configure bounded write behavior so a
non-reading client cannot block forever; preserve the original error when a
write fails.

**Verify**: `cargo test daemon` → passes.

### Step 2: Detect closed subscribers during idle periods

Make the subscription stream observable for peer closure without waiting for a
new revision (for example, nonblocking peer probing on the read side). Exit the
handler immediately on EOF or socket failure. Do not treat a transient
would-block state as a disconnect, and do not busy-spin.

**Verify**: `cargo test daemon` → a closed-peer regression passes.

### Step 3: Lock down healthy and unhealthy client behavior

Add a focused unit test using a local Unix-stream pair, or an integration test
using the existing raw `daemon_request` style, that closes a subscriber before
a new revision and confirms its handler terminates. Add a second test proving a
normal `daemon::subscribe` client still receives an event revision after the
change. If a test-only helper is needed to observe handler exit, keep it behind
`#[cfg(test)]` and do not expose it in the CLI protocol.

**Verify**: `mise run check` → exit 0.

## Test plan

- Use `tests/rust_smoke.rs:80-100` as the normal update-consumer pattern.
- Add a unit-level closure test rather than timing a process's thread count.
- Ensure an event request completes even when a separate subscriber is slow or
  disconnected.

## Done criteria

- [ ] No subscription response is written while holding `Shared`.
- [ ] A closed idle subscriber exits without needing a new revision.
- [ ] A healthy picker subscription receives its initial and changed views.
- [ ] `mise run check` and `mise run package-check` exit 0.

## STOP conditions

- Peer closure cannot be detected with the current blocking socket design
  without changing the protocol.
- The required write timeout drops updates for a healthy local picker.
- The implementation needs a new runtime/dependency not already approved.

## Maintenance notes

Keep subscription cleanup and backpressure behavior local to the daemon. Any
new streaming request must snapshot shared state before doing network I/O.
