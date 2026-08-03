# Plan: Deepen and harden amux

## Goal

Resolve every finding from the architecture and codebase review at commit
`810a206`, deepen the five identified modules, and deliver the three grounded
product directions without regressing amux's tmux, hook, state-v1, packaging,
or TPM behavior. The work must remain incremental: every phase lands as a
separately verified conventional commit, and no phase leaves the daemon,
fallback writer, installed hooks, or release archives in a half-migrated state.

## Approach

Start by making the library the only Rust module graph and moving command
dispatch behind a small crate interface. This removes the duplicate 41-test
pass and creates a stable place for subsequent changes. Preserve the existing
CLI and serialized state-v1 behavior while doing so; the refactor must not
change command output, hook mappings, state paths, or the canonical TPM
launcher path at `~/.tmux/plugins/amux/bin/amux`.

Land the small durability fixes next, then make daemon ownership and maintenance
explicit. Daemon startup gets a kernel-released startup claim around stale
socket inspection, removal, and bind. The current `Shared` data bag becomes a
deep live-model module whose implementation owns the invariant
`state + topology -> views + revision`. Background compaction becomes tracked
maintenance rather than a detached best-effort thread: `clear` coordinates
with it, failures are recovered or reported while the daemon remains alive,
and deterministic test barriers cover the handoff windows.

Deepen IPC before event intake. The IPC module will own newline framing,
one-shot response validation, stream timeouts, and the distinct long-lived
subscription lifecycle. Its error type must distinguish daemon unavailability
from a daemon rejection so rejected input can never bypass validation through
the direct fallback.

Create a durable event-intake module around the current
`event::NormalizeInput`, `ipc::HookRequest`, record-to-log conversion, and
`state::write_event` sequence. It will enforce a single input-size contract,
cap every derived identifier, retain only an explicit metadata allowlist, and
produce the same durable result for daemon and daemon-less callers. The
filesystem remains a local-substitutable dependency tested with isolated state
directories; internal seams used for fault injection stay private.

Move agent lifecycle classification out of hook arguments and
`event::normalize_at` into one in-process policy module. Codex, Claude, Pi,
and opencode remain adapters at a real seam because all four provide different
external event shapes. The installed templates retain only transport-specific
matchers and commands. Tests at the lifecycle interface become authoritative;
rendered-hook smoke tests continue proving each adapter end to end.

After those foundations, complete hook drift reporting, harden CI and release
gating, establish license/notices automation, and add the event-history and
live-watch commands. Pi/opencode fidelity starts with a documented upstream
capability matrix; implementation proceeds only for signals confirmed stable
by primary upstream documentation or fixtures.

The following maps the complete review into phases:

| Review item | Covered by |
| --- | --- |
| Clear/compaction race | Phases 4–5 |
| Immutable workflow action pins | Phase 10 |
| Collision-safe hook backups | Phase 2 |
| Directory sync after state mutations | Phase 2 |
| Concurrent stale-socket recovery | Phase 3 |
| Bounded event-input contract | Phases 6–7 |
| Live compaction recovery/reporting | Phases 4–5 |
| Pi/opencode drift visibility | Phase 9 |
| No-op release packaging | Phase 10 |
| Duplicate crate module graph | Phase 1 |
| Untested `next-attention` behavior | Phase 1 |
| Raw hook-data minimization | Phase 7 |
| Project license and complete notices | Phase 11 |
| Durable event-intake module | Phase 7 |
| Agent lifecycle policy module | Phase 8 |
| Daemon live-model module | Phase 4 |
| Deep IPC transport module | Phase 6 |
| Canonical crate module graph | Phase 1 |
| Event-history command | Phase 12 |
| Headless live stream | Phase 13 |
| Pi/opencode lifecycle fidelity | Phases 14–15 |

Dependencies are strict: Phase 1 precedes all Rust work; Phase 4 precedes
maintenance repair; Phase 6 precedes event intake and live watch; Phase 7
precedes exposing history; Phase 8 precedes integration fidelity. Phase 11
cannot start until the maintainer chooses amux's license. Phase 15 is
conditional on the capability matrix from Phase 14.

Explicitly out of scope:

- Reintroducing the removed status-right or polling status integration.
- Indexing session projection; the measured cost remains below the documented
  threshold in `docs/performance.md`.
- Treating the canonical TPM launcher as drift.
- Windows support; the current runtime is intentionally Unix/tmux based.
- Changing the on-disk state version or silently migrating incompatible data.
- Installing speculative ports where only one adapter exists.

## Implementation Phases

### Phase 1: Establish one crate graph and test next-attention

- Move clap types, command dispatch, and the current helpers from
  `src/main.rs` into a library-owned CLI module. Reduce `src/main.rs` to the
  small crate interface that runs the command and returns its exit status.
