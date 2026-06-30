# Release Checklist

## 0.1.0

- [ ] `make check` passes.
- [ ] `scripts/install-hooks.sh --dry-run` is read-only.
- [ ] `scripts/install-hooks.sh --write` installs Codex, Claude, opencode, and Pi hooks with backups.
- [ ] Loading amux through TPM succeeds in tmux.
- [ ] `prefix + A` opens the picker and stays open when no agents are tracked.
- [ ] Synthetic events show `▲` attention state in the tmux status segment.
- [ ] Tag `v0.1.0`.
