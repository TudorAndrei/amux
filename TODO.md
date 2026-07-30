# TODO: Classify Claude completion notifications correctly

## Phase 1: Make Claude notification normalization payload-aware

- [x] Update `hooks/claude/settings.fragment.json` to defer Claude
  notification classification and explicitly clear attention on `Stop`.
- [x] Add notification-type classification in `src/event.rs::normalize_at`.
- [x] Add `src/event.rs` unit coverage for `idle_prompt`, `permission_prompt`,
  `agent_completed`, `agent_needs_input`, and unknown notification types.
- [x] Commit: `fix(claude): distinguish completion notifications from attention`

## Phase 2: Prove the installed hook contract and document it

- [x] Add a rendered-hook regression scenario in `tests/rust_smoke.rs` for
  `Stop` followed by `idle_prompt` and `permission_prompt`, in both daemon and
  `AMUX_NO_DAEMON=1` paths.
- [x] Update `docs/events.md` and `README.md` with the Claude
  notification-type mapping.
- [x] Run `cargo test`, `tests/smoke.sh`, and `hk check`.
- [x] Commit: `test(claude): cover stop followed by idle notification`

## Verification

- [x] `src/event.rs` keeps a Claude `idle_prompt` record at `done` with
  `attention == false` after a preceding `Stop`.
- [x] `src/event.rs` maps `permission_prompt` and `agent_needs_input` to
  `attention`.
- [x] Explicit `--status` and `--attention` arguments retain precedence over
  Claude notification-type inference.
- [x] The rendered Claude template works in both daemon and daemon-less event
  paths without hard-coding the TPM launcher location.
- [x] `tests/rust_smoke.rs` verifies the rendered hook lifecycle regression.
- [x] `cargo test`, `tests/smoke.sh`, and `hk check` pass.
- [x] Codex's existing explicit terminal mapping remains unchanged.

## Review

- [x] Code reviewed.
- [x] PLAN.md updated if approach changed during implementation.
- [x] All phase commits are clean and describe their intent.
- [x] TODO.md items all checked off.
