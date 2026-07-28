# Known Issues

## Deferred work

All findings from the `improve` audit at `8cf1622` are resolved.

- Terminal rendering escapes C0/C1 characters visibly in every text sink.
- State writes use a kernel-released advisory lock; `AMUX_LOCK=0` disables it
  for network filesystems where `flock` is unreliable.
- Hook configuration writes use a fsynced temporary file and atomic rename;
  launcher paths are shell quoted.
- Daemon requests are limited to 1 MiB before deserialization.
- Picker terminal cleanup is guarded by RAII.
- Invalid stale, retention, and compaction environment overrides fall back to
  documented defaults and are reported by `amux doctor`.
- Conventional-commit verification runs in the local check task and release
  preparation path.
- Projection measurement at 100 sessions / 200 panes / 200 records took
  1.47 ms in the Rust unit-test build on Darwin arm64 (2026-07-28), below the
  5 ms indexing threshold. Indexing is REJECTED as premature.