- Declare `hooks` in the library graph and stop redeclaring `config`,
  `daemon`, `event`, `fsutil`, `ipc`, `model`, `render`,
  `sessions`, `state`, `tmux`, and `ui` in the binary.
- Keep the current library exports temporarily only where compilation or
  downstream compatibility requires them; document any intentionally removed
  incidental export before committing.
- Move the pure newest-live-attention selection rule from
  `cmd_next_attention` into `src/sessions.rs`, then add focused tests for
  newest selection, offline exclusion, and no target.
- Extend the isolated tmux coverage in `tests/rust_smoke.rs` so
  `next-attention` is executed and switches to the expected pane rather than
  merely appearing in `list-keys`.
- Verify `cargo test --all-features -- --list` reports shared module tests
  once, while all existing CLI and TPM smoke behavior remains unchanged.
  **Commit:** `refactor(cli): make the library the canonical module graph`

### Phase 2: Make state mutations and hook backups durable

- Call `fsutil::sync_dir` after the `state.json` rename in
  `state::write_event` and after `state::clear` removes any state or event
  files.
- Change `hooks::backup` to reserve a unique destination without overwriting
  an existing same-second backup. Keep timestamp readability and retry with a
  collision suffix using non-destructive creation.
- Add tests proving two immediate writes preserve the original and intermediate
  hook configurations as distinct backups.
- Add focused state tests for the directory-sync call sites using the existing
  atomic-write convention; do not weaken the current file `sync_all` or event
  log `sync_data` behavior.
  **Commit:** `fix(storage): preserve durable state and hook backups`

### Phase 3: Give daemon startup one owner

- Add a daemon-startup claim path to `Config` beside `amux.sock`; implement
  it with a fresh advisory-locked file handle whose drop releases the claim.
- Hold that claim across socket liveness probing, stale-socket inode
  validation/removal, `UnixListener::bind`, and socket permission setup.
- Recheck liveness after acquiring the claim so a waiter exits cleanly when the
  first starter has already bound.
- Add a two-process regression test beginning with a stale socket and assert
  exactly one reachable daemon remains, the losing starter does not unlink the
  live socket, and shutdown removes only the owned socket.
  **Commit:** `fix(daemon): serialize stale socket recovery`

### Phase 4: Deepen the daemon live model

- Replace field-by-field mutation of `Shared.state`, `Shared.topology`,
  `Shared.views`, and `Shared.revision` with methods that apply an event
  state, clear state, apply topology, and produce a response snapshot.
- Keep monitor identity, shutdown, and maintenance coordination inside the
  daemon implementation without exposing them through the external interface.
- Make every mutation recompute views and increment revision in exactly one
  place; preserve the current rule that unchanged topology does not publish a
  revision.
- Add in-process tests at the live-model interface for event, clear, changed
  topology, unchanged topology, and response snapshot coherence.
- Keep request parsing and socket I/O outside the live-model module.
  **Commit:** `refactor(daemon): concentrate live model invariants`

### Phase 5: Coordinate and recover event-log maintenance

- Replace `maintenance_active: Arc<AtomicBool>` and the detached
  `schedule_compaction` result discard with a tracked maintenance coordinator
  that serializes compaction scheduling and clear.
- Ensure `Request::Clear` cannot return while pre-clear retained lines can
  still be installed. A compaction scheduled from an event that completed
  before clear must either finish before clear or observe cancellation and
  decline its final install.
- On compaction failure after the live-log rename, recover the orphan while the
  daemon remains alive or retain an explicit diagnostic that `doctor` can
  report. A later event must be able to compact successfully without restart.
- Keep event order intact when adopting `.compacting`, retained lines, and
  events appended during the unlocked streaming pass.
- Add an internal deterministic test seam around the post-rename/pre-install
  handoff. Use it to prove clear wins that race and to inject failures at each
  fallible stage after rename.
- Extend `tests/rust_smoke.rs` to prove cleared state and history stay absent
  after maintenance settles.
  **Commit:** `fix(state): coordinate and recover event log maintenance`

### Phase 6: Deepen IPC transport and error semantics

- Move socket-path ownership, one-shot request framing, response decoding, and
  response-error validation from `src/daemon.rs` into `src/ipc.rs`.
- Introduce a typed client error that distinguishes connection/unavailability
  from a valid daemon rejection. Update event fallback logic to retry or write
  directly only for unavailability.
- Preserve the distinct shutdown semantics: an absent daemon is success, but a
  connected daemon response must still be framed and validated.
- Preserve subscription as a long-lived lifecycle inside the IPC
  implementation; expose revision and session views rather than discarding the
  revision in the client thread.
- Add read/write timeouts for one-shot clients that fit inside the existing
  five-second external hook budget without imposing those timeouts on a live
  subscription.
