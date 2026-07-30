# Plan: Classify Claude completion notifications correctly

## Goal

Prevent a completed Claude Code turn from being overwritten as `attention` when
Claude subsequently emits its normal `Notification` payload with
`notification_type: "idle_prompt"`. Preserve attention for actual permission
and input requests, while keeping Codex's existing lifecycle mapping unchanged.

## Approach

Keep the integration as a single Claude `Notification` hook so it continues to
capture every current and future notification payload. Move the decision from
the template's unconditional `--attention 1` override into
`src/event.rs::normalize_at`, where the raw `notification_type` is available to
both daemon and daemon-less event paths.

For Claude notifications, classify `permission_prompt` and
`agent_needs_input` as `attention`; classify `idle_prompt` and
`agent_completed` as `done` with attention cleared. Unknown notification types
will retain the current conservative attention behavior. Make `Stop` explicit
in the Claude template, matching Codex's terminal-hook contract. Update the
event mapping documentation and tests, including the observed `Stop` followed
by `idle_prompt` sequence. Explicit CLI status and attention overrides retain
their existing precedence over this inferred classification. The TPM launcher
path remains canonical and is out of scope for this change.

## Implementation Phases

### Phase 1: Make Claude notification normalization payload-aware

- Update `hooks/claude/settings.fragment.json` so `Notification` does not
  force attention before payload-aware normalization, and so `Stop` explicitly
  writes `done` with attention cleared.
- Add Claude-specific notification-type classification in
  `src/event.rs::normalize_at`, without changing generic or Codex inference.
- Add focused unit tests in `src/event.rs` covering `idle_prompt`,
  `permission_prompt`, `agent_completed`, `agent_needs_input`, and an unknown
  notification type.
  **Commit:** `fix(claude): distinguish completion notifications from attention`

### Phase 2: Prove the installed hook contract and document it

- Extend `tests/rust_smoke.rs` to render the Claude hook template and assert
  that an emitted `Stop` followed by `idle_prompt` stays `done`, while a
  `permission_prompt` becomes `attention`, in both daemon and
  `AMUX_NO_DAEMON=1` paths.
- Update `docs/events.md` and the Claude status-model wording in `README.md`
  to document the notification-type mapping and terminal-event precedence.
- Run the relevant Rust tests, `tests/smoke.sh`, and `hk check`.
  **Commit:** `test(claude): cover stop followed by idle notification`

## Risks & Tradeoffs

- Claude Code may add notification types. Unknown values remain `attention` to
  avoid hiding a genuine request for user action.
- `agent_completed` may come from background agents rather than the foreground
  pane; mapping it to `done` is consistent with its completed state and avoids
  a false attention badge.
- Hook matcher semantics vary across Claude Code releases. Payload-aware
  normalization avoids relying on matcher support beyond the existing `*`
  subscription.

## Open Questions

- None. The observed `idle_prompt` payload and expected completed UI state
  define the required behavior.
