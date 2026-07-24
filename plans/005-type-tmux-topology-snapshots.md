# Plan 005: Parse tmux topology into typed records once

> **Executor instructions**: Follow this plan step by step and update the index
> only after the planned commit succeeds.
>
> **Drift check (run first)**:
> `git diff --stat 8cf1622..HEAD -- src/tmux.rs src/sessions.rs`
> `src/daemon.rs src/ipc.rs tests/rust_smoke.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/002-attach-tmux-monitor-to-existing-daemon.md`
- **Category**: tech-debt
- **Planned at**: commit `8cf1622`, 2026-07-24

## Why this matters

Tmux session names, titles, commands, and paths are user-controlled metadata.
The daemon currently persists those fields as pipe-delimited strings and three
consumers split them positionally. A literal pipe shifts fields, so a record may
be omitted or matched to the wrong session/pane; future field changes must be
edited in lockstep across multiple modules.

## Current state

`src/tmux.rs:13-20` defines:

```rust
pub struct Topology {
    pub sessions: Vec<String>,
    pub panes: Vec<String>,
    // ...
}
```

`src/tmux.rs:33-45` emits pipe-delimited control data. `src/sessions.rs:31-84`
separately creates and parses pipe records for both direct and cached views;
`src/daemon.rs:364-375` parses a pane line once again for event context. Keep
the documented session sorting, agent selection, and CLI JSON shapes intact.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Focused tests | `cargo test tmux sessions` | all matching tests pass |
| Integration | `cargo test --test rust_smoke` | all tests pass |
| Full check | `mise run check` | exit 0 |
| Package check | `mise run package-check` | exit 0 |

## Scope

**In scope**: `src/tmux.rs`, `src/sessions.rs`, `src/daemon.rs`, `src/ipc.rs`
only if response serialization needs adapting, relevant tests, and the index.

**Out of scope**: changing tmux commands, filtering rules, daemon revision
semantics, public `sessions --json` fields, or multi-server monitoring.

## Git workflow

- Branch: `advisor/005-typed-topology`
- Commit message: `refactor(tmux): type topology snapshots`

## Steps

### Step 1: Define typed topology records at the tmux boundary

Move the session/pane data structures to `src/tmux.rs` (or an equally focused
module) and make `Topology` store typed, serializable records rather than raw
lines. Centralize both direct `tmux` output and control-client output parsing
there. Use an explicit field separator that cannot occur in supported tmux
metadata, validate the exact field count, and return/record a clear error for a
malformed snapshot rather than silently shifting fields.

**Verify**: `cargo test tmux` → parser tests pass.

### Step 2: Migrate all consumers without changing session behavior

Make `sessions::views` and `views_with_topology` consume the typed records;
remove local positional splitting. Change `daemon::context_for_pane` to inspect
the typed pane collection. Retain existing session/status sort order and the
highest-priority target-pane selection in `sessions::views_from`.

**Verify**: `cargo test sessions` → session-view tests pass.

### Step 3: Add delimiter and regression coverage

Add parser/view tests containing `|` in session names, titles, commands, and
working directories. Assert that the resulting `SessionView` retains those
values and associates the expected pane. Update the existing isolated tmux
integration assertions only as needed for the internal health topology shape;
do not change `sessions --json` expectations.

**Verify**: run `cargo test --test rust_smoke`, `mise run check`, and
`mise run package-check` → all exit 0.

## Test plan

- Follow `src/tmux.rs:205-226` for test-runtime conventions and
  `src/sessions.rs:340-380` for typed fixture construction.
- Include a direct parser unit test and a daemon context lookup test; do not
  rely only on a live tmux server accepting a particular special character.

## Done criteria

- [ ] No production consumer splits a pipe-delimited topology line.
- [ ] Pipes in each relevant metadata field do not shift identity fields.
- [ ] Existing session ordering and target-pane integration tests pass.
- [ ] `mise run check` and `mise run package-check` exit 0.

## STOP conditions

- tmux rejects the proposed safe separator encoding on a supported CI target.
- Changing `Topology` requires an undocumented external protocol migration.
- Existing `sessions --json` output changes beyond field ordering in JSON.

## Maintenance notes

New tmux fields belong in the typed parser and record type first. Reviewers
should reject new raw string formats or positional parsing outside that
boundary.
