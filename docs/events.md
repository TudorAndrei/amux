# amux Event Mappings

All integrations call:

```bash
bin/amux event --agent <agent> --event <event>
```

The raw hook payload is passed on stdin. amux retains a compact, bounded copy
in `events.jsonl`; it is a transition log, so repeated events with unchanged
status, attention, and reason are not appended.

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

Codex global hooks are installed into `~/.codex/hooks.json`; every shipped
command carries an explicit status or attention value, so the mapping does not
depend on event-name inference. Codex has no `Notification` hook, so
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
`Stop` state as attention. Explicit command-line status and attention values
continue to override this payload-based mapping.

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
