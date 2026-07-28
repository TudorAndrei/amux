# Native Runtime Measurements

These are release-build measurements recorded on Darwin arm64 with Rust
1.97.1. They are descriptive rather than CI thresholds: terminal, filesystem,
and tmux-server load all materially affect the numbers.

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

## Session projection cost

The `sessions::views_from` measurement test builds 100 sessions, 200 panes, and
200 records — about three times the observed deployment topology — then projects
all session views in **1.47 ms** in the Rust unit-test build on Darwin arm64
(2026-07-28). This is below the 5 ms threshold, so a session index is rejected
as premature. Run it with:

```bash
cargo test --lib \
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
