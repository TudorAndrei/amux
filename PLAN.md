# Plan: Integrated Nucleo Session Picker

## Goal

Replace the external `fzf` picker path with one integrated Ratatui picker backed
by Nucleo, while preserving live daemon updates, navigation, session switching,
status presentation, and deterministic `picker --rows` output. Search must rank
and filter exclusively on the tmux session name; status, reason, cwd, pane, age,
and agent details remain visible but must never contribute matches.

## Approach

- Add the high-level `nucleo` crate at the current `0.5` release in
  `Cargo.toml` and regenerate `Cargo.lock`. Use `Nucleo<String>` with exactly one
  matching column and one worker thread: amux normally has few sessions, so
  additional workers would add overhead without improving interaction. Confirm
  its MPL-2.0 notice/source obligations remain compatible with amux's MIT
  source and prebuilt release archives.
- Keep `App.sessions` in `src/ui.rs` as the authoritative, latest
  `SessionView` model. Store only the session name as each Nucleo candidate and
  fill only column zero from that name. This separation makes it structurally
  impossible for status text such as `notification`, the reason, cwd, pane, or
  rendered row text to satisfy a query.
- Replace the current `App::filtered` subsequence matcher with Nucleo's ranked
  snapshot. Query edits will call `MultiPattern::reparse` for column zero using
  case-insensitive matching and smart Unicode normalization. The event loop
  will call `Nucleo::tick(0)` so matching completion never blocks keyboard
  polling.
- On `App::replace`, restart the Nucleo input stream, inject the new set of
  session names, and retain the selected session by name when it is still
  present. A Nucleo snapshot may briefly describe the preceding revision, so
  rendering and Enter handling will resolve matched names against the current
  `App.sessions` collection and discard names that no longer exist.
- Remove `fzf_available`, `fzf_has_rows`, and `run_fzf` from `src/ui.rs`;
  `ui::run` will always launch the live native picker. Retain `rows`,
  `render_rows`, `picker --rows`, `Ctrl-R`, and `Command` usage because scripts,
  tests, and tmux switching still depend on those contracts.
- Keep the existing Ratatui layout and key bindings. Rename the query box title
  to `search session name` so the searchable scope is visible in the UI.
- Expand the `src/ui.rs` reducer tests around ranked results, async matcher
  ticks, query changes, and live replacements. In particular, construct rows
  whose status/reason contains `notification` and prove that a query for
  `noti` returns only sessions whose `session` field matches.
- Update `README.md` to document the single integrated picker, remove the
  conditional fzf behavior, and state explicitly that only session names are
  searched. Leave historical fzf entries in `CHANGELOG.md` and
  `docs/performance.md` unchanged.
- The existing uncommitted first-launch fix in `src/ui.rs` is part of the
  baseline and will be subsumed by removing the fzf branch; no unrelated
  worktree changes will be modified.

Out of scope: indexing files with FFF, searching status/reason/agent fields,
changing daemon IPC or `SessionView`, changing the `picker --rows` schema, or
redesigning the picker layout beyond clarifying the query label.

## Implementation Phases

### Phase 1: Replace fzf with the Nucleo-backed native picker

- Add `nucleo = "0.5"` to `Cargo.toml`, refresh `Cargo.lock`, and confirm the
  dependency supports the repository's Rust 1.96 baseline and all three release
  targets; record any release-notice action required by its MPL-2.0 license.
- Refactor `App`, `App::replace`, query input handling, selection movement, and
  `draw` in `src/ui.rs` around a one-column `Nucleo<String>` worker and ranked
  snapshots.
- Remove external fzf detection/spawning while preserving native startup,
  daemon subscriptions, terminal restoration, key bindings, target-pane
  switching, and `picker --rows`.
- Add unit regressions proving session-name-only matching, Nucleo ranking,
  empty/no-match behavior, first-match selection after query changes, and
  selection preservation across passive live updates.
- Update the picker documentation in `README.md` and clarify the query box title
  in the Ratatui UI.
- Run formatting, Clippy, all Rust and shell smoke tests, package verification,
  and manual tmux picker checks with `fzf` both present and absent from `PATH`.
- Compare the stripped release binary size with the current approximately
  1.1 MiB baseline and record the delta in the implementation handoff.

  **Commit:** `feat(picker): integrate nucleo session matching`

## Risks & Tradeoffs

- Nucleo computes results asynchronously, so a snapshot can lag a daemon update
  by a frame. Resolving matched names against the current `App.sessions` data
  prevents removed sessions or stale pane targets from being selected.
- Nucleo ranking will not be byte-for-byte identical to fzf or the current
  subsequence filter. Tests will lock down the required semantics—name-only
  matching, stable selection, and sensible best-first results—without depending
  on undocumented score values.
- The dependency adds Rayon and matching code to the release binary. Limiting
  the worker to one thread and measuring the stripped artifact makes the cost
  visible; if the size increase is disproportionate, using Nucleo's lower-level
  `nucleo-matcher` crate is the fallback design.
- Nucleo is MPL-2.0 while amux is MIT. The implementation will use the upstream
  crate without modifying its source and verify what notice or source reference
  should accompany binary archives; this plan does not assume that dependency
  licensing is automatically satisfied by `Cargo.lock`.
- Removing the optional fzf path changes familiar styling for users who have
  fzf installed. The integrated picker keeps amux's colors, help text, live
  details, and key bindings consistently available on every installation.
- Query syntax and Unicode normalization may differ at punctuation boundaries.
  Add cases for mixed case, Unicode session names, spaces, hyphens, and an empty
  query before considering the behavior stable.

## Open Questions

- None. This plan assumes the integrated Ratatui/Nucleo picker replaces fzf
  completely, as confirmed in the preceding discussion.

## Implementation Record

- Added Nucleo 0.5.0 with one worker and one matcher column. Cargo resolved the
  dependency set for the repository's Rust 1.96 baseline; the existing CI
  matrix builds macOS arm64, Linux x86_64, and Linux arm64 archives.
- Added `THIRD_PARTY_NOTICES.md` to release archives. It identifies Nucleo's
  MPL-2.0 license and its unmodified 0.5.0 source location, so executable
  recipients can obtain the covered source.
- Verification completed on 2026-07-24: `cargo fmt --check`, Clippy with
  warnings denied, all Rust tests, both shell smoke tests, `mise run check`,
  and `mise run package-check`. Isolated tmux checks opened the same native
  picker with `fzf` present and absent from `PATH`.
- Stripped release binary size changed from 1,090,016 bytes to 1,206,256 bytes:
  +116,240 bytes (+10.7%).
