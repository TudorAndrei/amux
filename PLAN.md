# Plan: Close the six minor review items

## Goal

Clear the six judgement-call items left open by the review at `402eac7`. None is
a defect on its own, but two of them (`Lock::drop`'s residual race and the
silent `AMUX_LOCK_TIMEOUT_SECONDS` fallback) turned out to be symptoms of the
`mkdir`-based lock rather than things worth patching, so the first phase
replaces that lock with std advisory file locking and closes both by deletion.
The rest are a misleading diagnostic message, a durability-and-permissions gap,
and a test whose name overpromises.

## Approach

The six items, as recorded in the review:

1. `Lock::drop` narrows a TOCTOU rather than eliminating it.
2. `draw_error_unwinds_through_terminal_restore_guard` tests the guard, not
   `run_native`'s use of it.
3. No directory fsync after the hook-config rename.
4. The temp file's `0o600` becomes the destination's mode.
5. An invalid `AMUX_LOCK_TIMEOUT_SECONDS` falls back silently.
6. `command_arguments` reports quoting-only drift as *"argument drift"*.

Items 1 and 5 are closed by Phase 1's rewrite, not by comments. Items 3 and 4
are the same function and land together. Item 6 needs a different data model
than first drafted. Item 2 is a rename.

### Phase 1 — advisory file locking replaces the mkdir lease

`std::fs::File::try_lock` is available (stabilised 1.89; the project pins 1.96).
It is `flock(2)` on Linux and macOS, so the lock belongs to the **open file
description** and the kernel releases it when the holder dies. That removes the
entire reason the heartbeat lease exists.

Two properties were verified empirically on this machine rather than assumed:

- **Threads in one process are excluded.** 8 threads × 50 rounds, each
  independently opening the lock file and calling `lock()`, produced 0
  mutual-exclusion violations. This matters because the daemon serves
  connections on threads and they all reach `write_event`.
- **Re-locking the *same* handle succeeds silently** — no exclusion. That is the
  footgun, and it dictates the design: every acquisition must open its own fresh
  `File` and hold it for the critical section. Never share, clone, or cache one.

Design consequences:

- `acquire` and `Lock::new` merge. `acquire` opens the lock file, retries
  `try_lock` on the existing `lock_acquire_timeout_ms` budget, and returns a
  guard owning that `File`. The current split cannot survive — the guard must
  own the handle whose drop releases the lock.
- The lock file lives at a stable path and is **never deleted** on release.
  Deleting it lets two processes lock different inodes.
- **Migration.** `Config::lock_dir()` is `state.lock` and is a *directory* today;
  opening that path as a file fails. `ensure_private_dir` removes a `state.lock`
  that is a directory, which can only be a leftover of the old scheme.
- `lock_timeout_seconds`, `AMUX_LOCK_TIMEOUT_SECONDS`, the heartbeat thread, its
  token, the marker file, and the stale-takeover branch in `acquire` all go.
- `lock_acquire_timeout_ms` **stays**. std has no timed lock, so the
  `try_lock` + 10 ms sleep retry loop remains, and it is what bounds the waiter.
- **Unsupported filesystems degrade to today's behaviour.** `ENOTSUP`,
  `EOPNOTSUPP`, and `ENOSYS` from `try_lock` are treated as "acquired" and the
  write proceeds unlocked, rather than failing the hook. This follows cargo,
  which documents the posture as *"intended to provide a graceful fallback
  instead of refusing to work"*.

  Detecting them on bare std is less tidy than it looks. Measured on macOS:
  `ENOSYS` (78) maps to `io::ErrorKind::Unsupported`, but `ENOTSUP`/`EOPNOTSUPP`
  (45) maps to `Uncategorized`. So matching on `ErrorKind` alone misses the main
  case, and the check must also compare `raw_os_error()` against `cfg`-gated
  numeric constants (45 on macOS, 95 on Linux). Roughly six lines with a comment
  recording why; verify the Linux mapping on CI, and match both the kind and the
  numbers so the check holds either way.
- **`AMUX_LOCK=0` disables locking outright**, reported by `amux doctor`. This
  is the escape hatch for the NFS case below.

#### Why std and not a crate

The crates.io field for file locking splits cleanly on one property — whether
the lock belongs to the **open file description** (`flock`) or to the
**process** (`fcntl` record locks). Only the first excludes threads within one
process, which is mandatory here because the daemon serves every connection on
its own thread and they all reach `write_event`.

