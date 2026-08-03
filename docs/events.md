# amux Event Mappings

All integrations call:

```bash
bin/amux event --agent <agent> --event <event>
```

The raw hook payload is passed on stdin and is limited to 256 KiB. Before any
state or history write, amux retains only allowlisted lifecycle, session, cwd,
reason, and subagent metadata. Tool input, messages, command text, and unknown
fields are discarded. `events.jsonl` is a transition log, so repeated events
with unchanged status, attention, and reason are not appended.

The Rust `install-hooks` command renders these templates with the absolute
`bin/amux` path and merges duplicate-safe JSON entries. Use
`bin/amux install-hooks --dry-run` to preview changes or `--write` to apply
them; timestamped backups are created before existing configuration files are
changed.

## Retained history

`amux events` streams valid records from `events.jsonl`, skips malformed lines
with the same tolerance as compaction, filters by `--agent`, `--session`, and
`--pane`, and keeps the newest matches in chronological order. The default
limit is 100 and the accepted range is 1–1,000. A session filter matches a tmux
session or, when hooks ran outside tmux, an agent session id.

Plain output is tab-separated ISO timestamp, agent, status, tmux session, pane,
event, reason, and cwd. All eight fields pass through terminal sanitization.
`--json` emits one bounded document with `version: 1` and an `events` array.
Before either form is emitted, stored records are capped again and raw metadata
is re-applied to the lifecycle/session/cwd/reason/subagent allowlist. This keeps
legacy or manually altered lines from bypassing the current privacy contract.
History is transition-oriented, subject to compaction, and is not a complete
audit trail. Setting `AMUX_EVENTS_PER_SESSION=0` disables automatic compaction;
the command's 1,000-event maximum still applies.

## Live revision stream

`amux watch --json` uses the same IPC subscription as the native picker. It
connects to a running daemon or starts one with the bounded startup retry, then
emits one newline-delimited object containing `revision` and `views` for every
published model revision. The initial line is always the current snapshot;
subsequent revision numbers must increase strictly. A one-update channel bounds
client-side buffering while each complete line is flushed to stdout.

Stdout is reserved for NDJSON. Connection and protocol failures are written to
stderr and exit nonzero, with no silent fallback to cached files after a stream
has connected. A downstream broken pipe exits successfully, while Ctrl-C uses
normal quiet Unix signal termination. Consumers that stop reading longer than
the daemon's write timeout are disconnected rather than allowed to grow an
unbounded queue.

## Codex

- `SessionStart` (`startup|resume|clear`) -> `running`
- `UserPromptSubmit` -> `running`
- `PreToolUse` -> `running`
- `PostToolUse` -> `running`
- `PermissionRequest` -> `attention`
- `PreCompact` -> `running`
- `PostCompact` -> `running`
- `Stop` -> `done`
- `SessionEnd` -> `offline`

Codex global hooks are installed into `~/.codex/hooks.json`. The rendered hook
commands carry only adapter concerns (event names, matchers, timeout, and the
stable permission reason); the in-process lifecycle policy owns every status
and attention mapping. Codex has no `Notification` hook, so
`PermissionRequest` is its attention signal.

`Stop` marks the end of the last observed turn, not the end of the Codex
session. `PreToolUse` holds the row at `running` while tools execute and
`PostToolUse` clears an earlier permission attention after approval. Codex has
no hook that announces a model-only turn start, so a tool-less turn after
`Stop` remains visibly `done` until its next hook; that limitation is
intentional and documented rather than masked with a timer.

## Claude

- `SessionStart` -> `running`
- `UserPromptSubmit` -> `running`
- `Notification` (`permission_prompt`, `agent_needs_input`) -> `attention`
- `Notification` (`idle_prompt`, `agent_completed`) -> `done`
- Unknown `Notification` types -> `attention`
- `Stop` -> `done`

Claude global hooks are installed into `~/.claude/settings.json`. Claude emits
`idle_prompt` after a normal response, so it must not overwrite the preceding
`Stop` state as attention. The in-process policy reads only the retained
notification type. Explicit command-line status and attention values remain
available for manual callers and independently override policy results.

Unknown events from any adapter are treated conservatively as non-terminal
activity (`running`, without attention) unless their stable event name matches
the generic permission/input, completion, or session-end rules.

