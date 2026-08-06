<!-- markdownlint-disable MD013 -->

# Update the Release Checklist

> **Executor instructions:** Implement this plan after plans 001, 007, and
> 010. Run every check and update this plan's row in `plans/README.md` when
> done. Compare the checklist with the current workflow, README, and runtime
> behavior before editing it.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- RELEASE.md README.md .github/workflows/ci.yml mise.toml`
> Changes from prerequisite plans are expected. Compare them with this plan. A
> material mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Small
- Risk: Low
- Depends on: `001-add-cargo-audit.md`, `007-verify-release-provenance.md`, `010-pin-cargo-edit-with-mise.md`
- Category: Documentation, release operations
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

`RELEASE.md` still names version 0.1.0 and asks for a status segment that the current README says was removed. It also does not include the planned security audit and release archive verification.

An obsolete checklist can make a correct release look incomplete and can omit required checks.

## Current State

- `RELEASE.md:3` names release 0.1.0.
- `RELEASE.md:12` asks for a status segment.
- `README.md:289-294` documents removal of the status segment.
- Release automation and project checks have changed since the checklist was written.

## Scope

In scope:

- `RELEASE.md`
- `plans/README.md`

Out of scope:

- Release workflow code
- Version changes
- A release publication
- Checking boxes for work that the executor did not perform

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
docs: update release checklist
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Make the checklist version-neutral

Replace the fixed 0.1.0 heading and text with a version placeholder or version-neutral wording. Keep all checklist boxes unchecked in the committed file.

Remove the obsolete status segment task. Do not add a compatibility task for a removed feature.

Verify:

```sh
! rg '0\.1\.0|status segment' RELEASE.md
```

### 2. Align required automated checks

List the current release checks in their execution order. Include:

- `mise install`
- `mise run audit`
- `hk check --all`
- Rust tests
- Shell smoke tests
- TPM bootstrap tests
- Hook install dry-run and write tests
- Doctor checks
- Tmux picker or topology smoke coverage
- Package checks for all supported targets

Do not duplicate commands that `hk check --all` already runs unless the separate command gives necessary release evidence. If there is duplication, state why it is run again.

### 3. Add release asset verification

Add post-build checks for:

- The expected archive for each supported target.
- Correct archive names and version.
- Artifact attestation verification for each archive.
- A bootstrap test against a published test or final release when the process permits it.

Use the repository identity `TudorAndrei/amux` in the example verification command.

Do not claim that an archive is verified before the command succeeds.

### 4. Align manual product checks

Keep only current product behavior. Include focused manual checks for:

- Hook event mapping.
- Picker navigation and action.
- Doctor output.
- Upgrade of the canonical TPM runtime at `~/.tmux/plugins/amux/bin/amux`.

Treat the canonical TPM path as expected. Do not call it install drift because it differs from a source checkout.

Verify:

```sh
markdownlint RELEASE.md
```

## Test Plan

Run:

```sh
markdownlint RELEASE.md
hk check --all
```

Review every checklist command against `mise.toml`, `.github/workflows/ci.yml`, and the named script before commit.

## Done Criteria

- The checklist has no fixed old version.
- The checklist has no status segment task.
- Security audit and archive attestation checks are present.
- All supported package targets are explicit.
- Manual checks describe current behavior and the canonical TPM runtime.
- No box is prechecked.
- Markdown and project checks pass.

## Stop Conditions

Stop and revise this plan if:

- A prerequisite plan changes its command or release process.
- The supported target set cannot be found in the workflow.
- The project intentionally restores the status segment.

Do not invent release approval or publishing steps that the repository does not use.

## Maintenance Notes

Review `RELEASE.md` in every release workflow change. Keep exact commands in one place when possible, and link to them from the checklist.
