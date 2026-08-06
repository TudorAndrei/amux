<!-- markdownlint-disable MD013 -->

# Bound Daemon Client Resources

> **Executor instructions:** Implement this plan only after plan 002 is
> complete. Run every check and update this plan's row in `plans/README.md`
> when done. Stop if the daemon concurrency model has changed.
>
> **Drift check (run first):**
> `git diff --stat 4493b48..HEAD -- src/daemon.rs src/ipc.rs tests/rust_smoke.rs`
> Changes from plan 002 are expected. Compare them with this plan. A material
> mismatch is a stop condition.

## Status

- Priority: P1
- Effort: Medium
- Risk: High
- Depends on: `002-serialize-daemon-mutations.md`
- Category: Reliability, resource safety
- Planned at: `4493b48` on 2026-08-06

## Why This Change Is Needed

The daemon starts one thread for each accepted client. A client can connect and send no request. The initial read has no timeout. Many silent clients can therefore hold an unbounded number of threads and file descriptors.

The daemon needs a short initial request timeout and a fixed client limit. Normal subscriptions must continue to work.

## Current State

- `src/daemon.rs:162-179` starts a thread for each accepted client.
- `src/daemon.rs:200-218` reads the first request.
- `src/ipc.rs:250` does not apply an initial read timeout.

The daemon already uses timeouts for some later protocol work. Follow those error and logging conventions.

## Scope

In scope:

- `src/daemon.rs`
- `src/ipc.rs` only if a small timeout helper is necessary
- Daemon unit tests
- `tests/rust_smoke.rs`
- `plans/README.md`

Out of scope:

- An asynchronous runtime
- A worker pool rewrite
- Per-user operating system limits
- A new wire protocol

## Git Workflow

Create one focused commit after all checks pass. Suggested commit:

```text
fix: bound daemon client resources
```

Do not include unrelated worktree changes.

## Implementation Steps

### 1. Add an initial request deadline

Set a read timeout immediately after the daemon accepts a stream and before it reads the first request. Use a named constant with a short duration, such as two seconds.

After a valid request is decoded, change the timeout to the value that the selected operation needs. A subscription must not keep the initial request timeout.

Treat a timeout as a client protocol failure. It must not stop the daemon.

Verify:

```sh
cargo test daemon::tests --all-features
```

### 2. Add a fixed active-client limit

Add a small RAII permit type that counts active handler threads. Use a process-local atomic counter or a mutex-protected counter. The permit must release the count on every return and panic unwind.

Set the production limit to 64 active clients. Count long subscriptions in this limit.

When the limit is full, send a short rejection if possible and close the stream. Use the existing rejection format and a short write timeout. Do not run the command through the direct persistence fallback. A saturated daemon is still the owner of its state.

Keep the limit configurable inside tests so that a test can use a value of two or three.

Verify:

```sh
cargo test daemon::tests client_limit --all-features
```

### 3. Add deterministic resource tests

Add tests for these cases:

- A client connects and sends no bytes. The handler exits after the request deadline.
- The configured active-client limit is never exceeded.
- A client above the limit receives a rejection or a closed connection.
- A permit returns after a malformed request and after a normal request.
- A valid subscription remains active after the initial request deadline.

Use barriers or channels to hold test clients. Do not use sleep calls as the main synchronization method.

Verify:

```sh
cargo test daemon::tests --all-features
cargo test --test rust_smoke daemon
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

- A silent client cannot hold a handler forever.
- The daemon has no more than 64 active client handlers in production.
- Every handler path releases its permit.
- A valid subscription is not closed by the initial request timeout.
- Saturation does not cause direct state mutation outside the daemon.
- All project checks pass.

## Stop Conditions

Stop and revise this plan if:

- Plan 002 changes the handler ownership model.
- The protocol has an intentional request phase longer than the selected timeout.
- A fixed limit would count internal connections that cannot be rejected safely.

Do not add a retry loop that creates more threads.

## Maintenance Notes

Keep timeout and limit constants near the daemon protocol constants. If telemetry is added later, count accepted, timed-out, and rejected clients separately.
