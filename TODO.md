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

- [x] Put state, topology, views, and revision mutation behind live-model
  methods.
- [x] Centralize view recomputation and revision publication.
- [x] Keep monitor, shutdown, and maintenance facts internal.
- [x] Test event, clear, changed/unchanged topology, and coherent snapshots.
- [x] Verify no CLI or serialized-state behavior changes.
- [x] Commit: `refactor(daemon): concentrate live model invariants`

## Phase 5: Coordinate and recover event-log maintenance

- [x] Replace detached compaction state with tracked maintenance coordination.
- [x] Prevent pre-clear retained lines from installing after clear.
- [x] Recover or report post-rename compaction failures without restart.
- [x] Preserve chronology across orphan, retained, and newly appended events.
- [x] Add deterministic barriers for clear/compaction and failure injection.
- [x] Prove cleared state and event history remain absent after maintenance.
- [x] Commit: `fix(state): coordinate and recover event log maintenance`

## Phase 6: Deepen IPC transport and error semantics

- [x] Move socket path, framing, decoding, and validation into `src/ipc.rs`.
- [x] Distinguish daemon unavailability from daemon rejection.
- [x] Preserve absent-daemon shutdown success and validate connected responses.
- [x] Return revision plus views from the subscription interface.
- [x] Add bounded one-shot timeouts without timing out subscriptions.
- [x] Consolidate malformed, oversized, large-response, disconnect, rejection,
  and timeout socket tests.
- [x] Commit: `refactor(ipc): own framing and response validation`

## Phase 7: Build one bounded durable event-intake module

- [x] Add one module for bounded parsing, normalization, retention, and
  persistence.
- [x] Reject oversized stdin before full allocation or JSON parsing.
- [x] Project retained raw data through an explicit metadata allowlist.
- [x] Cap every persisted identifier and key contribution on UTF-8 boundaries.
- [x] Route daemon and daemon-less writes through the same intake interface.
- [x] Prevent daemon rejections from falling through to direct persistence.
- [x] Test path equivalence, oversized input/identifiers, sensitive unknown
  fields, subagent metadata, empty stdin, and malformed JSON.
- [x] Commit: `fix(event): enforce one bounded durable intake path`

## Phase 8: Concentrate lifecycle policy

- [x] Add the in-process lifecycle policy module.
- [x] Move Claude, Codex, and generic classification out of
  `event::normalize_at`.
- [x] Remove duplicated status policy from hook command arguments.
- [x] Preserve explicit override precedence and conservative unknown handling.
- [x] Move policy assertions to lifecycle-interface tests.
- [x] Keep rendered Codex/Claude adapter smoke coverage.
- [x] Update `README.md` and `docs/events.md`.
- [x] Commit: `refactor(events): centralize agent lifecycle policy`

## Phase 9: Report drift for every installed integration

- [x] Compare rendered Pi and opencode text assets in `hooks::drift_at`.
- [x] Normalize launcher assignments without treating TPM paths as drift.
- [x] Verify Pi settings registration.
- [x] Print honest per-integration doctor results.
- [x] Test stale text, launcher-only differences, missing Pi registration, and
  all-four-current output.
- [x] Update `docs/updating.md`.
- [x] Commit: `fix(doctor): verify every installed agent integration`

## Phase 10: Harden workflow dependencies and release gating

- [x] Pin every `uses:` entry to a verified full commit SHA with tag comments.
- [x] Add scheduled GitHub Actions updates in `.github/dependabot.yml`.
- [x] Emit whether Cocogitto created a new release commit/tag.
- [x] Gate release push, packaging, and publication on that result.
- [x] Keep all three check jobs on every push.
- [x] Verify actionlint, immutable pins, releasing flow, and docs-only flow.
- [x] Commit: `ci(release): pin actions and skip no-op packaging`

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

- [x] Add the `events` CLI command.
- [x] Stream valid retained JSONL with agent/session/pane/limit filters.
- [x] Define chronological plain-text and bounded JSON output.
- [x] Sanitize every default text field.
- [x] Test empty history, filters, limits, malformed lines, disabled retention,
  and terminal controls.
- [x] Document history privacy and retention.
- [x] Commit: `feat(cli): expose retained event history`

## Phase 13: Add a headless live JSON stream

- [x] Add `watch --json` with one NDJSON object per revision.
- [x] Reuse the IPC subscription in `src/ui.rs`.
- [x] Define startup, disconnect, and stderr behavior.
- [x] Document stream framing and revision monotonicity.
- [x] Test initial output, revisions, large snapshots, slow consumers,
  disconnect, and Ctrl-C.
- [x] Commit: `feat(cli): stream live session revisions as json`

## Phase 14: Record Pi and opencode lifecycle capabilities

- [x] Read current primary Pi and opencode hook documentation.
- [x] Capture representative lifecycle fixtures.
- [x] Add the four-adapter capability matrix to `docs/events.md`.
- [x] Record unsupported/unstable signals without inferred timers.
- [x] Decide whether confirmed signals unblock Phase 15.
- [x] Commit: `docs(integrations): record lifecycle capability matrix`

## Phase 15: Improve supported Pi and opencode lifecycle fidelity

- [x] Extend only signals confirmed stable in Phase 14.
- [x] Route them through the lifecycle policy module.
- [x] Add fixtures, rendered-install checks, and drift checks.
- [x] Test daemon and daemon-less lifecycle outcomes.
- [x] Preserve conservative unknown handling and document asymmetries.
- [x] Commit:
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
