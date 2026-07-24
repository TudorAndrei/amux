# Release Checklist

## 0.1.0

- [ ] `mise run check` passes.
- [ ] `bin/amux install-hooks --dry-run` is read-only.
- [ ] `bin/amux install-hooks --write` installs Codex, Claude, opencode, and
  Pi hooks with backups.
- [ ] TPM installation (`prefix + I`) downloads the matching native release and
  loading amux succeeds in tmux.
- [ ] `prefix + A` opens the picker and stays open when no agents are tracked.
- [ ] Synthetic events show `▲` attention state in the tmux status segment.
- [ ] CI artifacts contain `bin/amux`, `bin/amux-rs`, `amux.tmux`, and hooks for
  macOS arm64 and Linux x86_64/arm64.
- [ ] `bin/amux doctor` reports a compatible state, private daemon socket, and
  healthy monitor after a hook event in tmux.
- [ ] `mise run cog-check` passes from the latest release tag.
- [ ] Merge conventional commits to `main`; Cocogitto creates the version bump,
  changelog, and tag, then CI builds and publishes three release archives.
