<!-- markdownlint-disable MD013 -->

# Reject Unsupported State Versions Before Mutation

> **Executor instructions:** Follow this plan step by step. Run every check and
> update this plan's row in `plans/README.md` when done. Stop if the code has
> changed in a way that invalidates the evidence or the proposed interface.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- src/state.rs src/cli.rs tests/rust_smoke.rs`
> If an in-scope file changed, compare this plan with the live code. A material
> mismatch is a stop condition.

## Status

- Priority: P1
- Effort: Medium
- Risk: High
- Depends on: None
- Category: Correctness, data safety
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

The state loader accepts a state document with any version. Mutation commands can then write the document again as version 1. An old amux binary can therefore overwrite a state format that belongs to a newer binary.

The doctor command must still read enough of the document to report the unsupported version. Normal commands must reject it before they write any data.

## Current State

- `src/state.rs:12-21` defines the state version.
- `src/state.rs:145-178` loads and writes state without a version gate in the normal load path.
- `src/cli.rs:276-288` checks the version only in doctor output.

The current tests do not prove that an unsupported state file stays unchanged after a command fails.

## Scope

In scope:

- `src/state.rs`
- `src/cli.rs`
- State unit tests
- `tests/rust_smoke.rs`
- `plans/README.md`

Out of scope:

- A new state format
- State migration
- A change to version 1 data
- Recovery of a corrupt state file

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
fix: reject unsupported state versions before mutation
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Separate inspection from normal loading

Add a crate-private inspection load path in `src/state.rs`. It can deserialize the document and return its declared version without accepting it for normal use.

Keep the normal state load path strict. After deserialization, require the supported version. Return a clear error that includes both the found version and the supported version.

Do not silently change the version. Do not treat an unknown version as version 1.

Verify:

```sh
cargo test state::tests --all-features
```

### 2. Keep doctor diagnostic access

Change doctor in `src/cli.rs` to use the inspection load path. Doctor must report an unsupported version as a diagnostic result. It must not make the state valid for other commands.

All commands that can list, record, clear, compact, or start the daemon must use the strict load path.

Verify:

```sh
cargo test --test rust_smoke doctor
```

### 3. Add byte-preservation tests

Create a state file with a valid document shape and an unsupported version. Save its exact bytes before each test action.

Test these cases:

- A direct event command fails.
- A list command fails.
- Daemon startup fails before it creates or rewrites persistent state.
- Doctor reports the unsupported version.
- The state bytes are unchanged after every case.

Use a temporary state directory. Do not use a user state file.

Verify:

```sh
cargo test --test rust_smoke unsupported_state_version
```

## Test Plan

Run:

```sh
cargo test --all-features
bash tests/smoke.sh
hk check --all
```

## Done Criteria

- Normal state loading rejects every version other than the supported version.
- No mutation path can rewrite an unsupported state file.
- Doctor can report the unsupported version.
- Tests compare the state bytes before and after failure.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- A supported migration path already exists in the current branch.
- Some normal command must intentionally read an unsupported version.
- The state version is no longer stored in the document.

Do not add an automatic downgrade or a lossy migration.

## Maintenance Notes

When a new state version is added, keep inspection independent from acceptance. Add an explicit migration before normal commands accept the new version.
