<!-- markdownlint-disable MD013 -->

# Plan 004: Keep private settings out of hook dry-run output

> **Executor instructions**: Follow every step and check. Stop on a STOP
> condition. Update `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat 4493b48..HEAD -- src/hooks.rs tests/rust_smoke.rs README.md`
> If current code does not match the excerpts, stop.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `4493b48`, 2026-08-06

## Why this matters

Hook installation preserves complete user settings documents. Dry-run mode
then prints the complete merged document. If unrelated settings contain a
credential or private value, it can enter terminal history or CI logs.

## Current state

- `src/hooks.rs:559-596` merges the amux fragment into the complete JSON value.
- `src/hooks.rs:692-702` sends the full serialized value to `write_text`, which
  prints it in dry-run mode:

```rust
println!("would update {name}: {}", path.display());
print!("{text}");
```

- `README.md:215-223` describes dry run as a preview.
- Tests use isolated `HOME` directories in `tests/rust_smoke.rs` and
  `src/hooks.rs`. Match that pattern and never report the test credential value.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Hook tests | `cargo test hooks::tests --all-features` | all pass |
| Integration tests | `cargo test --test rust_smoke --all-features` | all pass |
| Full tests | `cargo test --all-features` | all pass |
| Format | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:

- `src/hooks.rs`
- `tests/rust_smoke.rs`
- `README.md`
- `plans/README.md`

**Out of scope**:

- The merge result written by `--write`.
- Backup behavior, file permissions, and symlink behavior.
- Hook event mappings and launcher paths.
- Any read of real files outside the isolated test home.

## Git workflow

- Branch: `advisor/004-redact-hook-dry-run`
- Commit message: `fix(hooks): redact dry-run settings output`
- Do not push or open a pull request unless asked.

## Steps

### Step 1: Separate operation output from file content

Change dry-run output so `write_text` never prints the complete destination
content. It can print the operation, integration name, and destination path.
For JSON hooks, add a short owned summary such as event names added or removed.
For text assets, print that the owned asset would be replaced. Do not print any
existing unrelated value.

Use a writer parameter or returned preview records so tests can capture output
without global stdout tricks. Keep the normal CLI text stable where possible.

**Verify**: `cargo test hooks::tests --all-features` → all pass.

### Step 2: Add a privacy regression test

In an isolated test home, create a valid agent settings JSON document with one
unrelated credential-type field. Run `install-hooks --dry-run`, capture stdout
and stderr, and assert:

- the command succeeds;
- the adapter and file path are present;
- the private field name and its value are absent;
- no file changed.

Do not put a real credential in the fixture.

**Verify**: run the named integration test; it passes.

### Step 3: Document the preview contract

Update README dry-run text: it reports owned operations and paths, and it does
not print complete existing settings.

**Verify**: `rg -n 'dry-run' README.md` → the new contract is present.

### Step 4: Run full verification

Run all commands in the command table.

## Test plan

- Keep the existing read-only dry-run test.
- Add output privacy checks for one JSON merge and Pi settings registration.
- Confirm text-asset dry run still names the owned file.
- Confirm `--write` produces the same merged documents as before.

## Done criteria

- [ ] Dry-run output contains no complete settings document.
- [ ] Unrelated field names and values are absent from captured output.
- [ ] Dry run changes no file.
- [ ] Write mode output files are unchanged from the current expected result.
- [ ] Full tests pass.
- [ ] Only in-scope files changed.
- [ ] The index status is `DONE`.

## STOP conditions

- The fix requires removal of useful owned event names from the preview.
- Any test needs a real credential or a real user home.
- Write mode produces a different hook document.
- Backup or symlink behavior changes.

## Maintenance notes

All future dry-run paths must describe changes without printing unrelated user
content. Treat complete user configuration as private even when its schema does
not define credential fields.
