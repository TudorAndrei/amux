<!-- markdownlint-disable MD013 -->

# Plan 001: Add a mise-managed Cargo advisory gate

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before the next step.
> If a STOP condition occurs, stop and report it. Do not improvise. When done,
> update this plan's row in `plans/README.md`.
>
> **Drift check (run first)**:
> `git diff --stat 4493b48..HEAD -- mise.toml hk.pkl`
> If an in-scope file changed, compare the excerpts below with the live code.
> A mismatch is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security / dependencies / dx
- **Planned at**: commit `4493b48`, 2026-08-06

## Why this matters

The locked Rust dependencies have no repository security-advisory gate.
`gitleaks` and the license check cover different risks. Add `cargo-audit` as a
fixed mise tool and make the normal project check fail on a RustSec advisory.

## Current state

- `mise.toml:1-12` pins Rust and all project tools, but it does not contain
  `cargo-audit`.
- `hk.pkl:9-21` defines source and secret checks.
- `hk.pkl:29-49` defines full verification, but it has no dependency audit.
- The current release of `cargo-audit` selected for this plan is `0.22.2`.
- The project uses mise tasks for named commands. For example:

```toml
[tasks.licenses-check]
description = "Reject unaccepted licenses and stale third-party notices"
run = '''
...
'''
```

The official tool documentation uses `cargo audit` against `Cargo.lock`:
<https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md>.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Install tools | `mise install` | exit 0 |
| Advisory check | `mise run audit` | exit 0; no vulnerability report |
| Rust tests | `cargo test --all-features` | 97 or more unit tests and 24 or more integration tests pass |
| Full check | `hk check --all` | exit 0 |

## Scope

**In scope**:

- `mise.toml`
- `hk.pkl`
- `plans/README.md` for the status update

**Out of scope**:

- `Cargo.toml` and `Cargo.lock`; do not update application dependencies here.
- Advisory ignore rules or `audit.toml`.
- GitHub issue or release publication.

## Git workflow

- Branch: `advisor/001-add-cargo-audit`
- Commit message: `ci(security): audit locked Rust dependencies`
- Do not push or open a pull request unless the operator asks.

## Steps

### Step 1: Pin cargo-audit in mise

Add this tool to `[tools]` in `mise.toml`, near the other Cargo tools:

```toml
"cargo:cargo-audit" = "0.22.2"
```

Add a named task:

```toml
[tasks.audit]
description = "Check Cargo.lock against RustSec advisories"
run = "cargo audit"
```

**Verify**: `mise tasks | rg '^audit\b'` → one `audit` task is shown.

### Step 2: Add the audit to the full hk gate

Add `security-audit` to the `verification` mapping in `hk.pkl`. Use
`check = "mise run audit"`. Make it `exclusive = true` because the advisory
database and Cargo cache are shared resources. Do not put it in `pre-commit`.
The existing `pre-push` and `check` maps already include all verification
steps.

**Verify**: `rg -n 'security-audit|mise run audit' hk.pkl` → the new step and
command are shown once.

### Step 3: Run the new gate

Run `mise install`, then `mise run audit`.

**Verify**: `mise run audit` → exit 0 with no vulnerable locked package.

If it reports an advisory, stop. Do not add an ignore and do not update a
dependency in this plan.

### Step 4: Run project verification

Run `cargo test --all-features`, then `hk check --all`.

**Verify**: both commands exit 0.

## Test plan

- No Rust test is required for tool configuration.
- Confirm `mise exec -- cargo audit --version` prints `cargo-audit 0.22.2`.
- Confirm `hk check --all` executes the named `security-audit` step.

## Done criteria

- [ ] `mise exec -- cargo audit --version` reports `0.22.2`.
- [ ] `mise run audit` exits 0.
- [ ] `hk check --all` exits 0 and includes `security-audit`.
- [ ] `cargo test --all-features` exits 0.
- [ ] No advisory ignore was added.
- [ ] Only the in-scope files changed.
- [ ] The status row in `plans/README.md` is `DONE`.

## STOP conditions

- `cargo-audit` 0.22.2 cannot build with Rust 1.96.
- `cargo audit` reports any advisory or cannot update its advisory database.
- hk cannot make the step check-only and exclusive.
- An in-scope excerpt changed after commit `4493b48`.

## Maintenance notes

Review future `cargo-audit` updates like other build-tool updates. Do not ignore
an advisory only to make CI green. Record the reachability decision and a time
limit if a later plan must add an ignore.
