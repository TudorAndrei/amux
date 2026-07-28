# Plan: Fix stuck Codex status and bound the event log

## Goal

A Codex agent that is actively working is rendered as `done` / `Stop` in the
amux picker and tmux status. Codex fires `Stop` once per assistant **turn**, not
once per session, while amux installs only two Codex hooks that can map back to
`running` — `SessionStart` and `UserPromptSubmit` — and both need a human. Commit
`4cc4263 fix(runtime): migrate hooks and mise tasks` deleted the `PostToolUse`
entry from `hooks/codex/hooks.json`, which was the only mid-turn liveness
signal, so since 2026-07-24 a Codex record has been pinned at `done` for the
whole of any turn not started by a hand-typed prompt.

**The regression is in the installed hook set, not in normalization.** Verified
against the released binary with a temp state dir: replaying `UserPromptSubmit →
Stop → PreToolUse → Stop → PreToolUse` already yields `running`/`done` correctly
today, because `normalize_at` (src/event.rs:176) maps any event without a
stop/end/idle/done/complete substring to `running`. amux never sees those events
only because `hooks/codex/hooks.json` does not ask Codex for them. Any regression
test must therefore exercise the **template**, not hand-fed event names. The one
mapping genuinely wrong in normalization is `SessionEnd`, which infers `done`.

This plan re-models the Codex hook set, bounds `events.jsonl` (563 MB,
unrotated), makes stale installs visible through `amux doctor`, and cuts the
per-hook cost so four activity hooks are affordable.

## Approach

### What Codex actually offers

