# Release Checklist

Use this checklist for each amux release. Keep all boxes clear in the
repository. Mark a box only after the named check passes for the release.

## Automated checks

- [ ] Run `mise install`.
- [ ] Run `mise run audit`. Confirm that Cargo.lock has no RustSec advisory.
- [ ] Run `hk check --all`.
- [ ] Run `cargo test --all-features` again if the hk output does not show the
  native Rust test result for the release host.
- [ ] Run `bash tests/smoke.sh`.
- [ ] Run `bash tests/tpm-bootstrap.sh`.
- [ ] Run the hook dry-run and write tests in an isolated test home.
- [ ] Run `bin/amux doctor` after a synthetic hook event in tmux.
- [ ] Open the picker and verify its navigation and pane switch action.

## Packages and attestations

- [ ] Confirm that CI builds these archives:
  - `aarch64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu`
- [ ] Confirm that each archive contains `bin/amux`, `bin/amux-rs`,
  `amux.tmux`, hook assets, runtime scripts, and license notices.
- [ ] Verify each archive with this command:

  ```bash
  gh attestation verify <archive> --repo TudorAndrei/amux
  ```

- [ ] Confirm that TPM can install the published archive for a supported host.
- [ ] Confirm that the canonical runtime is
  `~/.tmux/plugins/amux/bin/amux`.
- [ ] Confirm that an installed old daemon exits and the next command starts
  the new daemon.

## Product checks

- [ ] Run `bin/amux install-hooks --dry-run`. Confirm that it changes no file
  and prints no complete user settings.
- [ ] Run `bin/amux install-hooks --write` in an isolated test home. Confirm
  the Codex, Claude, Pi, and opencode event mappings.
- [ ] Confirm that `bin/amux doctor` reports compatible state, a private daemon
  socket, a healthy monitor, and current hooks.
- [ ] Confirm that `prefix + A` opens the picker and stays open with no tracked
  agent.
- [ ] Confirm that attention, running, done, and offline indicators are correct
  in the picker.

## Publication

- [ ] Confirm that all commits after the last tag use Conventional Commits.
- [ ] Merge the release change to `main`.
- [ ] Confirm that Cocogitto creates the version change, changelog, and tag.
- [ ] Confirm that CI publishes all three attested archives.
- [ ] Confirm that the release notes and archive names use the new version.
