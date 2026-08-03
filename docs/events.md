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

## opencode

The global plugin records all opencode events it receives. `session.idle` is
treated as `attention` because it is the closest current plugin signal for "the
agent is ready for the user to look".

The plugin is installed into `~/.config/opencode/plugins/amux.js`.

## Pi

The Pi extension records `session_start`, which maps to `running`. Pi attention
support is best-effort until a stronger approval or idle event is exposed.

The extension is installed into `~/.pi/agent/extensions/amux.ts` and registered
in `~/.pi/agent/settings.json`.
