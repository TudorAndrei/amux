# Plan: Hook-Driven Agent Multiplexer

## Goal
Build `amux` into a tmux plugin that tracks Codex, Claude, Pi, and opencode through their global hook or plugin systems, then exposes accurate agent state in tmux. The first version should prioritize reliable "needs user attention" signals over pane-scraping heuristics, while keeping the current repo small, shell-friendly, and easy to install from the existing `amux.tmux` entrypoint.

## Approach
The plugin will use a single event sink as its core contract: every agent hook calls an `amux` command with JSON on stdin, and `amux` normalizes that event into a small state file under `${XDG_STATE_HOME:-$HOME/.local/state}/amux`. This keeps tmux rendering cheap because status and picker scripts read cached state instead of capturing panes.

The current repo only contains `README.md`, `amux.tmux`, and `scripts/picker.sh`, so the first architectural step is to add a real command surface under `bin/amux` and shared shell helpers under `lib/`. `amux.tmux` will set `AMUX_ROOT`, bind the picker, and expose a status fragment via `#(...)`. `scripts/picker.sh` will stop being a placeholder and will render state from the shared store with `fzf` when available.

Global hooks will be delivered as installable assets rather than hard-coded dotfile edits. Codex and Claude will get global hook JSON snippets that call `bin/amux event --agent <name> --event <event>`. opencode will get a global plugin module for `~/.config/opencode/plugins/`, and Pi will get an extension module compatible with its `extensions` settings. A `scripts/install-hooks.sh` installer will create backups before touching global files and will be idempotent enough to rerun after updates.

State will be intentionally simple: normalized records keyed by agent, agent session id when available, tmux session, pane id, cwd, status, attention flag, reason, last event, and timestamp. Supported statuses for the initial scope are `running`, `attention`, `done`, and `unknown`. Hook events that request permission, notify that the agent is idle, or stop waiting for user input will set `attention=true`; normal activity events will set `running`; stop/session-idle events will set `done` unless the event implies the user should review or respond.

Out of scope for the initial implementation: pane text scraping, a daemon, cross-machine sync, and a full TUI. The initial implementation should include installing the plugin into the current dotfiles tmux configuration at `../tmux.conf`, because that is how this machine actually loads tmux behavior today.

## Implementation Phases

### Phase 1: Core CLI and State Store
- Add `bin/amux` as the main command with subcommands for `event`, `status`, `list`, `clear`, and `doctor`.
- Add shared helpers under `lib/` for path resolution, JSON escaping/parsing with `jq`, tmux context discovery, and atomic state writes.
- Define the state schema in `README.md`, including status meanings and the event normalization rules.
- Add fixture-based shell tests or scriptable smoke checks under `tests/` for ingesting sample Codex, Claude, Pi, and opencode events.
**Commit:** `feat(core): add hook event sink and state store`

### Phase 2: tmux Plugin UI
- Update `amux.tmux` to configure `AMUX_ROOT`, default key bindings, and optional status integration without overwriting a user's whole status line.
- Replace `scripts/picker.sh` with a state-backed picker that shows agent, tmux session, status, attention reason, cwd, and last update time.
- Add `scripts/status.sh` for a compact tmux status segment, prioritizing attention counts over running/done counts.
- Add `scripts/next-attention.sh` to jump to the next tmux pane or session with `attention=true` when the state includes tmux identifiers.
**Commit:** `feat(tmux): add status and attention picker`

### Phase 3: Dotfiles tmux Installation
- Update the parent dotfiles tmux configuration at `../tmux.conf` to load the local `amux.tmux` plugin from `configs/tmux/amux`.
- Wire a visible `amux` status segment into the existing `status-right` without replacing the current date/time segment.
- Decide whether the existing `prefix + l` session picker should remain on `../session-picker.sh` or move to the new `amux` picker after feature parity.
- Document the local dotfiles install path in `README.md` so this repo can be used both as a standalone TPM plugin and as the checked-out plugin in `/Users/tudor/dotfiles/configs/tmux/amux`.
**Commit:** `chore(dotfiles): install amux in local tmux config`

### Phase 4: Global Agent Hook Assets
- Add `hooks/codex/hooks.json` for global Codex events including `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`, and `Stop`.
- Add `hooks/claude/settings.fragment.json` or equivalent mergeable config for global Claude hooks including `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, and `Stop`.
- Add `hooks/opencode/amux.js` as a global opencode plugin that records at least `session.idle` and generic event activity.
- Add `hooks/pi/amux.ts` as a Pi extension that records session and tool lifecycle events through Pi's extension API.
- Document any event names that are best-effort because the agent API does not expose a stronger attention signal yet.
**Commit:** `feat(hooks): add global integrations for supported agents`

### Phase 5: Installer and Documentation
- Add `scripts/install-hooks.sh` to install or merge the global hook assets with backups for `~/.codex/hooks.json`, `~/.claude/settings.json`, `~/.config/opencode/plugins/amux.js`, and `~/.pi/agent/settings.json`.
- Add `scripts/uninstall-hooks.sh` or documented rollback steps for removing installed hook entries.
- Expand `README.md` with TPM installation, manual tmux sourcing, hook installation, state file locations, and troubleshooting.
- Add `docs/events.md` with sample raw hook input and normalized output for each supported agent.
**Commit:** `docs(install): document and automate global hook setup`

### Phase 6: Hardening and Release Prep
- Add lint/check targets in a lightweight `Makefile` for shell syntax, shellcheck when available, and fixture smoke tests.
- Add defensive handling for missing `jq`, missing `tmux`, unavailable `fzf`, stale state records, and malformed hook JSON.
- Add a minimal release checklist and version metadata so the plugin can be tagged cleanly.
- Run end-to-end manual tests from tmux with synthetic hook events before tagging the first usable version.
**Commit:** `chore(release): add checks and first release prep`

## Risks & Tradeoffs
- Hook APIs differ across agents and may change. Mitigation: keep each integration in its own `hooks/<agent>/` asset and normalize through `bin/amux event` instead of spreading assumptions through tmux UI scripts.
- Some agents may not expose a perfect "needs attention" event. Mitigation: mark uncertain mappings in `docs/events.md` and prefer conservative `attention` only for explicit permission, notification, idle, or stop signals.
- Merging global JSON config is easy to get wrong. Mitigation: require `jq`, write backups, make install idempotent, and provide manual snippets for users who do not want automatic merging.
- tmux status commands must be fast. Mitigation: `scripts/status.sh` reads cached state only and never captures panes or walks process trees.
- Nested repo placement inside the parent dotfiles tree can be confusing. Mitigation: make the parent `../tmux.conf` change an explicit install phase and document whether it is committed in the parent dotfiles repo or tracked as local setup.

## Open Questions
- Should `done` imply `attention=true` by default, or should only explicit permission/notification/idle events request attention?
- Should `scripts/install-hooks.sh` mutate existing global config files automatically by default, or require an explicit `--write` flag after showing a dry run?
- Should the state store use one compact `state.json` file or a JSONL event log plus derived state file for easier debugging?
- What icon/text format should the tmux status segment use for attention in your existing Dracula-style tmux theme?
- Should `prefix + l` eventually be replaced by the `amux` picker, or should `amux` live on `prefix + A` until the existing session picker behavior is fully preserved?
