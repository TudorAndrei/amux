# Implementation Plans

Generated on 2026-07-24 from the `improve` audit at commit `8cf1622`. Execute
these plans in order. Each executor must read its plan completely, honor its
STOP conditions, and update this index only after its commit succeeds.

## Execution order & status

| Plan | Title | Priority | Effort | Depends on | Status |
| ---- | ----- | -------- | ------ | ---------- | ------ |
| 001 | Coordinate clear with daemon state | P1 | M | — | DONE |
| 002 | Attach a tmux monitor to an existing daemon | P1 | M | 001 | DONE |
| 003 | Enforce private fallback permissions | P1 | S | — | DONE |
| 004 | Bound and reclaim daemon subscriptions | P1 | M | 001 | DONE |
| 005 | Use typed tmux topology snapshots | P1 | M | 002 | TODO |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (with one-line reason) |
REJECTED (with one-line rationale).

## Dependency notes

- 002 follows 001 because both extend daemon IPC and state publication; land the
  clear transition before adding monitor-attachment requests.
- 004 follows 001 because it changes the same daemon request/subscriber path.
- 005 follows 002 so monitor attachment and its integration test are stable
  before the topology representation changes.
- 003 is independent and may run in parallel with 001.

## Findings considered and deferred

- Terminal control-character escaping: valuable S-sized security hardening, but
  deferred from this batch to keep behavioral-output changes separately
  reviewable.
- Recoverable state lock, hook-config atomicity/quoting, request-size limits,
  projection indexing, picker error cleanup, stale-duration validation, and
  release-DX alignment: confirmed findings, deferred by the user's selected
  first batch.