- Consolidate socket-pair tests for oversized requests, complete large
  responses, disconnects, malformed responses, daemon rejection, and timeout.
  **Commit:** `refactor(ipc): own framing and response validation`

### Phase 7: Build one bounded durable event-intake module

- Add a dedicated Rust module that owns bounded stdin parsing, extraction of
  lifecycle metadata, normalization, identifier caps, retained-raw projection,
  record/event-log construction, and the call to `state::write_event`.
- Replace unbounded `read_to_string` in the event command with a reader that
  rejects input over the chosen limit before allocating or parsing the full
  document. Keep empty stdin equivalent to `{}`.
- Use an explicit retained metadata allowlist containing only fields required
  by lifecycle classification, session identity, cwd, and subagent detection.
  Drop tool input, messages, command text, and unknown fields before either
  `state.json` or `events.jsonl` is written.
- Cap agent, event, agent-session, tmux, cwd, reason, and persisted-key
  contributions before key construction. Preserve UTF-8 boundaries.
- Route daemon and daemon-less callers through the same durable intake
  interface; remove their duplicate record-to-log conversion.
- Ensure a daemon rejection never invokes direct fallback, while transport
  unavailability still does.
- Add equivalence tests for both paths plus oversized input, oversized derived
  identifiers, sensitive unknown fields, required subagent metadata, and
  malformed JSON.
  **Commit:** `fix(event): enforce one bounded durable intake path`

### Phase 8: Concentrate lifecycle policy

- Add an in-process lifecycle policy module that classifies status, attention,
  reason defaults, and terminal behavior from agent, event name, and the
  already-extracted metadata.
- Move Claude notification classification and generic event-name inference out
  of `event::normalize_at`; encode Codex's current explicit mapping in the
  same policy.
- Reduce Codex and Claude template commands to adapter concerns: event
  subscription, matcher, launcher, timeout, and stable human reasons where
  needed. Do not duplicate status policy in command arguments.
- Represent unknown agent events conservatively and retain explicit CLI
  overrides with their current precedence.
- Replace policy-specific tests scattered through `src/event.rs` with tests
  at the lifecycle interface, while retaining rendered Codex/Claude end-to-end
  scenarios in `tests/rust_smoke.rs`.
- Update `docs/events.md` and `README.md` from the resulting behavior.
  **Commit:** `refactor(events): centralize agent lifecycle policy`

### Phase 9: Report drift for every installed integration

- Extend `hooks::drift_at` to inspect rendered Pi and opencode text assets as
  well as Codex and Claude JSON documents.
- Normalize only the rendered launcher assignment when comparing text assets;
  do not treat the canonical TPM checkout path or another valid absolute
  checkout as drift.
- Check that Pi settings still reference the installed extension path.
- Change `doctor` output from the global “installed templates match” claim to
  explicit per-integration results or an honest “unverified” state.
- Add tests for stale text, launcher-only differences, missing Pi registration,
  foreign user edits outside the amux-owned text, and all-four-current output.
- Update `docs/updating.md` with the complete drift contract.
  **Commit:** `fix(doctor): verify every installed agent integration`

### Phase 10: Harden workflow dependencies and release gating

- Replace every GitHub Actions tag in `.github/workflows/ci.yml` with the
  current verified full commit SHA and retain the human-readable release tag
  in a comment.
- Add `.github/dependabot.yml` for reviewable GitHub Actions updates; include
  Cargo updates only if the maintainer wants one shared update policy.
- Record the pre-bump commit in `prepare-release`, emit a boolean indicating
  whether Cocogitto created a new release commit/tag, and gate the push,
  `package-release`, and `publish-release` work on it.
- Keep the three-platform `check` matrix on every push. A docs-only or
  non-releasing commit may skip release packaging but must never skip checks.
- Run `actionlint` through `hk check`, inspect all `uses:` entries for
  immutable pins, and verify both releasing and non-releasing conditions.
  **Commit:** `ci(release): pin actions and skip no-op packaging`

### Phase 11: Define licensing and generate complete notices

- Stop before this phase until the maintainer selects amux's project license.
- Add the chosen root license file and declare the matching SPDX expression or
  `license-file` in `Cargo.toml`; do not infer the choice from dependencies.
- Add a reproducible Cargo license/notices tool to `mise.toml` with checked
  policy/configuration committed to the repository.
- Regenerate `THIRD_PARTY_NOTICES.md` from `Cargo.lock` so direct and
  transitive compiled dependencies are covered, including Nucleo's MPL terms.
- Add a check-mode hk step that fails when notices or policy drift.
- Verify the generated notices ship in every archive through
  `mise run package-check`.
  **Commit:** `docs(license): define terms and generate dependency notices`

### Phase 12: Add event-history inspection