## Adapter capability matrix

This matrix separates events that the adapters can observe from states amux
cannot determine reliably. It was checked against the current Pi and opencode
primary documentation and source on 2026-08-03. A dash means that the upstream
adapter exposes no stable signal amux can use; amux does not replace missing
signals with timers or polling.

| Adapter | Start | Activity | Attention | Completion | End |
| --- | --- | --- | --- | --- | --- |
| Codex | yes | yes | permission | yes | yes |
| Claude | yes | yes | permission/input | yes | no |
| Pi | yes | yes | no | yes | yes |
| opencode | yes | yes | permission | yes | deletion only |

The exact signals behind the matrix are:

- Codex: `SessionStart`; `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
  `PreCompact`, and `PostCompact`; `PermissionRequest`; `Stop`; and
  `SessionEnd`.
- Claude: `SessionStart`; `UserPromptSubmit`; `Notification` with
  `permission_prompt` or `agent_needs_input`; and `Stop` or `Notification`
  with `idle_prompt` or `agent_completed`. The installed adapter has no
  session-end signal.
- Pi: `session_start`; `agent_start`; no attention signal; `agent_settled`;
  and `session_shutdown`.
- opencode: `session.created`; `session.status` with `busy` or `retry`;
  `permission.asked`; `session.status` with `idle`; and `session.deleted` for
  explicit deletion only. `question.asked` is unstable, deprecated
  `session.idle` duplicates idle status, and normal close is unsupported.

Pi's documented `agent_settled` fires only after the run has no automatic
retry, compaction, or queued continuation left, making it a stronger completion
boundary than `agent_end`. `session_shutdown` covers quit, reload, and session
replacement. Pi also exposes `tool_call` and an `input` event, but `tool_call`
is an extension interception point and `input` means user input was received;
neither reports that Pi itself is waiting for a permission or answer. That
attention capability therefore remains unsupported.

opencode's documented `session.idle` event is a deprecated compatibility
completion notification, not a permission request. `session.status` reports
the authoritative `busy`, `retry`, and `idle` states. The documented
`permission.asked` and `permission.replied` pair carries a session id. Although
the runtime also forwards `question.asked`, that event is absent from the
public plugin event list and typed plugin event union, so amux records it as
unstable and does not use it. `session.deleted` accurately ends a deleted
session but does not report a normal TUI or process close. The current runtime
forwards event data to legacy plugins under `event.properties`; adapters must
not infer lifecycle state from undocumented payload fields.

These confirmed signals are implemented for both adapters. Pi gains exact
activity, completion, and shutdown transitions while retaining its documented
attention limitation. opencode gains exact start, activity, idle, permission,
and deletion transitions while retaining its normal-close and question-input
limitations. Unknown future events remain conservative activity at the policy
seam.

Primary references are pinned to the inspected [Pi revision][pi-revision] and
[opencode revision][opencode-revision]. The exact documentation, type, schema,
runtime publisher, and compatibility-bridge links are recorded in
`docs/integration-capability-research.md`.

[pi-revision]: https://github.com/earendil-works/pi/tree/ebf33c0c
[opencode-revision]: https://github.com/anomalyco/opencode/tree/89130db6

## opencode

The global plugin is installed into `~/.config/opencode/plugins/amux.js`. It
forwards only the confirmed events in the matrix, plus `permission.replied` to
clear attention. Unrelated events, the deprecated duplicate `session.idle`,
and unstable `question.*` events are ignored so they cannot overwrite the
current lifecycle state. The plugin projects only `sessionID` and
`status.type`; prompts, permission resources, tool data, and other event fields
never enter amux intake.

`session.deleted` marks explicit deletion as offline. opencode exposes no
stable session-specific plugin event for normal TUI or process closure, so
normal close continues to rely on the configured stale-record expiry rather
than a timer or process poll in the adapter.

## Pi

The Pi extension is installed into `~/.pi/agent/extensions/amux.ts` and
registered in `~/.pi/agent/settings.json`. It sends only the confirmed event
name, session id, cwd, and optional lifecycle reason. Status and attention are
classified in Rust. Pi exposes no observer event for another component's
permission dialog or an agent waiting for input, so those states remain
unsupported instead of being inferred.
