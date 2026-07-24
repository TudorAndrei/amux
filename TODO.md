# TODO: Integrated Nucleo Session Picker

## Phase 1: Replace fzf with the Nucleo-backed native picker

- [x] Add `nucleo = "0.5"` to `Cargo.toml`, regenerate `Cargo.lock`, and verify
      Rust 1.96 plus macOS arm64 and Linux x86_64/arm64 compatibility.
- [x] Verify Nucleo's MPL-2.0 notice/source obligations for amux's MIT source
      and prebuilt release archives, and add any required release notice.
- [x] Refactor `src/ui.rs::App` to own a one-column, one-worker
      `Nucleo<String>` matcher while keeping the latest `SessionView` values as
      the authoritative display and selection data.
- [x] Inject only `SessionView.session` into Nucleo column zero and reparse only
      that column when the query changes.
- [x] Tick Nucleo without blocking the keyboard loop and render its ranked
      snapshot after discarding candidates absent from the latest session
      revision.
- [x] Preserve first-match selection after query edits and preserve the
      selected session by name across passive daemon revisions when it remains
      visible.
- [x] Remove `fzf_available`, `fzf_has_rows`, and `run_fzf`; always launch the
      live native picker while retaining `picker --rows`, Ctrl-R, Esc, Ctrl-C,
      navigation, Enter, and pane-target switching.
- [x] Rename the query box to `search session name`.
- [x] Update `README.md` to remove conditional fzf behavior and document
      session-name-only search.
- [x] Add focused `src/ui.rs` tests for Nucleo ranking, query changes, live
      replacements, empty/no-match snapshots, mixed case, Unicode, spaces, and
      hyphenated session names.
- [x] Add a regression where status and reason equal `notification` but the
      session name does not match `noti`; confirm the row is excluded.
- [x] Commit: `feat(picker): integrate nucleo session matching`

## Verification

- [x] `cargo fmt --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] `cargo test --all-features` passes, including the new Nucleo picker
      reducer tests in `src/ui.rs`.
- [x] `bash tests/smoke.sh` preserves the deterministic tab-separated
      `picker --rows` contract and existing multi-agent target-pane behavior.
- [x] `bash tests/tpm-bootstrap.sh` still installs and launches the packaged
      native binary.
- [x] `mise run check` and `mise run package-check` pass.
- [x] Manual smoke test: load `amux.tmux`, open `prefix + A` immediately after
      daemon startup, and confirm the picker stays open until sessions arrive.
- [x] Manual smoke test: with sessions whose displayed reason/status contains
      `notification`, type `noti` and confirm only session names matching
      `noti` remain.
- [x] Manual smoke test: create, rename, and remove sessions while the picker is
      open; confirm ranked results update and Enter never targets a removed or
      stale pane.
- [x] Manual smoke test: run with `fzf` installed and with it absent from
      `PATH`; confirm the same integrated picker opens in both environments.
- [x] Edge cases tested: empty query, no matches, mixed case, Unicode, spaces,
      hyphens, backspace from a non-empty query, and a selected session removed
      by a live update.
- [x] No regressions in status colors, age/reason rendering, multi-agent detail,
      keyboard responsiveness, selection retention, or tmux pane switching.
- [x] Compare the stripped `target/release/amux-rs` size with the current
      approximately 1.1 MiB baseline and report the delta.

## Review

- [x] Code reviewed.
- [x] `PLAN.md` updated if the approach changed during implementation.
- [x] The phase commit is clean and uses the exact planned message.
- [x] `TODO.md` items are all checked off.
