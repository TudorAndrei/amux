<!-- markdownlint-disable MD013 -->

# Test Daemon Exit After Binary Replacement

> **Executor instructions:** Implement this plan after plan 002. Run every
> check and update this plan's row in `plans/README.md` when done. Read the
> production replacement check and existing real-daemon test helpers first.
> Do not kill processes by name.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- src/daemon.rs tests/rust_smoke.rs`
> Changes from plan 002 are expected. Compare them with this plan. A material
> mismatch is a stop condition.

## Status

- Priority: P2
- Effort: Medium
- Risk: Medium
- Depends on: `002-serialize-daemon-mutations.md`
- Category: Integration test, upgrade reliability
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

The daemon has production logic that detects replacement of its executable and exits. Only the metadata comparison helper has unit coverage. There is no integration test that starts a real daemon, replaces its executable, and proves that a new daemon can take ownership.

This behavior is important for TPM runtime upgrades.

## Current State

- `src/daemon.rs:135-159` checks the running executable for replacement.
- `src/daemon.rs:470-487` tests only the helper logic.
- `tests/rust_smoke.rs:1547` and later lines start and stop a real daemon, but do not replace its binary.
- Real server tests use shared synchronization and temporary state paths. Reuse those conventions.

## Scope

In scope:

- `tests/rust_smoke.rs`
- `plans/README.md`

Out of scope:

- Production daemon changes
- Shorter production polling intervals
- In-place modification of the test runner binary
- A TPM network installation test

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
test: cover daemon exit after binary replacement
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Build an isolated executable path

In a new real-server integration test, take the existing real server test lock. Copy the current test amux executable to a temporary `bin/amux` path. Preserve executable permissions.

Start that copy with a temporary state directory. Wait for the daemon socket with the existing bounded helper. Keep the child handle in a cleanup guard so a failed assertion does not leave a process.

Do not use the canonical installed TPM binary in this test.

Verify:

```sh
cargo test --test rust_smoke daemon_exits_after_binary_replacement -- --nocapture
```

### 2. Replace the executable atomically

Create a second copy at a sibling temporary path. Confirm that its replacement metadata differs from the running file metadata. If the file system timestamp resolution is coarse, prepare the two files with a bounded wait or retry before daemon startup.

Rename the second copy over `bin/amux` atomically. Do not edit bytes in the running test binary. Do not remove a broad directory.

Wait with a bounded loop for the old daemon process to exit and for its socket ownership to end. Use the current production polling interval to select a timeout with margin, such as eight seconds.

### 3. Prove that the replacement can take ownership

Run a normal event command through the replacement executable. Confirm that it starts or reaches a new daemon and that a ping or list command succeeds.

Confirm that the new daemon has a different process instance or protocol revision observation as supported by existing helpers. Then stop it through the normal daemon shutdown command.

The test must fail if the old daemon stays alive and continues to own the socket.

### 4. Make cleanup exact

On every test exit:

- Stop only the child processes started by this test.
- Remove only the temporary socket and directory owned by this test.
- Release the real server test lock.

Do not use `pkill`, process-name matching, or user daemon paths.

Verify:

```sh
cargo test --test rust_smoke daemon_exits_after_binary_replacement -- --nocapture
cargo test --test rust_smoke daemon -- --nocapture
```

## Test Plan

Run:

```sh
cargo test --all-features
bash tests/smoke.sh
bash tests/tpm-bootstrap.sh
hk check --all
```

Run the new test on both macOS and Linux because executable replacement metadata can differ by file system.

## Done Criteria

- The test starts a daemon from an isolated copied executable.
- Atomic replacement makes the old daemon exit within a bounded time.
- A command through the replacement reaches a new daemon.
- Cleanup cannot affect an installed or user daemon.
- The test passes on macOS and Linux.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- The production daemon no longer checks its executable metadata.
- The current platform cannot atomically replace a running executable by rename.
- Existing test helpers cannot identify and stop only the test process.
- Plan 002 changes daemon startup or shutdown semantics.

Do not weaken the assertion to only test the metadata helper.

## Maintenance Notes

Keep this as an integration test of the complete upgrade behavior. If the replacement detector changes from metadata to another signal, update test setup but preserve the old-daemon exit and new-daemon ownership assertions.
