# Native Runtime Measurements

Unless a section says otherwise, these are release-build measurements recorded
on Darwin arm64 with Rust 1.97.1. They are descriptive rather than CI
thresholds: terminal, filesystem, and tmux-server load all materially affect
the numbers.

## Direct hook persistence

Thirty `PostToolUse` events were sent with a warm state directory and daemon
startup disabled, so both implementations exercised their durable direct-write
path. The historical shell was checked out at `d082e7c`; the native binary was
the 1,089,984-byte release build.

| Implementation | p50 | p95 | Max |
| --- | ---: | ---: | ---: |
| Rust | 18.2 ms | 19.5 ms | 19.8 ms |
| Historical shell + jq | 45.2 ms | 47.7 ms | 48.5 ms |

The Rust median was 60% lower for this path. With a warm daemon, thirty event
submissions reached its cached state in 22.8 ms p50 and 25.4 ms p95, including
the short-lived hook client process and socket request.

## Durable daemon event transaction

This sample used the release binary on Darwin arm64, an isolated state
directory, one 340-byte state record, no tmux server, one warm-up event, and
thirty warm daemon events. The sandbox makes file synchronization slow, so the
results are useful only as a before-and-after comparison from the same host and
method.

| Revision | State read and parse calls | Mean | p50 | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before (`92c043a`) | 2 | 364.472 ms | 364.308 ms | 375.652 ms | 383.007 ms |
| Durable transaction | 1 | 366.574 ms | 367.193 ms | 374.359 ms | 375.260 ms |

The state read counts follow the complete daemon event call path. Before the
change, the durable commit loaded state and the daemon loaded it again before
publication. The durable transaction returns its committed state, so the
daemon does not do the second load. The latency samples overlap and do not show
a latency improvement. This change makes no performance claim.

## Topology, idle work, and input responsiveness

- A tmux notification is coalesced for 20 ms, then reconciled with exactly one
  flat `list-sessions` and one `list-panes -a` request on the persistent control
  connection. The isolated tmux integration test covers create, rename, link,
  close, server restart, and rebuild of this snapshot.
- The daemon has no one-second model reload. It reacts to tmux control events
  and performs a safety reconciliation every 30 seconds. The old fzf picker
  re-executed its shell, `jq`, and `awk` rendering pipeline every second when
  live reload was available.
- Picker input and daemon updates use separate tasks. The reducer test delays a
  refresh for 200 ms and verifies that navigation completes in under 100 ms;
  query changes also retain the first-match and passive-selection contracts.

The figures above compare the former shell path with equivalent native work.
They intentionally do not treat tmux's own status redraw interval or terminal
paint timing as an amux latency measurement.

## Live subscription data

The `live_model::measures_idle_clones_and_subscription_bytes` test uses 100
sessions, 200 panes, and 200 records. Twenty checks represent one idle second at
the 50 ms subscription interval. Byte counts use the exact newline-delimited
JSON encoding and do not depend on compiler optimization.

| Shape | State clones | Topology clones | View clones | Snapshot B | Wire B |
| --- | ---: | ---: | ---: | ---: | ---: |
| Legacy state + topology + views | 20 | 20 | 20 | 131,206 | 131,251 |
| Revision + views | 0 | 0 | 0 | 61,011 | 61,035 |

An unchanged check now copies no model data. A changed revision sends 70,216
fewer bytes for this workload, a measured 53.5% reduction. Run the measurement
with:

```bash
mise run test-container -- cargo test --lib \
  measures_idle_clones_and_subscription_bytes -- --nocapture
```

## Session projection cost

The `sessions::views_from` measurement test builds 100 sessions, 200 panes, and
200 records — about three times the observed deployment topology — then projects
all session views in **1.47 ms** in the Rust unit-test build on Darwin arm64
(2026-07-28). This is below the 5 ms threshold, so a session index is rejected
as premature. Run it with:

```bash
mise run test-container -- cargo test --lib \
  projection_at_100_sessions_200_panes_200_records_is_measured -- --nocapture
```

## Codex activity-hook cost

This measured the pre-change native commit `81ad313` against the current
release binary on the same Darwin arm64 host. Each run used a warm daemon,
an isolated tmux server, one warm-up `PreToolUse`, and fifty `PostToolUse`
submissions for the same pane.

| Version | 50 hooks | Per hook | `events.jsonl` bytes | Log lines |
| --- | ---: | ---: | ---: | ---: |
| Before | 1.25 s | 25.0 ms | 18,154 | 51 |
| After | 0.56 s | 11.2 ms | 710 | 2 |

Before this change each hook client spawned two `tmux display-message`
lookups plus `tmux refresh-client`; the daemon path now sends only `TMUX_PANE`,
uses its persistent topology snapshot, and coalesces refreshes. The remaining
two log lines are the real status transitions (`PreToolUse → running` and
`PostToolUse → running` after a reason change); repeated equivalent activity
is deliberately not history.
