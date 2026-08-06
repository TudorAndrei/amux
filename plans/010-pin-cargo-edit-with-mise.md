<!-- markdownlint-disable MD013 -->

# Pin Cargo Edit With Mise

> **Executor instructions:** Implement this plan after plan 001 so changes to
> `mise.toml` do not conflict. Run every check and update this plan's row in
> `plans/README.md` when done. Read the current release job before editing it.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- mise.toml .github/workflows/ci.yml`
> Changes from plan 001 are expected. Compare them with this plan. A material
> mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Small
- Risk: Low
- Depends on: `001-add-cargo-audit.md`
- Category: Build reproducibility
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

The release workflow installs the latest cargo-edit during each run. A new cargo-edit release can change release behavior without a repository change. Other project tools use fixed versions through mise.

Cargo-edit must use the same pinned tool process.

## Current State

- `.github/workflows/ci.yml:60-61` runs `cargo install cargo-edit` without a version.
- `mise.toml` pins project tools.
- The Cocogitto release hook uses `cargo set-version`.
- The current cargo-edit release found during planning is `0.13.13`.

## Scope

In scope:

- `mise.toml`
- `.github/workflows/ci.yml`
- `plans/README.md`

Out of scope:

- A Cargo dependency update
- Cocogitto configuration changes
- Release version policy
- Other tool upgrades

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
build: pin cargo-edit with mise
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Pin cargo-edit

Add this tool to the existing mise tool table:

```toml
"cargo:cargo-edit" = "0.13.13"
```

Keep the existing table order convention.

Verify:

```sh
mise install
mise exec -- cargo set-version --version
```

Confirm that the command reports cargo-edit 0.13.13.

### 2. Remove the unpinned CI install

Delete the workflow step that runs `cargo install cargo-edit`. The existing mise setup must make `cargo set-version` available before Cocogitto needs it.

Add a small version verification step in the release preparation job if the tool is not otherwise run before a release mutation. The step must use `cargo set-version --version`.

Verify:

```sh
actionlint .github/workflows/ci.yml
! rg 'cargo install cargo-edit' .github/workflows/ci.yml
```

### 3. Verify the release tool path

Run the non-mutating project release checks. Confirm that Cocogitto can find the mise-provided command in the same environment that the workflow uses.

Verify:

```sh
cog check
hk check --all
```

Do not run a real version bump or publish a release.

## Test Plan

Run:

```sh
mise install
mise exec -- cargo set-version --version
actionlint .github/workflows/ci.yml
cog check
hk check --all
```

Inspect the next release preparation log and confirm that no network install of the latest cargo-edit occurs.

## Done Criteria

- `mise.toml` pins cargo-edit 0.13.13.
- CI does not run an unversioned cargo-edit installation.
- The release environment can run `cargo set-version`.
- Workflow validation and project checks pass.

## Stop Conditions

Stop and revise this plan if:

- The mise action does not install Cargo tools for the release job.
- Cargo-edit 0.13.13 does not support Rust 1.96 or the current manifest.
- The release hook uses a different binary than `cargo set-version`.

Do not add a second install method as a fallback.

## Maintenance Notes

Update cargo-edit through the normal tool update process. Review its release notes before a version change because it changes manifests during release work.
