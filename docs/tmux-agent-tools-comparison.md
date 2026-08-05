# Comparison with OpenSessions and tmux-agent-sidebar

Research date: 2026-08-05

This note compares amux with two current tmux agent tools. It uses the local
amux source and first-party project documentation.

## Summary

The three tools have different primary jobs:

- **amux** is an attention router. It receives agent hook events, keeps a small
  state model, shows a popup picker, and moves the tmux client to the agent that
  needs attention.
- **OpenSessions** is a session dashboard and local control plane. It uses a
  persistent sidebar, built-in agent data scanners, a local HTTP and WebSocket
  server, and session management actions.
- **tmux-agent-sidebar** is a pane-level agent console. It uses agent hooks and
  tmux pane options. It shows detailed agent data and includes Git worktree and
  tmux window operations.

## Capability matrix

<!-- markdownlint-disable MD013 -->

| Area | amux | OpenSessions | tmux-agent-sidebar |
| --- | --- | --- | --- |
| Main UI | Popup picker | Persistent sidebar | Persistent sidebar |
| Main list item | One tmux session, with agent details | One tmux session, with thread details | One agent pane |
| State input | Agent lifecycle hooks | Agent file/database scanners and HTTP events | Agent hooks and an OpenCode plugin bridge |
| Named agents | Codex, Claude, Pi, opencode | Amp, Claude Code, Codex, OpenCode; the Rust runtime also has Pi and Droid scanners | Claude Code, Codex, OpenCode |
| Status model | `running`, `attention`, `done`, `offline`, `unknown` | `idle`, `running`, `tool-running`, `done`, `error`, `waiting`, `interrupted`, `stale` | `running`, `background`, `waiting`, `idle`, `error` |
| Main actions | Search, switch, jump to newest attention item | Switch, reorder, hide, restore, create, and kill sessions | Switch panes, filter panes, and create or remove a worktree and window |
| Extra context | Reason, directory, pane, agent, and age | Branch, dirty state, directory, threads, ports, custom status, progress, and logs | Prompts, response previews, tools, tasks, subagents, Git state, ports, and permission mode |
| Program interface | CLI JSON, NDJSON watch stream, and private Unix socket | Local HTTP API and WebSocket state stream | Tmux pane options for scripts |
| Durable history | Bounded lifecycle transition log | No equivalent lifecycle history is documented | Bounded tool activity files in `/tmp` |
| Native layout | One Rust binary | Two Rust binaries plus bundled `lazydiff` | One Rust binary |
| License | Apache-2.0 | MIT | MIT |

<!-- markdownlint-enable MD013 -->

## State quality and data use

amux uses explicit lifecycle hooks. Its event policy does not replace a missing
agent signal with a timer or process guess. This gives a small and clear model,
but some upstream limits remain visible. For example, Pi has no stable
attention signal, and opencode has no stable normal-close signal. See the local
[event mapping](events.md).

OpenSessions scans Amp thread files, Claude and Codex transcripts, and the
OpenCode SQLite database. Its Claude scanner can use recent silence to derive
`waiting` and longer silence to derive `stale`. It also accepts direct events
through `POST /api/agent-event`. This design needs less per-agent hook setup,
but it has more dependence on agent storage formats and polling behavior.
Sources: [OpenSessions contracts](https://github.com/Ataraxy-Labs/opensessions/blob/main/CONTRACTS.md)
and [architecture](https://github.com/Ataraxy-Labs/opensessions/blob/main/docs/explanation/architecture.md).

tmux-agent-sidebar uses hooks for agent events. The hook command writes state
to tmux pane options. The sidebar reads these options each second. It also
stores a bounded tool activity file for each pane in `/tmp`. Agent support is
not equal: Claude Code supplies the most data, while Codex and OpenCode supply
fewer hook fields. Sources: [state model](https://github.com/hiroppy/tmux-agent-sidebar/blob/main/docs/state-management.md),
[agent support matrix](https://hiroppy.github.io/tmux-agent-sidebar/agents/),
and [activity log](https://hiroppy.github.io/tmux-agent-sidebar/features/activity-log/).

amux has the smallest stored data set. It discards messages, tool input,
command text, and unknown fields. OpenSessions reads agent transcripts and can
show the last prompt. tmux-agent-sidebar writes the latest prompt or response
to a tmux pane option and stores selected tool labels in activity files. Thus,
the two sidebar tools give more context, but they also have a larger local data
footprint.

## User experience

OpenSessions is best for users who manage many tmux sessions and local web
services. Its sidebar can show Git and port data. It can create, reorder, hide,
restore, and kill sessions. It also supplies a general HTTP surface for custom
agent state and build or deployment data. Sources: [README](https://github.com/Ataraxy-Labs/opensessions),
[features](https://github.com/Ataraxy-Labs/opensessions/blob/main/docs/reference/features-and-keybindings.md),
and [programmatic API](https://github.com/Ataraxy-Labs/opensessions/blob/main/docs/reference/programmatic-api.md).

tmux-agent-sidebar is best for users who need detailed pane data, especially
for Claude Code. It shows tool activity, task progress, wait reasons,
subagents, Git changes, and listening ports. It can create a Git worktree,
create a tmux window, and start an agent in one operation. It can also remove
the window, worktree, and branch. This is a large and potentially destructive
control surface. Source: [worktree behavior](https://hiroppy.github.io/tmux-agent-sidebar/features/worktree/).

amux is best for users who want a low-clutter answer to one question: which
agent needs me now? Its popup does not reserve terminal width. The
`next-attention` command can switch directly to the newest live attention
item. The picker also keeps agent payload content out of the UI and state.
See the local [README](../README.md) and [performance note](performance.md).

## Product conclusion

OpenSessions and tmux-agent-sidebar have a larger feature surface than amux.
They are not direct replacements for the current amux product goal.

amux has a useful, clear position if it keeps these properties:

1. Exact hook-based lifecycle state.
2. A small, private data projection.
3. Fast attention-first navigation.
4. No permanent sidebar and no session or worktree management.

Useful ideas that fit the amux boundary are per-agent seen/unseen state, simple
status filters, and optional Git branch text. Transcript scanners, prompt and
response storage, tool logs, and worktree deletion do not fit the current
boundary. They would change amux from an attention router into an agent
operations console.
