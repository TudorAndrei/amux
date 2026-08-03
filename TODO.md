# TODO: Deepen and harden amux

## Phase 1: Establish one crate graph and test next-attention

- [x] Move CLI dispatch from `src/main.rs` into the library-owned CLI module.
- [x] Make `src/main.rs` call the small crate interface.
- [x] Declare shared modules, including `hooks`, only in the library graph.
- [x] Add pure newest-live-attention tests in `src/sessions.rs`.
- [x] Execute `next-attention` against an isolated tmux server in
  `tests/rust_smoke.rs`.
- [x] Confirm shared module test names appear once in
  `cargo test --all-features -- --list`.
- [x] Commit: `refactor(cli): make the library the canonical module graph`

## Phase 2: Make state mutations and hook backups durable

- [x] Sync the state directory after `state.json` rename and clear deletions.
- [x] Make `hooks::backup` reserve collision-safe, non-overwriting paths.
- [x] Test two immediate hook updates preserve both rollback snapshots.
- [x] Retain existing file and event-log sync behavior.
- [x] Commit: `fix(storage): preserve durable state and hook backups`

## Phase 3: Give daemon startup one owner

- [x] Add the daemon-startup claim path to `Config`.
- [x] Hold an advisory claim across stale-socket recovery and bind.
- [x] Recheck daemon liveness after claim acquisition.
- [x] Add a two-starter stale-socket regression test.
- [x] Verify one reachable daemon and no unlinked live listener remain.
- [x] Commit: `fix(daemon): serialize stale socket recovery`

## Phase 4: Deepen the daemon live model

- [ ] Put state, topology, views, and revision mutation behind live-model
  methods.
- [ ] Centralize view recomputation and revision publication.
- [ ] Keep monitor, shutdown, and maintenance facts internal.
- [ ] Test event, clear, changed/unchanged topology, and coherent snapshots.
- [ ] Verify no CLI or serialized-state behavior changes.
- [ ] Commit: `refactor(daemon): concentrate live model invariants`

## Phase 5: Coordinate and recover event-log maintenance

- [ ] Replace detached compaction state with tracked maintenance coordination.
- [ ] Prevent pre-clear retained lines from installing after clear.
- [ ] Recover or report post-rename compaction failures without restart.
- [ ] Preserve chronology across orphan, retained, and newly appended events.
- [ ] Add deterministic barriers for clear/compaction and failure injection.
- [ ] Prove cleared state and event history remain absent after maintenance.
- [ ] Commit: `fix(state): coordinate and recover event log maintenance`

## Phase 6: Deepen IPC transport and error semantics

- [ ] Move socket path, framing, decoding, and validation into `src/ipc.rs`.
- [ ] Distinguish daemon unavailability from daemon rejection.
- [ ] Preserve absent-daemon shutdown success and validate connected responses.
- [ ] Return revision plus views from the subscription interface.
- [ ] Add bounded one-shot timeouts without timing out subscriptions.
- [ ] Consolidate malformed, oversized, large-response, disconnect, rejection,
  and timeout socket tests.
- [ ] Commit: `refactor(ipc): own framing and response validation`

## Phase 7: Build one bounded durable event-intake module

- [ ] Add one module for bounded parsing, normalization, retention, and
  persistence.
- [ ] Reject oversized stdin before full allocation or JSON parsing.
- [ ] Project retained raw data through an explicit metadata allowlist.
- [ ] Cap every persisted identifier and key contribution on UTF-8 boundaries.
- [ ] Route daemon and daemon-less writes through the same intake interface.
- [ ] Prevent daemon rejections from falling through to direct persistence.
- [ ] Test path equivalence, oversized input/identifiers, sensitive unknown
  fields, subagent metadata, empty stdin, and malformed JSON.
- [ ] Commit: `fix(event): enforce one bounded durable intake path`

## Phase 8: Concentrate lifecycle policy

- [ ] Add the in-process lifecycle policy module.
- [ ] Move Claude, Codex, and generic classification out of
  `event::normalize_at`.
- [ ] Remove duplicated status policy from hook command arguments.
- [ ] Preserve explicit override precedence and conservative unknown handling.
- [ ] Move policy assertions to lifecycle-interface tests.
- [ ] Keep rendered Codex/Claude adapter smoke coverage.
- [ ] Update `README.md` and `docs/events.md`.
- [ ] Commit: `refactor(events): centralize agent lifecycle policy`

## Phase 9: Report drift for every installed integration

- [ ] Compare rendered Pi and opencode text assets in `hooks::drift_at`.
- [ ] Normalize launcher assignments without treating TPM paths as drift.
- [ ] Verify Pi settings registration.
- [ ] Print honest per-integration doctor results.
- [ ] Test stale text, launcher-only differences, missing Pi registration, and
  all-four-current output.
- [ ] Update `docs/updating.md`.
- [ ] Commit: `fix(doctor): verify every installed agent integration`

## Phase 10: Harden workflow dependencies and release gating

