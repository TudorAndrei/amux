<!-- markdownlint-disable MD013 -->

# Use Lossless Tmux Field Encoding

> **Executor instructions:** Read the current tmux query formats and parsers
> before implementation. Run every check and update this plan's row in
> `plans/README.md` when done. Confirm the minimum supported tmux version. Stop
> if quoted format expansion is not compatible with that version.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- src/tmux.rs Cargo.toml Cargo.lock THIRD_PARTY_NOTICES.md tests/rust_smoke.rs`
> If an in-scope file changed, compare this plan with the live code. A material
> mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Medium
- Risk: Medium
- Depends on: None
- Category: Correctness, dependency
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

Tmux query output uses a fixed literal as a field separator. A session name, pane title, or path can contain that literal. The parser can then assign data to the wrong field or reject valid topology.

Tmux can quote format values. A shell-word decoder can then recover each field without a collision-prone separator.

## Current State

- `src/tmux.rs:40-41` defines the fixed separator.
- `src/tmux.rs:55-60` builds query formats with it.
- `src/tmux.rs:173-188` splits output on the literal.
- `Cargo.toml` does not include a shell-word decoder.
- `THIRD_PARTY_NOTICES.md` is generated from dependency metadata.

## Scope

In scope:

- `src/tmux.rs`
- Tmux parser unit tests
- `tests/rust_smoke.rs`
- `Cargo.toml`
- `Cargo.lock`
- `THIRD_PARTY_NOTICES.md`
- `plans/README.md`

Out of scope:

- Tmux control-mode protocol changes
- A shell command executor
- Tmux versions older than the project minimum
- A general serialization format

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
fix: decode tmux fields without separator collisions
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Add one parsing dependency

Add this direct dependency:

```toml
shell-words = "1.1.1"
```

Update `Cargo.lock`. Regenerate `THIRD_PARTY_NOTICES.md` with the project license task. Do not edit generated notices by hand.

Verify:

```sh
mise run licenses
mise run licenses-check
```

### 2. Change tmux formats to keyed quoted fields

Replace separator-based output with ordered keyed fields. Use tmux quoted expansion for each value, for example:

```text
session_id=#{q:session_id} session_name=#{q:session_name}
```

Apply the same scheme to every direct and control query that uses the fixed separator. Keep field names explicit and unique.

Do not pass the decoded text through a shell.

Verify:

```sh
rg 'FIELD_SEPARATOR' src/tmux.rs
```

This command must return no active separator use after the change.

### 3. Parse and validate exact fields

Use `shell_words::split` on one tmux output record. Require the expected number, order, and key prefix for the selected query. Strip each key prefix only after validation.

Return a clear parse error for:

- A missing field.
- A duplicate or unexpected key.
- Invalid quoting.
- Extra unparsed data.

Keep parsing in pure helper functions so tests do not need tmux.

Verify:

```sh
cargo test tmux::tests --all-features
```

### 4. Add collision and real-tmux tests

Add pure parser tests for values with:

- The old separator literal.
- Spaces.
- Single and double quote characters.
- Backslashes.
- Empty values.

If tmux permits it, include a newline value. If it does not, record that constraint in the test name or a short comment.

Add an isolated tmux smoke test that sets a pane title or session name to a value that contains the old separator. Confirm that topology reports the exact value and correct adjacent fields.

Verify:

```sh
cargo test --test rust_smoke tmux
```

## Test Plan

Run:

```sh
cargo test --all-features
bash tests/smoke.sh
bash tests/tpm-bootstrap.sh
mise run licenses-check
hk check --all
```

Run the real-tmux test on both macOS and Linux in CI.

## Done Criteria

- No tmux query depends on a fixed field separator.
- The parser validates named fields and decodes quoted values.
- Values with the old separator survive unchanged.
- The dependency and license notices are pinned and current.
- macOS and Linux tests pass.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- `#{q:...}` does not give compatible output on the minimum supported tmux version.
- Direct and control queries apply different escaping rules that one parser cannot decode.
- The license tool rejects the new dependency.

Do not replace the old separator with another random sentinel.

## Maintenance Notes

When a tmux query adds a field, add its key and parser test in the same change. Keep the output format private to the tmux adapter.
