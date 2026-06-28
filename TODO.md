# TODO: Hook-Driven Agent Multiplexer

## Phase 1: Core CLI and State Store
- [x] Add `bin/amux` with `event`, `status`, `list`, `clear`, and `doctor` subcommands.
- [x] Add shared shell helpers under `lib/` for paths, JSON handling, tmux context, and atomic state writes.
- [x] Document the normalized state schema and status meanings in `README.md`.
- [x] Add fixture-based smoke checks under `tests/` for Codex, Claude, Pi, and opencode sample events.
- [x] Commit: `feat(core): add hook event sink and state store`

## Phase 2: tmux Plugin UI
- [x] Update `amux.tmux` to configure environment, key bindings, and optional status integration.
- [x] Replace `scripts/picker.sh` with a state-backed picker showing agent, session, status, reason, cwd, and age.
- [x] Add `scripts/status.sh` for a compact attention-first tmux status segment.
- [x] Add `scripts/next-attention.sh` to jump to the next tmux pane or session needing attention.
- [x] Commit: `feat(tmux): add status and attention picker`

## Phase 3: Dotfiles tmux Installation
- [x] Update `../tmux.conf` to load `configs/tmux/amux/amux.tmux`.
- [x] Add the `amux` status segment to the existing `status-right` without removing the date/time segment.
- [x] Decide whether `prefix + l` stays on `../session-picker.sh` or moves to the new `amux` picker after feature parity.
- [x] Document standalone and dotfiles-local tmux installation paths in `README.md`.
- [x] Commit: `chore(dotfiles): install amux in local tmux config`

## Phase 4: Global Agent Hook Assets
- [x] Add `hooks/codex/hooks.json` for global Codex lifecycle and permission events.
- [x] Add `hooks/claude/settings.fragment.json` or equivalent mergeable global Claude hook config.
- [x] Add `hooks/opencode/amux.js` as a global opencode plugin.
- [x] Add `hooks/pi/amux.ts` as a Pi extension.
- [x] Document best-effort event mappings where an agent lacks an explicit attention signal.
- [x] Commit: `feat(hooks): add global integrations for supported agents`

## Phase 5: Installer and Documentation
- [x] Add `scripts/install-hooks.sh` with backups and idempotent global config merging.
- [x] Add `scripts/uninstall-hooks.sh` or documented rollback steps.
- [x] Expand `README.md` with TPM install, manual source, hook install, state paths, and troubleshooting.
- [x] Add `docs/events.md` with raw and normalized event examples for each supported agent.
- [x] Commit: `docs(install): document and automate global hook setup`

## Phase 6: Hardening and Release Prep
- [x] Add a lightweight `Makefile` for syntax checks, optional shellcheck, and fixture smoke tests.
- [x] Handle missing `jq`, missing `tmux`, missing `fzf`, stale state records, and malformed hook JSON.
- [x] Add release checklist and version metadata.
- [x] Run end-to-end manual tests from tmux with synthetic hook events.
- [x] Commit: `chore(release): add checks and first release prep`

## Verification
- [x] `bash -n bin/amux scripts/*.sh lib/*.sh` passes after shell files are added.
- [x] `shellcheck bin/amux scripts/*.sh lib/*.sh` is wired into `make check`; skipped locally because `shellcheck` is not installed.
- [x] `bin/amux event` correctly normalizes sample events in `tests/fixtures/`.
- [x] `scripts/status.sh` returns quickly and does not call `tmux capture-pane`.
- [x] `scripts/picker.sh` handles an empty state file without errors.
- [x] `scripts/next-attention.sh` handles missing tmux pane/session ids without switching to the wrong target.
- [x] `scripts/install-hooks.sh --dry-run` shows intended changes without writing global config files.
- [x] `tmux source-file ../tmux.conf` loads the local `amux.tmux` plugin without errors.
- [x] The existing `../tmux.conf` date/time status segment still renders after adding the `amux` segment.
- [x] Manual smoke test: inside tmux, send synthetic Codex, Claude, Pi, and opencode events to `bin/amux event`, confirm the status segment and picker prioritize attention records.
- [x] Edge cases tested: malformed JSON input, missing `jq`, missing `fzf`, stale state entries, repeated hook installation, and no active tmux server.
- [x] No regression in the existing `amux.tmux` TPM entrypoint or `prefix + A` picker binding.

## Review
- [x] Code reviewed.
- [x] PLAN.md updated if approach changed during implementation.
- [x] All phase commits are clean and describe their intent.
- [x] TODO.md items all checked off.