Ruled out on semantics: `file-lock`, `file-guard`, and `file-locker` all use
`fcntl`, so two daemon threads would not exclude each other — strictly worse
than the `mkdir` lock being replaced. Popularity is no guide: `file-guard` has
~2.5M recent downloads and is wrong for this use. `fslock` documents that
same-process multi-handle behaviour diverges by platform without its `multilock`
feature, which is precisely our case. `pidlock` is the pidfile approach being
abandoned.

Ruled out on fitness: `cluFlock` (untouched 4+ years), `fs2` (dead since 2018),
`async-fd-lock` (async; amux is sync).

That leaves `fd-lock` — `flock` via `rustix`, ~12.4M recent downloads, and a
genuinely good crate — versus std. They issue the same syscall. `fd-lock` buys
only typed `Errno` for the unsupported-filesystem check above, which does not
justify a dependency; cargo migrated *off* crates onto std for the same reason.

#### Precedent

[cargo's `util/flock.rs`](https://doc.rust-lang.org/stable/nightly-rustc/src/cargo/util/flock.rs.html)
is the closest analogue — same language, same std API — and independently
arrives at three of the choices above: never delete the lock file (`Drop` calls
`unlock()`, the file stays), try non-blocking first, and swallow the
unsupported-filesystem errno family. It diverges by then blocking
*indefinitely* with a `Blocking waiting for file lock` message, which suits an
interactive build and not a hook that must finish inside the agent's
`timeout: 5`; amux keeps the bounded retry and errors out instead.

Cargo also skips locking on NFS entirely, because *"the failure mode for `flock`
on NFS is blocking forever, even if the 'non-blocking' flag is passed"*. amux
does **not** replicate that detection — see Risks.

**Behaviour change worth naming:** today a contender can take over from a live
process wedged inside the critical section. With flock there is no takeover — a
hung-but-alive holder blocks waiters until their budget expires and they error
out. For a status cache, one failed hook event beats two concurrent writers, but
it is a real change. Deleting `AMUX_LOCK_TIMEOUT_SECONDS` is also a breaking
config change, so this lands as `feat!`.

### Phase 2 — the drift model has to be per-command, not per-group

The first draft proposed a `quoted: bool` on `OwnedHook`. That is the wrong
shape: `owned_hooks` (src/hooks.rs:110) collects **every** matching command in a
hook group into one sorted `Vec`, and `is_amux_command` (src/hooks.rs:146)
deliberately accepts both spellings, so one group can legally hold a quoted and
an unquoted command at once. No single bool represents that.

Worse, "compare `quoted` and `arguments` separately" is unsound given how
`drift_document` matches. It runs independent `any()` passes across all groups
for an event (src/hooks.rs:95-102), so an install with two amux groups on `Stop`
— one matching arguments but not quoting, the other matching quoting but not
arguments — would satisfy both passes and report **no drift**, though neither
group matches the template.

So: `OwnedHook.commands: Vec<OwnedCommand>` where
`OwnedCommand { arguments: String, quoted: bool }`, sorted by arguments and
preserving multiplicity so duplicate stale commands are not hidden. Matching
becomes **group-coherent** — for each expected hook, look for an installed group
that matches as a whole:

- a group whose matcher, arguments, and quoting all match → no drift;
- else a group whose arguments match ignoring quoting → `launcher quoting drift`;
- else, if the matcher differs → `matcher drift`;
- else → `argument drift`.

This also fixes the pre-existing incoherence where matcher, arguments, and
quoting could each be satisfied by a different group.

### Phase 3 — modes, symlinks, and durability in `write_text`

Preserving the destination's existing mode is the right call:
`~/.claude/settings.json` and `~/.codex/hooks.json` are shared files that other
tools read, and silently tightening them to `0600` is a side effect nobody asked
for. Files amux creates still get `0600`.

**Symlinked destinations are resolved and their target replaced**, so a
dotfiles-managed `~/.claude/settings.json` keeps its symlink. Today `fs::rename`
replaces the link itself with a regular file, silently detaching the dotfiles
repo.

Exact sequence, which the first draft left under-specified:

1. Resolve the destination through any symlink; all subsequent work targets the
   resolved path, and the temp file is created in **that** directory.
2. Read the existing mode if the file exists, masked with `0o777` so file-type
   bits and any setuid/setgid/sticky bits are not propagated onto a JSON file.
3. Create the temp file at `0o600`.
4. Write.
5. `set_permissions` to the final mode — **before** `sync_all`, so the
   permission change is inside the durability guarantee. `set_permissions` also
   sidesteps umask filtering, which `OpenOptions::mode` does not.
6. `sync_all`, `rename`, then fsync the parent directory.

`sync_dir` moves from `src/state.rs:347` to a new `src/fsutil.rs` shared by both
modules.

The permission test must use a mode the umask would otherwise strip — `0o664`,
not `0o644`. Under a typical `umask 022`, `OpenOptions::mode(0o644)` yields
`0o644` anyway, so a `0644` test passes even against a naive implementation that
threads the mode through `.mode()` instead of `set_permissions`. That test would
be vacuous.

### Phase 4 — rename the guard test only

The `drive()` extraction from the first draft is dropped. It adds no coverage —
the loop still needs a real terminal for `event::poll`/`event::read` — and
`run_native` is already short with the guard on the line before
`ratatui::init()`. Churn on code that just landed, for no property gained.

The rename alone is not quite enough either: the current test exits a block
normally carrying an `Err`, it never early-returns through `?`. Rewrite it
around a small fallible helper that genuinely uses `?`, so
`restore_guard_runs_on_early_return` is a truthful name.

### Phase 5 — item 5's sibling

Item 5 evaporates with `AMUX_LOCK_TIMEOUT_SECONDS` in Phase 1. The same defect
class survives one line away: an **unparseable** `AMUX_EVENTS_PER_SESSION`
(src/config.rs:41-44) still falls back silently, while `AMUX_STALE_SECONDS` and
`AMUX_EVENTS_COMPACT_BYTES` report. `0` stays meaningful there — it disables
compaction — so only a parse failure is rejected. Included because item 5 itself
disappears; cut this phase if that reads as scope creep.

### Out of scope

- `backup` (src/hooks.rs:454) uses `fs::copy` with no fsync, so the backup is
  less durable than the file it protects. Adjacent to item 3; deliberately left.
- `command_arguments` uses the *first* `" event "` occurrence, so a launcher
  path containing that substring would misparse. Pre-existing, untouched.
- Network filesystems: `AMUX_STATE_DIR` is user-settable and flock over macOS
  NFS is unreliable. Documented as a limitation, not handled.

## Implementation Phases

### Phase 1: Replace the mkdir lease with advisory file locking

- `src/state.rs`: merge `acquire` and `Lock::new` into one function returning a
  guard that owns the locked `File`; retry `try_lock` on the
  `lock_acquire_timeout_ms` budget.
- Delete the heartbeat thread, token, marker file, and the stale-takeover branch.
- `src/config.rs`: delete `lock_timeout_seconds` and `AMUX_LOCK_TIMEOUT_SECONDS`;
  keep `lock_acquire_timeout_ms`.
- `Config::lock_dir()` becomes `lock_file()` at the same `state.lock` path;
  `ensure_private_dir` removes a `state.lock` that is a directory.
- Comment the OFD assumption and the same-handle footgun at the acquisition site.
- Update `src/daemon.rs` and `src/sessions.rs` test fixtures for the dropped
  config field.
  **Commit:** `feat!: replace the state lock lease with advisory file locking`

### Phase 2: Name quoting drift for what it is

- `src/hooks.rs`: introduce `OwnedCommand { arguments, quoted }`; change
  `OwnedHook.arguments` to `commands: Vec<OwnedCommand>`, sorted by arguments,
  multiplicity preserved.
- `command_arguments` returns plain arguments again; quoting is derived per
  command from whether the launcher segment ends in `'`.
- `drift_document`: group-coherent matching with the four outcomes above.
  **Commit:** `fix(doctor): distinguish launcher quoting drift from argument drift`

### Phase 3: Preserve modes and symlinks, and fsync hook-config writes

- Add `src/fsutil.rs` with `pub(crate) fn sync_dir`; move it from `src/state.rs`
  and re-point callers.
- `src/hooks.rs::write_text`: resolve symlinks, preserve the masked existing
  mode, `set_permissions` before `sync_all`, rename, fsync the parent.
  **Commit:** `fix(hooks): preserve configuration modes and symlinks on atomic write`

### Phase 4: Make the restore-guard test truthful

- Rewrite `draw_error_unwinds_through_terminal_restore_guard` (src/ui.rs:475)
  around a fallible helper that early-returns through `?`, and rename it
  `restore_guard_runs_on_early_return`.
  **Commit:** `test(picker): prove the restore guard runs on early return`

### Phase 5: Report an unparseable events-per-session override

- `src/config.rs:41`: reject an unparseable `AMUX_EVENTS_PER_SESSION` and push
  it onto `rejected_overrides`; keep `0` meaningful.
  **Commit:** `fix(config): report an unparseable events-per-session override`

## Risks & Tradeoffs

- **No takeover from a wedged holder.** The single biggest behavioural change.
  A live process stuck in the critical section now blocks every waiter until
  their budget expires. Previously a contender could seize the lease after 30 s.
  Trading a rare "two concurrent writers" for a rare "one failed hook event" is
  the right direction, but it is a trade.
- **Mixed-version window during upgrade.** An old binary using the `mkdir` lock
  and a new one using flock do not exclude each other. The window is seconds and
  the worst case is one lost `state.json` update that the next hook rewrites.
  Name it in the commit body.
- **NFS can hang a hook, and this plan does not prevent it.** cargo skips
  locking on NFS because `flock` there blocks forever *even with the
  non-blocking flag*, which means `lock_acquire_timeout_ms` never gets a chance
  to expire — the hook would hang until the agent's own `timeout: 5` killed it.
  Replicating that detection needs `statfs` (a `libc` dependency) or parsing
  `/proc/self/mountinfo`, which has no macOS equivalent; probing with a watchdog
  thread costs a thread that may never join. All three are disproportionate for
  a state directory that is local by default. The mitigation is the `AMUX_LOCK=0`
  escape hatch plus a documented limitation — not a fix. Revisit if anyone
  actually points `AMUX_STATE_DIR` at a network mount.
- **flock is advisory**, so it binds only processes that ask for it. Fine here:
  amux is the sole writer of its own state directory.
- **Phase 2 changes `doctor` output strings.** Anyone grepping for
  `argument drift` on a pre-quoting install sees `launcher quoting drift`
  instead. Human diagnostic, exit code unchanged.
- **Phase 3 stops tightening permissions.** A `0644` `~/.claude/settings.json`
  stays `0644` instead of being quietly hardened to `0600`. Correct — amux does
  not own the file — but it removes accidental hardening someone may be relying
  on.
- **Symlink resolution is not atomic.** A link retargeted between resolution and
  rename would land the write on the new target. Vanishingly rare; not guarded.

## Open Questions

Resolved before implementation:

- Advisory file locking replaces the mkdir lease (items 1 and 5 close by
  deletion, not by comment).
- Symlinked hook-config destinations are resolved and their target replaced,
  preserving the link.
- Phase 4 is the test rewrite and rename only; the `drive()` extraction is
  dropped.

Still open:

- Should `AMUX_LOCK_TIMEOUT_SECONDS` be accepted-and-reported as a removed knob
  for one release rather than vanishing? Cleanly removing it is consistent with
  the `feat!` already shipped this cycle; a deprecation shim costs three lines
  and one `doctor` message.

Deferred to its own plan, not dismissed:

- **Should the storage layer become transactional?** Replacing `state.json` and
  `events.jsonl` with SQLite would subsume the lock entirely and close two
  things this plan does not. First, `write_event` appends to the log and
  rewrites the state as **two non-atomic operations** under one lock, so a crash
  between them leaves them disagreeing — low-stakes today only because
  `events.jsonl` is documented as a debugging record, not a rendering source.
  Second, and larger: the whole retention subsystem — swap-aside, off-lock
  streaming into per-key ring buffers, ordinal-preserving merge, `.compacting`
  orphan adoption — collapses into one `DELETE … WHERE rowid NOT IN (…)`, and
  its crash-recovery path disappears into transactions. That is where the
  residual complexity of this codebase lives.

  Not folded in here because it is a storage-layer decision with its own
  tradeoffs — a C dependency in a binary tuned for size (`opt-level = "z"`, LTO,
  `strip`) and distributed through TPM, and a failure rather than a graceful
  fallback on filesystems without working locks. It deserves its own evaluation
  with a measured binary-size delta and a migration path, not a ride-along on a
  locking fix. LMDB (thread-affine write transactions, one env open per process)
  and redb (single process per database) were both considered and do not fit
  amux's many-short-lived-writers model; SQLite is the only serious candidate.
