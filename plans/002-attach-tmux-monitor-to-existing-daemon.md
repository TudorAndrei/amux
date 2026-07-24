# Plan 002: Attach tmux monitoring when a client joins later

> **Executor instructions**: Follow this plan step by step and update the index
> only after the planned commit succeeds.
>
> **Drift check (run first)**:
> `git diff --stat 8cf1622..HEAD -- src/main.rs src/ipc.rs src/daemon.rs`
> `src/tmux.rs tests/rust_smoke.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/001-coordinate-clear-with-daemon-state.md`
- **Category**: bug
- **Planned at**: commit `8cf1622`, 2026-07-24

## Why this matters

The daemon starts lazily from any hook event. When that first event is outside
tmux, `daemon::run` never starts a control monitor. A later in-tmux event can
be persisted, but cached session views use the empty topology forever because
there is no later monitor-attachment path.

## Current state

- `src/main.rs:140-148` sends a pane ID but not the tmux server identity with an
  event request; `src/main.rs:150-199` starts or sends to a daemon.
- `src/tmux.rs:57-67` derives the tmux socket path from `TMUX`.
- `src/daemon.rs:67-87` starts a monitor exactly once, only from the daemon
  process's inherited `TMUX` environment.

The existing `tmux::spawn(stop, server, publish)` API already accepts an
explicit server path. Reuse it; do not introduce a second tmux transport.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Focused integration | `cargo test --test rust_smoke <monitor test>` | pass |
| Full check | `mise run check` | exit 0 |
| Package check | `mise run package-check` | exit 0 |

## Scope

**In scope**: `src/main.rs`, `src/ipc.rs`, `src/daemon.rs`, `src/tmux.rs`,
`tests/rust_smoke.rs`, `plans/README.md`.

**Out of scope**: changing control-monitor reconciliation cadence, supporting
multiple tmux servers concurrently, or changing session filtering/ranking.

## Git workflow

- Branch: `advisor/002-attach-monitor`
- Commit message: `fix(daemon): attach monitor from tmux clients`

## Steps

### Step 1: Carry the client tmux server through IPC

Extend `HookRequest` with an optional server identity derived by
`tmux::server_from_env()` in `cmd_event`. Preserve the no-tmux case explicitly
as absent rather than treating it as a path. Ensure the daemon-start child still
receives its existing environment for the first-event case.

**Verify**: `cargo test --test rust_smoke <fixture test>` → pass.

### Step 2: Give the daemon a single monitor attachment lifecycle

Refactor the monitor setup currently in `daemon::run` into a helper that can be
called at startup and on an event request. Store the active server identity and
its stop signal with daemon-owned state. On a request for the same server, do
nothing; on a different server after restart, stop the old monitor before
attaching the new one. Update shutdown to stop whichever monitor is active.
Publish the same `Topology`/view/revision updates as the existing callback.

**Verify**: `cargo test --test rust_smoke <monitor test>` → pass.

### Step 3: Reproduce external-first startup in an integration test

Use the isolated tmux-server pattern in `tests/rust_smoke.rs` around the control
monitor test. Start `amux-rs daemon` with no `TMUX`, then invoke an event client
with that isolated server's `TMUX` value. Assert that daemon-backed
`sessions --json` eventually contains the tmux session and that a health
response reports a connected topology. Clean up the daemon and tmux server on
all paths.

**Verify**: `cargo test --test rust_smoke` → new external-first regression
passes.

## Test plan

- Reuse the isolated-server creation and shutdown helpers from
  `tests/rust_smoke.rs:568-940`.
- Cover duplicate in-tmux events (no second monitor) and an outside-tmux event
  (no attempt to attach a nonexistent server).

## Done criteria

- [ ] A daemon started without `TMUX` attaches after its first valid tmux
      client.
- [ ] Repeated events from one server do not create duplicate monitors.
- [ ] `mise run check` and `mise run package-check` exit 0.
- [ ] Existing daemon startup/recovery tests still pass.

## STOP conditions

- `TMUX` does not provide a stable server path in the supported tmux versions.
- Correct attachment requires a public CLI or persistent-schema change.
- An existing monitor cannot be stopped safely before replacement.

## Maintenance notes

The daemon intentionally supports one tmux server. If multi-server support is
ever added, replace this single attachment with an explicitly keyed monitor map
rather than weakening the identity check.
