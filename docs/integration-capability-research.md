# Pi and opencode lifecycle capability research

Research date: 2026-08-03

This note records the upstream evidence used for amux Phase 14. It is a
research artifact, not the user-facing event contract. The capability matrix
and the implementation decision it supports live in `docs/events.md`.

## Scope and evidence standard

The review covers session start, agent activity, permission or input attention,
completion/idle, and session end. It uses only official documentation and
first-party source code, pinned to these upstream revisions:

- Pi: [`earendil-works/pi@ebf33c0c`](https://github.com/earendil-works/pi/tree/ebf33c0c2282fb8c027174d3d2b53519d8f564e3)
- opencode: [`anomalyco/opencode@89130db6`](https://github.com/anomalyco/opencode/tree/89130db6b0060a345548d870c51132ee71d6a828)

The labels below mean:

- **Supported**: present in the public extension/plugin documentation and in a
  first-party type or schema at the inspected revision.
- **Confirmed but unstable**: emitted by first-party implementation code, but
  omitted from or inconsistent with the public plugin contract.
- **Unsupported**: no session-specific public signal exists for the lifecycle
  transition. amux must not manufacture one with polling or a timer.

## Capability summary

`S` means supported, `C` means confirmed but unstable, and `U` means
unsupported. The following sections record the qualifications behind each
short matrix entry.

| Lifecycle point | Pi | opencode |
| --- | --- | --- |
| Session start | S: `session_start` | S: `session.created` |
| Agent activity | S: `agent_start` | S: `session.status` (`busy`) |
| Permission attention | U | S: `permission.asked` |
| Agent requests input | U | C: `question.asked` |
| Completion/idle | S: `agent_settled` | S: `session.status` (`idle`) |
| Session end | S: `session_shutdown` | U for normal close |

## Pi

### Public lifecycle and payloads

Pi's official lifecycle overview places `session_start` at runtime startup,
`agent_start` before an agent run, `agent_settled` after all automatic work, and
`session_shutdown` on replacement or process exit. It also shows the detailed
turn, message, and tool events available during activity. See the pinned
[lifecycle overview](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/docs/extensions.md#L280-L347).

The representative public types are:

```ts
type SessionStartEvent = {
  type: "session_start"
  reason: "startup" | "reload" | "new" | "resume" | "fork"
  previousSessionFile?: string
}

type AgentStartEvent = { type: "agent_start" }
type AgentEndEvent = { type: "agent_end"; messages: AgentMessage[] }
type AgentSettledEvent = { type: "agent_settled" }

type SessionShutdownEvent = {
  type: "session_shutdown"
  reason: "quit" | "reload" | "new" | "resume" | "fork"
  targetSessionFile?: string
}
```

These shapes come from Pi's first-party
[`SessionStartEvent` and `SessionShutdownEvent` definitions](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/src/core/extensions/types.ts#L557-L621)
and
[`AgentStartEvent`, `AgentEndEvent`, and `AgentSettledEvent` definitions](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/src/core/extensions/types.ts#L698-L725).
The `ExtensionAPI.on` overloads expose all four events to extensions
([source](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/src/core/extensions/types.ts#L1195-L1239)).

### Completion signal

`agent_settled` is the correct completion signal. The official documentation
warns that `agent_end` may be followed by an automatic retry, compaction and
retry, or a queued follow-up; it explicitly recommends `agent_settled` for
status integrations
([documentation](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/docs/extensions.md#L558-L571)).
The implementation clears the active-run flag, emits `agent_settled`, and only
then resolves idle waiters
([source](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/src/core/agent-session.ts#L581-L588)).

### Why permission/input attention remains unsupported

Pi exposes `tool_call`, but its contract is a pre-execution interception point:
the receiving extension may mutate or block the tool call. It is not a
notification that Pi or another extension is waiting for permission
([documentation](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/docs/extensions.md#L749-L789)).
The permission-gate example creates its own `ctx.ui.confirm`; there is no
separate permission-request event in the complete `ExtensionAPI.on` overload
set cited above.

Likewise, `input` fires when input is *received* from `interactive`, `rpc`, or
`extension` sources. It does not mean the agent is blocked waiting for input
([documentation](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/docs/extensions.md#L881-L914),
[type](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/src/core/extensions/types.ts#L826-L847)).
`project_trust` is also unsuitable: the event occurs before trust resolution,
so an `undecided` handler cannot know whether saved policy or the built-in UI
will actually prompt. Mapping any of these events to attention would create
false positives.

### Session end

`session_shutdown` is a real teardown signal with explicit `quit`, `reload`,
`new`, `resume`, and `fork` reasons
([documentation](https://github.com/earendil-works/pi/blob/ebf33c0c2282fb8c027174d3d2b53519d8f564e3/packages/coding-agent/docs/extensions.md#L507-L517)).
For replacement and reload flows, an offline transition may be followed
immediately by the replacement runtime's `session_start`; that is an accurate
event sequence, not a reason to omit shutdown.

## opencode

### How events reach a plugin

The public plugin interface offers a generic `event` hook and a `dispose` hook
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/plugin/src/index.ts#L222-L228)).
At runtime, opencode listens to its event bus and calls every plugin event hook
with `{ id, type, properties }`; the value is deliberately cast to `any`
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/plugin/index.ts#L253-L275)).
The official plugin documentation lists `permission.asked`,
`permission.replied`, `session.created`, `session.deleted`, `session.idle`, and
`session.status` among supported events
([documentation](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/web/src/content/docs/plugins.mdx#L142-L208)).

### Start, activity, and completion payloads

The authoritative status schema is:

```ts
type SessionStatus =
  | { type: "idle" }
  | { type: "busy" }
  | {
      type: "retry"
      attempt: number
      message: string
      next: number
      action?: object
    }

type SessionStatusEvent = {
  type: "session.status"
  properties: { sessionID: string; status: SessionStatus }
}
```

The first-party schema defines all three states and the event payload. It also
marks the separate `session.idle` event as deprecated
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/schema/src/session-status-event.ts#L9-L51)).
The runtime publishes `session.status` first and additionally publishes the
deprecated `session.idle` compatibility event when entering idle
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/session/status.ts#L30-L49)).

For a newly created session, the implementation publishes
`session.created` with `{ sessionID, info }`
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/session/session.ts#L501-L540)).
Creation is a valid start signal, but it does not repeat when work resumes in an
existing session. `session.status` `busy` is therefore the authoritative
activity transition. `retry` describes scheduled automatic work and should
remain non-attention activity; amux should not infer that the user is needed.

For completion, use `session.status` `idle`. The documentation's notification
example describes the legacy `session.idle` event as session completion
([documentation](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/web/src/content/docs/plugins.mdx#L218-L232)),
but new code should consume the non-deprecated status event and ignore the
duplicate compatibility event.

### Permission and question attention

The stable permission request payload is:

```ts
type PermissionAskedEvent = {
  type: "permission.asked"
  properties: {
    id: string
    sessionID: string
    permission: string
    patterns: string[]
    metadata: Record<string, unknown>
    always: string[]
    tool?: { messageID: string; callID: string }
  }
}
```

The first-party schema defines both `permission.asked` and
`permission.replied`; the reply contains `sessionID`, `requestID`, and
`reply: "once" | "always" | "reject"`
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/schema/src/v1/permission.ts#L27-L66)).
The permission service publishes the asked event before waiting for the reply
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/permission/index.ts#L86-L106)).
This is a reliable attention transition. amux needs only the event type and
session identity; patterns, tool inputs, and metadata must remain outside the
durable projection.

opencode also implements a semantically strong `question.asked` event with
`{ id, sessionID, questions, tool? }`, followed by `question.replied` or
`question.rejected`
([schema](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/schema/src/v1/question.ts#L15-L65),
[publisher](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/question/index.ts#L91-L110)).
However, `question.*` is omitted from the public plugin event list. At the
inspected revision it is also omitted from the legacy SDK `Event` union used by
the plugin hook type, even though the runtime's untyped forwarding means a
generic JavaScript hook receives it. Treat this as confirmed but unstable and
do not enable it in Phase 15 until upstream documents/types it as a plugin
contract.

### Session end remains asymmetric

`session.deleted` is public and carries `{ sessionID, info }`, but the runtime
emits it from the recursive session-removal operation
([source](https://github.com/anomalyco/opencode/blob/89130db6b0060a345548d870c51132ee71d6a828/packages/opencode/src/session/session.ts#L608-L628)).
It can accurately mark a deleted session offline; it is not a normal session
close signal.

The plugin `dispose` callback runs during plugin-runtime finalization, but it
has no session payload and may represent reload or instance teardown rather
than closure of one session. It cannot safely produce a session-specific
offline transition. Normal opencode TUI/process exit therefore remains an
explicit unsupported asymmetry; existing stale-record expiry is the only
honest fallback.

## Phase 15 recommendation

Proceed with a small, allowlisted signal set:

1. **Pi:** install handlers for `session_start`, `agent_start`,
   `agent_settled`, and `session_shutdown`. Route all four names through the
   Rust lifecycle policy. Do not add Pi attention based on `tool_call`,
   `input`, `project_trust`, elapsed time, or polling.
2. **opencode:** forward `session.created`, `session.status`,
   `permission.asked`, `permission.replied`, and `session.deleted`. Retain only
   `sessionID`, `status.type`, and the minimal reply classification needed by
   policy. Ignore the deprecated duplicate `session.idle` and unrelated event
   traffic so it cannot overwrite a terminal/attention transition as generic
   activity.
3. Keep `question.asked`/`question.replied`/`question.rejected` disabled and
   documented as unstable until they enter the public plugin event contract.
4. Document that Pi has no trustworthy permission/input-attention observer and
   opencode has no trustworthy normal-close event. Do not compensate with
   timers, process polling, or adapter-side status rules.

The opencode choice of whether an idle completion is displayed as `done` or as
the existing “ready for review” attention state is an amux product policy, not
an upstream payload fact. Preserve that choice in `src/lifecycle.rs`; the
JavaScript adapter should only report the upstream event and minimal metadata.
