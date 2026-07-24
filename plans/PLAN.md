# Plan: Harden daemon lifecycle and tmux topology

## Goal

Make the native runtime reliable across clears, daemon startup outside tmux,
fallback persistence, and long-lived picker subscriptions; then replace the
ambiguous string topology boundary with typed data. The work preserves existing
CLI and JSON behavior while adding regression coverage for the failure modes
identified in the audit.

## Approach

The implementation is split into independently releasable commits. First make
`clear` a serialized daemon state transition, then allow an existing daemon to
attach to the tmux server supplied by a later in-tmux client. The client also
supplies its session, window, and pane so the first event is associated before
the asynchronous monitor takes its initial snapshot. In parallel, harden the
direct write path's permissions. Subscription replies snapshot shared state
before I/O and detect disconnected clients. Finally, parse tmux data once into
typed session and pane records, using the ASCII unit separator and strict field
validation.

No public CLI subcommands, persisted `State` schema, picker row format, agent
classification rules, or new third-party dependencies are in scope.

## Implementation Phases

### Phase 1: Coordinate persisted and cached clears

- Add the daemon clear request and keep direct fallback clears under the same
  state mutation lock as event writes.
- Update cached state/views/status and revision atomically after a daemon clear.
- Add direct and daemon-backed clear regressions.

  **Commit:** `fix(state): coordinate clear with daemon state`

### Phase 2: Attach a monitor when tmux becomes available

- Send the invoking tmux server identity with event/picker IPC.
- Start or replace the daemon's monitor only when its server identity changes.
- Cover a daemon started outside tmux followed by an in-tmux event.

  **Commit:** `fix(daemon): attach monitor from tmux clients`

### Phase 3: Protect fallback persistence

- Apply owner-only directory and file modes on every state write path.
- Add a daemon-disabled permissions regression and document the guarantee.

  **Commit:** `fix(state): protect fallback event storage`

### Phase 4: Make subscriptions finite and non-blocking

- Snapshot shared state before socket writes, apply bounded write behavior, and
  exit subscription loops when clients disconnect.
- Add focused tests for closed and stalled subscribers.

  **Commit:** `fix(daemon): reclaim idle subscriptions`

### Phase 5: Type the tmux topology boundary

- Move tmux parsing into one boundary that returns typed session/pane records.
- Migrate daemon context lookup and session views to the typed snapshot.
- Cover pipe-containing tmux metadata and preserve ordering/target selection.

  **Commit:** `refactor(tmux): type topology snapshots`

## Risks & Tradeoffs

- Monitoring must never create two monitors for one tmux server or retain an
  old monitor after a server restart; track server identity and a stop signal.
- Tightening state permissions may expose misconfigured state directories; use
  an explicit error instead of silently continuing when permissions cannot be
  applied.
- Subscription timeouts must not drop healthy picker updates. Keep the existing
  revision protocol and test that a normal subscriber still receives changes.
- The topology representation changes daemon health JSON internals. Preserve
  CLI JSON contracts and update only tests that intentionally inspect health.

## Open Questions

- None. This plan assumes the default tmux server identity is derived from the
  client's `TMUX` environment, as it is today in `tmux::server_from_env`.
