<!-- markdownlint-disable MD013 -->

# Match Owned Hook Commands Exactly

> **Executor instructions:** Implement this plan after plan 004. Run every
> check and update this plan's row in `plans/README.md` when done. Read the
> current hook rendering and removal code first. Preserve foreign hook entries.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- src/hooks.rs tests/rust_smoke.rs`
> Changes from plan 004 are expected. Compare them with this plan. A material
> mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Medium
- Risk: Medium
- Depends on: `004-redact-hook-dry-run.md`
- Category: Correctness, configuration safety
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

Hook cleanup uses a substring search for the amux command. A foreign hook command that only mentions the same text can look like an amux-owned command. Install or uninstall can then remove configuration that amux does not own.

Ownership must depend on the command structure, not on a substring.

## Current State

- `src/hooks.rs:356-358` identifies amux commands with a substring match.
- `src/hooks.rs:657-669` removes entries that match this check.
- Hook rendering already has a quoting convention for the executable path.

## Scope

In scope:

- `src/hooks.rs`
- Hook unit tests
- Hook install and uninstall smoke tests in `tests/rust_smoke.rs` if needed
- `plans/README.md`

Out of scope:

- Execution of hook commands
- A general shell parser
- Changes to event names or agent names
- Removal of unknown legacy commands

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
fix: match owned hook commands exactly
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Define the owned invocation shape

Create one small parser or predicate for hook ownership. It must accept only a command with this structure:

1. One executable token.
2. An absolute executable path that ends with `/bin/amux`.
3. The literal subcommand `event`.
4. The literal option `--agent` and one supported agent name.
5. Only the remaining arguments that the current hook catalog renders for that event.

Support the current quoted executable token and the known legacy unquoted absolute path when the path has no whitespace. Reuse the renderer quoting rules where possible.

Do not run a shell. Do not use `eval`. Treat malformed quoting as foreign configuration.

Verify:

```sh
cargo test hooks::tests owned_command --all-features
```

### 2. Use structural ownership for drift and cleanup

Replace substring ownership checks in hook drift detection, install replacement, and uninstall removal with the new predicate.

Compare event mapping and command arguments after ownership is established. Preserve a foreign entry even if it contains the text `bin/amux event --agent` in an echo command, comment, environment value, or later argument.

The canonical TPM path `~/.tmux/plugins/amux/bin/amux` is expected. Do not report it as drift only because it differs from the inspected source checkout.

Verify:

```sh
cargo test hooks::tests --all-features
```

### 3. Add preservation tests

Add tests for:

- A normal rendered amux command is owned.
- The canonical TPM command is owned.
- A quoted path with whitespace is owned.
- A quoted path with an apostrophe is owned if the renderer can produce it.
- A known legacy unquoted path is owned.
- `echo '~/.tmux/plugins/amux/bin/amux event --agent codex'` is foreign.
- A wrapper command that has the amux text as a later argument is foreign.
- A malformed quoted command is foreign.
- Install and uninstall preserve all foreign entries byte for byte.

Use synthetic values. Do not use real user hook files.

Verify:

```sh
cargo test --test rust_smoke hooks
```

## Test Plan

Run:

```sh
cargo test --all-features
bash tests/smoke.sh
bash tests/tpm-bootstrap.sh
hk check --all
```

## Done Criteria

- Hook ownership uses parsed command structure.
- A foreign command cannot become owned only because it contains amux text.
- Current and known legacy rendered amux commands remain manageable.
- Install and uninstall preserve foreign hook entries.
- The canonical TPM runtime path is accepted as expected.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- The current hook command can contain shell pipelines or substitutions by design.
- The renderer has more than one executable command shape that this plan does not describe.
- A dependency is needed only to parse one controlled token.

Do not broaden ownership to match wrappers or aliases without an explicit compatibility requirement.

## Maintenance Notes

Keep rendering and ownership parsing close together. When hook arguments change, add a compatibility test before old owned entries are removed.