- [ ] Pin every `uses:` entry to a verified full commit SHA with tag comments.
- [ ] Add scheduled GitHub Actions updates in `.github/dependabot.yml`.
- [ ] Emit whether Cocogitto created a new release commit/tag.
- [ ] Gate release push, packaging, and publication on that result.
- [ ] Keep all three check jobs on every push.
- [ ] Verify actionlint, immutable pins, releasing flow, and docs-only flow.
- [ ] Commit: `ci(release): pin actions and skip no-op packaging`

## Phase 11: Define licensing and generate complete notices

- [ ] Obtain the maintainer's explicit project-license choice.
- [ ] Add the root license and matching Cargo metadata.
- [ ] Add a reproducible license/notices tool and checked policy.
- [ ] Regenerate `THIRD_PARTY_NOTICES.md` from `Cargo.lock`.
- [ ] Add an hk drift/policy check.
- [ ] Verify packaged archives contain current license and notices.
- [ ] Commit:
  `docs(license): define terms and generate dependency notices`

## Phase 12: Add event-history inspection

- [ ] Add the `events` CLI command.
- [ ] Stream valid retained JSONL with agent/session/pane/limit filters.
- [ ] Define chronological plain-text and bounded JSON output.
- [ ] Sanitize every default text field.
- [ ] Test empty history, filters, limits, malformed lines, disabled retention,
  and terminal controls.
- [ ] Document history privacy and retention.
- [ ] Commit: `feat(cli): expose retained event history`

## Phase 13: Add a headless live JSON stream

- [ ] Add `watch --json` with one NDJSON object per revision.
- [ ] Reuse the IPC subscription in `src/ui.rs`.
- [ ] Define startup, disconnect, and stderr behavior.
- [ ] Document stream framing and revision monotonicity.
- [ ] Test initial output, revisions, large snapshots, slow consumers,
  disconnect, and Ctrl-C.
- [ ] Commit: `feat(cli): stream live session revisions as json`

## Phase 14: Record Pi and opencode lifecycle capabilities

- [ ] Read current primary Pi and opencode hook documentation.
- [ ] Capture representative lifecycle fixtures.
- [ ] Add the four-adapter capability matrix to `docs/events.md`.
- [ ] Record unsupported/unstable signals without inferred timers.
- [ ] Decide whether confirmed signals unblock Phase 15.
- [ ] Commit: `docs(integrations): record lifecycle capability matrix`

## Phase 15: Improve supported Pi and opencode lifecycle fidelity

- [ ] Extend only signals confirmed stable in Phase 14.
- [ ] Route them through the lifecycle policy module.
- [ ] Add fixtures, rendered-install checks, and drift checks.
- [ ] Test daemon and daemon-less lifecycle outcomes.
- [ ] Preserve conservative unknown handling and document asymmetries.
- [ ] Commit:
  `feat(integrations): improve pi and opencode lifecycle fidelity`

## Verification

- [ ] `cargo test --all-features` passes after every phase.
- [ ] `bash tests/smoke.sh` passes after every phase touching runtime behavior.
- [ ] `bash tests/tpm-bootstrap.sh` passes after every packaging, launcher, or
  hook change.
- [ ] `hk check` passes before every phase commit.
- [ ] `git status --short` contains only the intended phase files before each
  commit.
- [ ] Shared Rust module tests execute once, with hook-only tests retained.
- [ ] Existing `list`, `sessions`, `picker`, `clear`, `doctor`,
  `install-hooks`, and `uninstall-hooks` behavior remains covered.
- [ ] State version remains 1 and existing state files load without migration.
- [ ] Daemon and `AMUX_NO_DAEMON=1` event paths produce equivalent records.
- [ ] Clear cannot be followed by reappearing pre-clear state or event history.
- [ ] Concurrent daemon starters leave exactly one reachable socket owner.
- [ ] Two same-second hook updates retain two non-overwritten backups.
- [ ] Oversized and rejected hook input cannot bypass intake limits.
- [ ] Persisted raw metadata contains no unknown tool/message/command fields.
- [ ] Doctor distinguishes Codex, Claude, Pi, and opencode drift without
  flagging the canonical TPM launcher path.
- [ ] A docs-only main push runs checks but skips release packaging/publication.
- [ ] A releasing main push still publishes all three target archives.
- [ ] Release archives contain the selected project license and generated
  third-party notices.
- [ ] `events` text output is terminal-safe and JSON output is bounded.
- [ ] `watch --json` emits monotonic revisions and fails visibly on disconnect.
- [ ] Manual smoke test: install rendered hooks in an isolated home, emit one
  lifecycle sequence per supported agent, inspect `events`, observe
  `watch --json`, open the picker, run `next-attention`, clear state, and
  confirm `doctor` reports all integrations accurately.
- [ ] Refactor phases 1, 4, 6, and 8 introduce no behavior change beyond the
  separately identified fixes.
- [ ] The measured native hook latency and session projection remain documented;
  any material regression is investigated before proceeding.

## Review

- [ ] Code reviewed after every phase.
- [ ] PLAN.md updated if an approach or phase split changes.
- [ ] All phase commits are clean and use the exact drafted messages.
- [ ] TODO commit boxes are checked only after the corresponding commit succeeds.
- [ ] Open questions are resolved before their dependent phases.
- [ ] No source change extends beyond the phase being committed.
- [ ] TODO.md items are all checked before declaring the roadmap complete.
