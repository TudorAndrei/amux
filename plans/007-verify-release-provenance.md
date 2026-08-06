<!-- markdownlint-disable MD013 -->

# Attest and Verify Release Archives

> **Executor instructions:** Read the current workflow and bootstrap scripts
> before implementation. Run every check and update this plan's row in
> `plans/README.md` when done. Resolve every action tag to a verified full
> commit SHA. Do not use a floating action tag.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- .github/workflows/ci.yml scripts/ensure-runtime.sh tests/tpm-bootstrap.sh README.md docs/updating.md`
> If an in-scope file changed, compare this plan with the live code. A material
> mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Medium
- Risk: High
- Depends on: None
- Category: Supply-chain security
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

The runtime bootstrap downloads a release archive and installs it without a provenance check. A compromised release asset or release process can therefore install an untrusted executable.

The release workflow must attest each archive. The bootstrap must verify the attestation before extraction or installation.

Official references:

- [GitHub artifact attestation guide](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
- [Official actions/attest repository](https://github.com/actions/attest)

## Current State

- `scripts/ensure-runtime.sh:40-54` downloads, extracts, and installs the archive.
- `.github/workflows/ci.yml:123` and later lines build and publish release archives.
- `tests/tpm-bootstrap.sh` provides fake release download coverage.
- The workflow pins existing actions with full commit SHAs. Preserve this convention.

## Scope

In scope:

- `.github/workflows/ci.yml`
- `scripts/ensure-runtime.sh`
- `tests/tpm-bootstrap.sh`
- `README.md`
- `docs/updating.md`
- `plans/README.md`

Out of scope:

- Package manager distribution
- A new signing service
- Key-managed manual signatures
- Source archive attestations that GitHub creates automatically

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
security: verify release archive provenance
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Attest each built archive

In the release package job, grant only the permissions that artifact attestation needs:

- `contents: read`
- `id-token: write`
- `attestations: write`

After all platform archives exist and before release publication, use the official `actions/attest` action at major version 4. Resolve the current v4 tag to its verified full commit SHA and pin that SHA. Set `subject-path` to the exact `dist/*.tar.gz` release archives.

Keep build and release jobs separate if they are separate now. Make sure the attestation job has the archives that it names.

Verify:

```sh
actionlint .github/workflows/ci.yml
rg 'actions/attest@[0-9a-f]{40}' .github/workflows/ci.yml
```

### 2. Verify before extraction

In `scripts/ensure-runtime.sh`, run this check after download and before extraction:

```sh
gh attestation verify "$archive_path" --repo TudorAndrei/amux
```

Use the actual local archive path variable. Pass arguments as separate shell words. Do not use `eval`.

Fail closed if verification fails or if the installed `gh` cannot run the command. Do not extract, copy, or replace a runtime before successful verification.

Verify:

```sh
bash -n scripts/ensure-runtime.sh
```

### 3. Extend bootstrap tests

Extend the fake `gh` in `tests/tpm-bootstrap.sh` to support both operations:

- `release download`
- `attestation verify`

Record their order. Test that verification occurs after download and before extraction or installation.

Add a failure case in which attestation verification returns a nonzero status. Assert that:

- Bootstrap fails.
- No runtime is installed or replaced.
- No extracted untrusted binary is executed.

Do not contact GitHub in these tests.

Verify:

```sh
bash tests/tpm-bootstrap.sh
```

### 4. Document the requirement

Update `README.md` and `docs/updating.md` to state that bootstrap requires a GitHub CLI version that supports artifact attestation verification. State that release archives are verified before installation.

Do not promise protection for manually copied binaries.

Verify:

```sh
markdownlint README.md docs/updating.md
```

## Test Plan

Run:

```sh
actionlint .github/workflows/ci.yml
bash tests/tpm-bootstrap.sh
bash tests/smoke.sh
hk check --all
```

On a test release or a workflow dispatch that cannot publish, also inspect the action log and confirm that every archive is an attestation subject.

## Done Criteria

- Every published platform archive has a GitHub artifact attestation.
- The attestation action uses a verified full commit SHA.
- Bootstrap verifies the expected repository identity before extraction.
- Verification failure leaves the installed runtime unchanged.
- Tests prove the download, verify, and install order.
- User documentation states the GitHub CLI requirement.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- The workflow cannot receive GitHub OIDC and attestation permissions.
- The release assets move to a repository other than `TudorAndrei/amux`.
- The selected official action version does not support the repository visibility or plan.
- Bootstrap must support an environment without `gh attestation verify`.

Do not replace provenance verification with only a checksum from the same release.

## Maintenance Notes

When the attestation action changes major version, verify its publisher and source, then update the full SHA. Keep the repository identity explicit in bootstrap.