The `HookEventsToml` enum in codex-cli 0.145.0 accepts: `PreToolUse`,
`PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`,
`SessionEnd`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`. There
is **no** `Notification` hook, so `PermissionRequest` stays the only attention
signal for Codex.

Verified from `events.jsonl` for pane `%29`: 15+ `Stop` events between 09:58 and
10:42 UTC on 2026-07-27, same `session_id`, distinct `turn_id` each,
`stop_hook_active: false` throughout, zero `UserPromptSubmit` between. The only
thing that ever unstuck it was incidental — `compact` in the `SessionStart`
matcher.

### Target mapping

| Hook | Status | Note |
| --- | --- | --- |
| `SessionStart` | `running` | matcher narrowed to `startup\|resume\|clear` |
| `UserPromptSubmit` | `running` | unchanged |
| `PreToolUse` | `running` | **new** — holds `running` for the tool's duration |
| `PostToolUse` | `running` | restores `attention` and `PreToolUse` |
| `PermissionRequest` | `attention` | unchanged |
| `PreCompact` | `running` | **new** — compaction is work, not idleness |
| `PostCompact` | `running` | **new** — explicit resume signal |
| `Stop` | `done` | this turn ended; the session is still alive |
| `SessionEnd` | `offline` | **new** — the session is actually over |

`PostToolUse` is not "coverage for long tools" — `PreToolUse` already holds
`running` across a tool's execution. Its real jobs are clearing an `attention`
left by `PermissionRequest` once the approved tool completes (the next
`PreToolUse` may be minutes away), and recovering if a `PreToolUse` is lost. It
is therefore **not** the hook to drop if cost needs cutting; dropping it would
leave `attention` stuck for the whole of an approved long-running tool.

### Accepted limitation: hooks cannot see tool-less work

`Stop → a model-only turn with no tool calls → Stop` emits no hook that maps to
`running`, so amux shows `done` for the duration. There is also always a real
`done` interval between a `Stop` and the next `PreToolUse`. No combination of the
available hooks closes this; only a Codex turn-start hook would. `Stop → done` is
kept and **redefined as "the last observed turn ended"**, not "the session or
task is finished". A debounce before rendering `done` was considered and rejected
for now: it trades a visible-but-honest gap for hidden timing state in the
daemon. Documented in `docs/events.md`, and the verification checklist says
"`running` across tool calls", not "`running` throughout".

### Removing implicit behaviour

1. **`compact` leaves the `SessionStart` matcher.** With `PreCompact` /
   `PostCompact` mapped explicitly it is a duplicate event for the same
   transition.
2. **No inferred status for shipped hooks.** Every Codex hook command carries
   explicit `--status` / `--attention`. Inference stays only for payload-driven
   integrations (opencode, Pi), with a `sessionend` / `session_end` → `offline`
   branch added ahead of the `end` → `done` rule — the one real normalization
   bug.
3. **The `UserPromptSubmit -> running` fixup** in `sessions.rs::views_from` is
   deleted. Both reviews confirm this is close to a no-op: `normalize_at` already
   infers `running` for that event name, and the shipped template has carried
   `--status running` since `71b19e6`. It only ever mattered for hand-edited or
   pre-`71b19e6` flagless installs. The `doctor` drift check lands **before** this
   removal so stale installs are diagnosable when it happens.

`SubagentStart` / `SubagentStop` stay out of scope — `subagent_record` in
`src/sessions.rs` already hides subagent-tagged records.

### Event log retention

The **retention rule** is count-based per record key, as asked: at most
`AMUX_EVENTS_PER_SESSION` (default 200) events kept per `agent:tmux_session:pane`.
The key is amux's identity unit; capping per bare tmux session would let one busy
pane evict another pane's history.

The **trigger** is file size — `metadata().len()` on `events.jsonl`, an O(1)
check, compacting at `AMUX_EVENTS_COMPACT_BYTES` (default 8 MiB). A counter in
`State` was the first design and is rejected: it starts at zero so the existing
563 MB file would not compact for `records.len() × cap` more events; lowering the
cap would not take effect promptly; it leaks an internal maintenance number into
the `list --json` v1 contract; and a crash between append and state write
undercounts it permanently. Size is self-correcting on all four.

A per-key count cap alone cannot bound the file, so compaction also **drops keys
absent from `state.records`**. Those records are already stale-pruned by
`stale_seconds` (src/state.rs:83), so pane-ID churn across tmux server restarts
no longer accumulates forever — the failure mode where one event per new key
means the trigger never fires and dead keys are retained regardless.

**Compaction never runs inside a hook's lock hold.** `state::acquire` gives up
after 500 × 10 ms = 5 s and the hook commands themselves carry `timeout: 5`, so
streaming 563 MB under the lock would make concurrent hooks *fail* with "timed
out waiting for state lock", not merely queue. Instead:

1. Under the lock: rename `events.jsonl` → `events.jsonl.compacting`, release.
   Appends immediately resume into a fresh, empty log.
2. Off-lock: stream the renamed file into per-key ring buffers capped at the
   retention count, carrying each line's original ordinal, dropping dead-key,
   key-less and unparseable lines.
3. Sort the retained set by ordinal and write `events.jsonl.retained`, fsync.
   Sorting by ordinal — not flattening a `BTreeMap` in key order — preserves the
   cross-pane chronology that makes the log useful for debugging.
4. Under the lock briefly: concatenate retained + the small live log into a temp
   file, fsync, rename over `events.jsonl`, fsync the directory, unlink
   `.compacting`.

Compaction runs **only in the daemon** — once at `daemon::run` startup (which
also adopts an orphaned `.compacting` file left by a crash) and on a maintenance
thread when a write trips the trigger, so no connection thread blocks. The
`AMUX_NO_DAEMON` fallback only ever appends.

### Bounding record size for real

The earlier claim that a count cap is sufficient "because `compact_raw` bounds
record size" was wrong: allowlisted strings (`cwd`, `reason`, `source`, IDs) were
unlimited, objects were depth-limited but not breadth-limited, and the outer
`Record` fields sit outside `raw` entirely. `compact_raw` therefore enforces a
real budget: allowlisted strings ≤ 1 KiB, other strings ≤ 256 B, ≤ 32 entries per
object or array, nesting ≤ 3 levels, and a total serialized cap of 4 KiB with
remaining keys dropped once exceeded. `normalize_at` additionally caps
`Record.reason` at 256 B and `Record.cwd` at 1 KiB.

### Out of scope

- `SubagentStart` / `SubagentStop` wiring.
- Any tmux-output-polling or pane-content heuristic for liveness.
- **Daemon/fallback cache coherence.** A direct `AMUX_NO_DAEMON` write persists
  to disk while a running daemon keeps serving its own `Shared.state`, so the
  event can be invisible in the picker until the daemon handles an event of its
  own. Pre-existing, orthogonal, and worth its own change — recorded here so it
  is not mistaken for something this plan introduces.
- Claude, Pi, and opencode hook mappings. Claude's `Stop` genuinely means
  "waiting for you".

## Implementation Phases

### Phase 1: Bound the event log

- `src/config.rs`: add `events_per_session: usize` (`AMUX_EVENTS_PER_SESSION`,
  default 200, `0` disables compaction) and `events_compact_bytes: u64`
  (`AMUX_EVENTS_COMPACT_BYTES`, default 8 MiB).
- `src/event.rs`: add `compact_raw` with the budget above; allowlist
  `session_id`, `cwd`, `hook_event_name`, `source`, `turn_id`, `tool_name`,
  `permission_mode`, `reason` plus the subagent markers `agent_id`,
  `agent_type`, `parent_agent_id`, `parent_session_id`, `is_subagent` that
  `sessions.rs::subagent_record` reads. Call it from `normalize_at`, and cap
  `Record.reason` / `Record.cwd` there too.
- `src/state.rs`: add `compact_events(config, retain_keys)` implementing the
  four-step swap-then-compact above, plus `adopt_orphaned_log(config)` for a
  leftover `.compacting` file.
- `src/daemon.rs`: run `adopt_orphaned_log` then `compact_events` at startup in
  `run` before serving, and spawn a one-at-a-time maintenance thread that
  compacts when a write leaves the log over `events_compact_bytes`.
- Tests: `compact_raw` budget and allowlist; per-key cap; dead-key lines dropped;
  chronological order preserved across keys; malformed and key-less lines
  dropped; `.compacting` adoption after a simulated crash; `0` disables.
  **Commit:** `fix(state): cap retained hook events per session and compact payloads`

### Phase 2: Report hook drift from `amux doctor`

Lands before Phase 3 so the diagnostic exists before compatibility behaviour is
removed.

- `src/hooks.rs`: `pub fn drift()` plus private `drift_at(paths)`, mirroring the
  `install` / `install_at` pattern — `Paths` is private, so the earlier
  `pub fn drift(paths)` signature was not implementable.
- Report per integration: template events missing from the install, amux entries
  for events the template no longer ships, **matcher** differences, and argument
  differences. Compare **only the arguments after `event`**, never the full
  command string: `merge_hooks` deliberately ignores the launcher path
  (src/hooks.rs:155-157) because source, TPM, and release installs live in
  different places, so a full-string compare would flag every non-source install.
- Identify amux-owned entries with the existing `bin/amux event --agent`
  predicate from `remove_matching`, so third-party hooks on the same events are
  never reported.
- Scope: the JSON-merged integrations only (Codex `hooks.json`, Claude
  `settings.json`). Pi and opencode are written as text templates and are not
  covered.
- Wire into `Doctor` in `src/main.rs`: print each drift with
  `amux install-hooks --write` as the remedy; exit code unchanged.
  **Commit:** `feat(doctor): report installed hook drift against the shipped templates`

### Phase 3: Re-model the Codex hook set, with template-level tests

- Rewrite `hooks/codex/hooks.json` to the nine-event mapping, explicit
  `--status` / `--attention` on every command, `SessionStart` matcher narrowed to
  `startup|resume|clear`.
- `src/event.rs`: add the `sessionend` / `session_end` → `offline` branch ahead
  of the `["stop","end","idle","done","complete"] -> done` rule.
- `src/sessions.rs`: delete the `UserPromptSubmit -> running` fixup in
  `views_from`.
- **Template contract test**: parse `hooks/codex/hooks.json` and assert all nine
  events with their exact matchers, `--status` and `--attention` flags. This is
  what actually fails on `main`.
- **Rendered-install test**: `install_at` into a fixture `HOME`, read back
  `~/.codex/hooks.json`, execute the generated command for each event against a
  temp state dir, and assert the resulting record status. Proves a real Codex
  configuration produces an activity event after `Stop` — the thing hand-fed
  event names cannot prove.
- Extend `tests/smoke.sh` with `SessionEnd -> offline`, pinning a **live codex
  pane** so the `offline` comes from the record and not from the no-agent-panes
  branch in `views_from` (src/sessions.rs:185-195), which would make the
  assertion pass on `main` today.
- Before writing the template: confirm against codex-cli whether `PreCompact`,
  `PostCompact`, and `SessionEnd` accept or require a `matcher`. A malformed
  entry that codex silently ignores is exactly the failure this plan exists to
  remove.
  **Commit:** `fix(codex): treat Stop as end-of-turn and track mid-turn activity`

### Phase 4: Cut the per-hook cost

Four activity hooks are only reasonable if a hook is cheap. Today each one spawns
`amux`, then two `tmux display-message` calls in `current_tmux_context`
(src/event.rs:49) **before** any daemon IPC, then `tmux refresh-client` in
`cmd_event` (src/main.rs:175), plus fsyncs of both files. That is roughly two
amux processes and six tmux processes per tool call.

- Send only `TMUX_PANE` on the daemon path and let the daemon resolve session and
  window from the topology it already maintains; shell out to tmux only on the
  direct fallback path.
- Skip the `events.jsonl` append when the incoming event changes none of
  `status` / `attention` / `reason` for that key. Liveness hooks exist for state,
  not for history; this cuts steady-state growth by roughly the tool-call rate
  while keeping every transition.
- Coalesce `tmux refresh-client` in the daemon rather than calling it per hook.
- Benchmark a tool-heavy Codex turn before and after; record the numbers.
  **Commit:** `perf(event): resolve tmux context in the daemon and log only transitions`

### Phase 5: Documentation

- `docs/events.md`: the nine-event Codex mapping, `Stop` as turn-scoped, the
  absent `Notification` hook, no inferred status for shipped hooks, and the
  tool-less-turn limitation stated plainly.
- `README.md`: `AMUX_EVENTS_PER_SESSION` and `AMUX_EVENTS_COMPACT_BYTES`, per-key
  retention with dead-key expiry, payload budget, `offline` in the status table,
  the `doctor` drift check, and the removal of the `SessionStart` `compact`
  matcher.
- Upgrade note: `amux install-hooks --write` must be re-run; `amux doctor` says
  when it is needed.
  **Commit:** `docs(codex): document the turn-scoped Stop mapping and event retention`

## Risks & Tradeoffs

- **Last-write-wins across racing hooks.** `state.records.insert` is
  unconditional and `updated_at` is second-granularity, so a slow `PostToolUse`
  that loses the lock race to a `Stop` can leave a record `running` after the
  turn ended. Pre-existing, but this plan multiplies the number of racing writers
  per turn. It self-heals on the next event, so no redesign — but Phase 3's tests
  must not depend on sub-second ordering.
- **A `done` gap is unavoidable.** See the accepted limitation above. Users will
  still occasionally see `done` on a working agent during a tool-less turn. This
  plan shrinks the window from "the entire session" to "between tool calls"; it
  does not close it.
- **Compaction is destructive.** No archived copy — events beyond the cap and all
  dead-key events are gone. That is the point of a count cap, but debugging
  history is lost.
- **Dead-key expiry is coupled to `stale_seconds`.** Lowering
  `AMUX_STALE_SECONDS` silently shortens event retention too, since compaction
  keeps only keys still in `state.records`.
- **The one-time 563 MB compaction still costs.** The swap makes it safe for
  concurrent hooks, but the daemon does real I/O at startup. `amux clear`
  beforehand avoids it.
- **Phase 4 changes the meaning of `events.jsonl`.** Transition-only logging
  makes it a status-change log, not a raw hook log. Worth it for the volume, but
  it is a documented semantic change, not an optimization users won't notice.

## Open Questions

- Resolved: retention is count-based per record key,
  `AMUX_EVENTS_PER_SESSION=200`, `0` disables. The size-based *trigger* is an
  implementation detail beneath that rule.
- Resolved: `amux doctor` reports hook drift — now Phase 2, ahead of the
  compatibility removal.
- Resolved: no reliance on implicit behaviour — `PreCompact` / `PostCompact`
  mapped explicitly, `compact` out of the `SessionStart` matcher, `views_from`
  fixup deleted.
- Resolved: **Phase 4 is in scope.** The daemon now resolves pane context from
  its topology, coalesces refreshes, and records transitions only. This makes
  the higher-frequency Codex activity hooks affordable.
- Resolved: **do not debounce `Stop → done`.** The brief, honest `done` interval
  between observed tool calls remains visible; a grace period would add hidden
  timing state and conceal the known tool-less-turn limitation.
