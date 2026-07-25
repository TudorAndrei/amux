# Known Issues

## Linux CI: tmux control-monitor subscription closes

- Status: fixed
- Affected platforms: `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`

The `control_monitor_reconciles_an_isolated_tmux_server` integration test failed
on both Ubuntu 24.04 CI runners with:

```text
thread 'control_monitor_reconciles_an_isolated_tmux_server' panicked at tests/rust_smoke.rs:98:17:
tmux monitor subscription closed
```

### Root cause

A daemon connection thread keeps two handles to one socket: `stream` for
writing and a `try_clone()` of it for reading. `try_clone()` duplicates the
descriptor, so both handles share a single open file description — and
`O_NONBLOCK` lives on that description, not on the descriptor.

The subscription loop called `set_nonblocking(true)` on the read handle to poll
for a disconnected subscriber. That also made every broadcast non-blocking.
Once the first topology update grew past what the socket could take in one
write, `write_all` failed mid-record with `EWOULDBLOCK` (`os error 11`), the
handler returned an error, and the connection closed — leaving the subscriber
with a truncated JSON line. Linux's smaller initial `AF_UNIX` send buffer made
this reproduce on every run there; macOS accepted the same record whole.

Reproduced in an `ubuntu:24.04` container (tmux 3.4, aarch64), where the
instrumented test reported the truncated record and the daemon's `EWOULDBLOCK`
directly.

### What changed

- `src/daemon.rs` paces the subscription loop with a read timeout
  (`set_read_timeout`) instead of `set_nonblocking`. A read timeout is a socket
  option that applies to reads only, so the write half stays blocking.
- `reply()` serializes a response into memory and writes it with a single
  `write_all`, so a failed write cannot leave a half-decodable record.
- Socket timeouts are matched through a shared `would_block()` helper, which
  also accepts `TimedOut` (macOS) and `Interrupted`.
- The accept loop no longer terminates the daemon on `ConnectionAborted`.
- The daemon reports per-connection handler errors on stderr instead of
  discarding them.
- `tests/rust_smoke.rs` distinguishes EOF, socket read errors, and undecodable
  responses (including the received line), and captures daemon stderr for the
  panic message.
- Regression test: `daemon::tests::a_large_broadcast_reaches_the_subscriber_intact`
  asserts a broadcast larger than the socket buffer arrives as one intact line.

### Verification

- `mise run check` passes on macOS (tmux 3.6b).
- `cargo test --all-features` passes in an `ubuntu:24.04` container with
  tmux 3.4 on aarch64; the monitor test failed there before the change and
  passes after it.

### Local reproduction

Run the isolated integration test:

```sh
cargo test --test rust_smoke \
  control_monitor_reconciles_an_isolated_tmux_server -- --nocapture
```

Run the same verification command used by CI:

```sh
mise run check
```

Reproduce a Linux runner from macOS:

```sh
docker run --rm -v "$PWD:/repo:ro" ubuntu:24.04 bash -c '
  apt-get update -qq && apt-get install -y -qq tmux curl build-essential
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
  cp -r /repo /work && cd /work && cargo test --all-features'
```
