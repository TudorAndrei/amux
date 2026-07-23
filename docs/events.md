# amux Event Mappings

All integrations call:

```bash
bin/amux event --agent <agent> --event <event>
```

The raw hook payload is passed on stdin and stored in `events.jsonl`.

## Codex

- `SessionStart` -> `running`
- `UserPromptSubmit` -> `running`
- `PermissionRequest` -> `attention`
- `PostToolUse` -> `running`
- `Stop` -> `done`

Codex global hooks are installed into `~/.codex/hooks.json`.
`UserPromptSubmit` is explicitly treated as running and never inferred as an
attention event merely because its name contains "prompt".

## Claude

- `SessionStart` -> `running`
- `UserPromptSubmit` -> `running`
- `PreToolUse` -> `running`
- `PostToolUse` -> `running`
- `Notification` -> `attention`
- `Stop` -> `done`

Claude global hooks are installed into `~/.claude/settings.json`.

## opencode

The global plugin records all opencode events it receives. `session.idle` is
treated as `attention` because it is the closest current plugin signal for "the
agent is ready for the user to look".

The plugin is installed into `~/.config/opencode/plugins/amux.js`.

## Pi

The Pi extension records `session_start`, `tool_call`, and `tool_result` events
through the extension API. These currently map to `running`; Pi attention
support is best-effort until a stronger approval or idle event is exposed.

The extension is installed into `~/.pi/agent/extensions/amux.ts` and registered
in `~/.pi/agent/settings.json`.