- Add an `events` CLI command beside `list` and `sessions`.
- Add a state reader that streams valid retained JSONL records, skips malformed
  lines using the same compaction tolerance, and supports agent, session, pane,
  and bounded-limit filters while preserving chronological order.
- Define and document stable plain-text and JSON output. JSON may expose only
  the minimized metadata established in Phase 7; the default text output must
  pass every field through `render::sanitize`.
- Add CLI tests for empty history, filters, chronological limits, malformed
  lines, disabled retention, and control characters.
- Document privacy and retention behavior in `README.md` and
  `docs/events.md`.
  **Commit:** `feat(cli): expose retained event history`

### Phase 13: Add a headless live JSON stream

- Add a `watch --json` CLI command that consumes the deep IPC subscription
  and writes one newline-delimited object per revision containing revision and
  session views.
- Reuse the same subscription interface in `src/ui.rs`; keep UI-specific
  channel reduction inside the UI implementation.
- Start or connect to the daemon using the existing bounded startup behavior.
  After a connected stream fails, exit nonzero with a diagnostic instead of
  silently polling internal files.
- Document framing, initial snapshot behavior, revision monotonicity,
  disconnect behavior, and stdout/stderr separation.
- Add socket-pair and process-level tests for initial output, multiple
  revisions, large snapshots, slow consumers within the current write timeout,
  disconnect, and clean Ctrl-C termination.
  **Commit:** `feat(cli): stream live session revisions as json`

### Phase 14: Record Pi and opencode lifecycle capabilities

- Read current primary upstream documentation and inspect representative
  fixtures for Pi and opencode lifecycle hooks.
- Add a capability matrix to `docs/events.md` covering start, activity,
  permission/input attention, completion/idle, and session end for all four
  adapters.
- Record exact unsupported or unstable signals instead of inventing inferred
  timers or polling.
- Decide which confirmed signals proceed to Phase 15; if neither upstream
  exposes enough stable information, stop there and keep the limitation
  explicit.
  **Commit:** `docs(integrations): record lifecycle capability matrix`

### Phase 15: Improve supported Pi and opencode lifecycle fidelity

- Extend `hooks/pi/amux.ts` and `hooks/opencode/amux.js` only for stable
  signals confirmed in Phase 14.
- Route each signal through the lifecycle policy module rather than embedding
  status rules in TypeScript or JavaScript.
- Add representative fixtures under `tests/fixtures/`, rendered-install
  checks, drift checks, and daemon/daemon-less lifecycle scenarios.
- Preserve conservative behavior for unknown future signals and document every
  remaining asymmetry.
  **Commit:** `feat(integrations): improve pi and opencode lifecycle fidelity`

## Risks & Tradeoffs

- The roadmap is intentionally large. The phase order and per-phase commits are
  the mitigation; do not combine phases into a single implementation change.
- Turning the implicit library target into a small interface could affect
  unknown Rust consumers. Confirm whether that surface is supported before
  removing public exports; otherwise retain deprecated compatibility exports
  temporarily and record their removal separately.
- Directory fsync adds latency to direct writes. Measure the existing
  `docs/performance.md` hook path after Phase 2 and reject batching that would
  weaken durability.
- Maintenance coordination can deadlock if a clear holds the live-model mutex
  while joining work that publishes through it. Join or cancel outside that
  mutex and prove the lock order in tests.
- Input minimization may remove metadata later needed by an adapter. The
  allowlist must be based on all four current fixtures, and unknown fields must
  not be retained merely “just in case.”
- Centralizing lifecycle policy changes rendered hook arguments, so users must
  rerun `install-hooks --write`; `doctor` and update documentation must make
  this explicit.
- Full action SHAs improve supply-chain control but require automation to avoid
  aging unnoticed.
- Event-history and watch outputs become public contracts. Version and document
  their JSON shapes before release.
- Legal obligations vary by dependency and jurisdiction. Tool-generated
  notices support review but do not replace the maintainer's license choice or
  legal advice.
- Pi/opencode upstream capabilities may not support the desired fidelity.
  Phase 15 must not manufacture unreliable attention states.

## Open Questions

- Which license should amux use? Recommendation: choose before Phase 11; no
  executor should guess.
- Is the current undocumented Rust library surface supported externally?
  Recommendation: treat it as internal and expose only the CLI entry interface,
  unless evidence of consumers exists.
- Should Cargo dependency updates join the GitHub Actions Dependabot policy in
  Phase 10? Recommendation: enable grouped, scheduled Rust updates separately
  from action pins.
- Should `events --json` be a JSON array or NDJSON? Recommendation: use a JSON
  array for bounded history and reserve NDJSON for unbounded `watch --json`.
- Which Pi/opencode signals are stable enough for Phase 15? Resolve from the
  Phase 14 capability matrix, not from inference.
