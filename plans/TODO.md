# TODO: Harden daemon lifecycle and tmux topology

## Phase 1: Coordinate persisted and cached clears

- [x] Add a locked direct clear and daemon `Clear` IPC transition.
- [x] Refresh daemon state, views, status, and revision after clear.
- [x] Add concurrent/direct and daemon-backed clear regressions.
- [x] Commit: `fix(state): coordinate clear with daemon state`

## Phase 2: Attach a monitor when tmux becomes available

- [x] Carry the invoking tmux server identity through IPC.
- [x] Attach/replace the daemon monitor for a newly supplied server.
- [x] Add an external-start then tmux-client integration regression.
- [x] Commit: `fix(daemon): attach monitor from tmux clients`

## Phase 3: Protect fallback persistence

- [x] Enforce owner-only modes for state directories and event/state files.
- [x] Add daemon-disabled mode assertions and update the state-storage docs.
- [x] Commit: `fix(state): protect fallback event storage`

## Phase 4: Make subscriptions finite and non-blocking

- [x] Release the shared mutex before subscription writes.
- [x] Detect closed/stalled subscribers and terminate their loops.
- [x] Add closed-client and healthy-update regressions.
- [x] Commit: `fix(daemon): reclaim idle subscriptions`

## Phase 5: Type the tmux topology boundary

- [x] Parse direct and daemon tmux output through one typed boundary.
- [x] Remove positional pipe splitting from session views and daemon lookup.
- [x] Add pipe-containing metadata regressions and preserve current ordering.
- [x] Commit: `refactor(tmux): type topology snapshots`

## Verification

- [x] `mise run check` exits 0 after every phase.
- [x] `mise run package-check` exits 0 after every phase.
- [x] `tests/rust_smoke.rs` covers direct and daemon-backed `clear`, an
      outside-tmux daemon later used inside tmux, and normal subscription
      updates.
- [x] Manual smoke test: clear a live daemon, then verify `status`, `sessions`,
      `picker --rows`, and `list --json` all show an empty state.
- [x] Manual smoke test: start a daemon outside tmux, enter tmux, send an event,
      and verify the picker receives the session.
- [x] No behavior change in documented session ordering, pane switching,
      `sessions --json`, or `picker --rows` output.

## Review

- [x] Code reviewed.
- [x] `plans/PLAN.md` updated if approach changed during implementation.
- [x] All phase commits are clean and use the planned messages.
- [x] This TODO and `plans/README.md` statuses are updated.
