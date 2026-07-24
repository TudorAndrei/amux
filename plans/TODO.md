# TODO: Harden daemon lifecycle and tmux topology

## Phase 1: Coordinate persisted and cached clears

- [x] Add a locked direct clear and daemon `Clear` IPC transition.
- [x] Refresh daemon state, views, status, and revision after clear.
- [x] Add concurrent/direct and daemon-backed clear regressions.
- [x] Commit: `fix(state): coordinate clear with daemon state`

## Phase 2: Attach a monitor when tmux becomes available

- [ ] Carry the invoking tmux server identity through IPC.
- [ ] Attach/replace the daemon monitor for a newly supplied server.
- [ ] Add an external-start then tmux-client integration regression.
- [ ] Commit: `fix(daemon): attach monitor from tmux clients`

## Phase 3: Protect fallback persistence

- [ ] Enforce owner-only modes for state directories and event/state files.
- [ ] Add daemon-disabled mode assertions and update the state-storage docs.
- [ ] Commit: `fix(state): protect fallback event storage`

## Phase 4: Make subscriptions finite and non-blocking

- [ ] Release the shared mutex before subscription writes.
- [ ] Detect closed/stalled subscribers and terminate their loops.
- [ ] Add closed-client and healthy-update regressions.
- [ ] Commit: `fix(daemon): reclaim idle subscriptions`

## Phase 5: Type the tmux topology boundary

- [ ] Parse direct and daemon tmux output through one typed boundary.
- [ ] Remove positional pipe splitting from session views and daemon lookup.
- [ ] Add pipe-containing metadata regressions and preserve current ordering.
- [ ] Commit: `refactor(tmux): type topology snapshots`

## Verification

- [ ] `mise run check` exits 0 after every phase.
- [ ] `mise run package-check` exits 0 after every phase.
- [ ] `tests/rust_smoke.rs` covers direct and daemon-backed `clear`, an
      outside-tmux daemon later used inside tmux, and normal subscription
      updates.
- [ ] Manual smoke test: clear a live daemon, then verify `status`, `sessions`,
      `picker --rows`, and `list --json` all show an empty state.
- [ ] Manual smoke test: start a daemon outside tmux, enter tmux, send an event,
      and verify the picker receives the session.
- [ ] No behavior change in documented session ordering, pane switching,
      `sessions --json`, or `picker --rows` output.

## Review

- [ ] Code reviewed.
- [ ] `plans/PLAN.md` updated if approach changed during implementation.
- [ ] All phase commits are clean and use the planned messages.
- [ ] This TODO and `plans/README.md` statuses are updated.
