# Updating amux

What happens when a new version lands, what runs automatically, and the one
step that is still manual.

## The parts that move independently

An amux installation is three things with separate lifecycles, which is why an
update is not a single atomic event:

- **Plugin checkout** — `~/.tmux/plugins/amux`, updated by TPM (`prefix + U`).
- **Native binary** — `bin/amux-rs` in that checkout, replaced automatically by
  `scripts/ensure-runtime.sh`.
- **Agent integrations** — Codex and Claude hook JSON, the Pi extension and
  registration, and the opencode plugin. They are rewritten only when **you**
  run `amux install-hooks --write`.

The daemon is a fourth moving part: a long-lived process started lazily from
whichever binary was current at the time.

## The update sequence

1. **`prefix + U`** — TPM pulls the new checkout, including a new `VERSION`.
2. **`amux.tmux` runs `scripts/ensure-runtime.sh`** on the next tmux reload or
   amux invocation. It compares `bin/amux-rs --version` against `VERSION`, and
   when they differ downloads the matching release archive with `gh` and
   installs the binary over the old one.
3. **The script retires the running daemon** with `amux daemon --stop`. This
   matters more than it looks — see "Why the daemon is stopped" below.
4. **The next hook event starts a fresh daemon** from the new binary. Nothing
   starts it eagerly; there is no service to restart.
5. **You run `amux install-hooks --write`** if the hook templates changed.

Steps 1–4 need no intervention. Step 5 does, and `amux doctor` tells you when.

## The manual step: hooks

Agent hook files are not amux's to rewrite unasked, so an update never touches
them. When a release changes the hook templates — a new event, a changed
matcher, a different launcher quoting — the installed files fall behind and
amux keeps behaving like the old version until you reinstall.

```bash
amux doctor
```

reports each difference and names the remedy:

```text
hooks Codex: missing amux hook for PreToolUse; run `amux install-hooks --write`
hooks Claude: current
hooks Pi: installed text is stale; run `amux install-hooks --write`
hooks opencode: current
```

Codex and Claude drift names a **missing** hook, **matcher drift**, **argument
drift**, or **launcher quoting drift**. Pi and opencode text assets are compared
with the shipped files, and Pi's settings must still register the installed
extension. Settings and JSON hooks owned by other tools are ignored.

Each integration gets its own `current`, drift, or `unverified` result. An
invalid or unreadable settings file is unverified rather than silently called
current. Only the launcher value in the text assets is normalized, and only
when it is an absolute `bin/amux` path. The canonical TPM launcher
`~/.tmux/plugins/amux/bin/amux`, a source checkout, and a release archive are
therefore equivalent; any other text edit remains visible as drift.

Then:

```bash
amux install-hooks --write     # --dry-run to preview
```

Each configuration file is backed up to `<file>.amux.bak.<timestamp>` first,
written through a temporary file and an atomic rename, and keeps its existing
permissions. A symlinked configuration keeps its symlink; the target is
replaced.

Run it from inside tmux so `AMUX_ROOT` resolves to the plugin checkout;
`amux doctor` prints the root it resolved.

## Why the daemon is stopped

Replacing the binary does not stop the daemon the previous build started. That
process keeps serving old code against on-disk formats the new build writes,
and the failure is not obvious.

The concrete case: the release that replaced the `mkdir` lock with advisory
file locking turned `state.lock` from a directory into a file. A daemon still
running the older build kept calling `mkdir` on that path, got `EEXIST`, read it
as "lock held", and retried for its full acquisition budget. That is five
seconds — the same timeout the shipped hook commands carry — so **every Codex
and Claude hook was killed mid-flight** while amux looked idle.

Two independent defences now prevent it:

- `scripts/ensure-runtime.sh` runs `amux daemon --stop` after installing a new
  binary. The shutdown request predates the versions this has to retire, so a
  daemon running older code still understands it. This is what covers the TPM
  path.
- The daemon fingerprints its own executable (size and mtime) and exits within
  five seconds of it changing, covering `cargo build` and manual installs that
  bypass the script.

The second cannot rescue the first: a daemon already running old code has no
such check, because behaviour cannot be retrofitted into a live process. That is
why the script half exists, and why an amux predating both needs one manual
`pkill -f 'amux-rs daemon'`.

## Verifying an update

```bash
amux doctor
```

A healthy installation reports the binary and root under
`~/.tmux/plugins/amux`, `state: v1 compatible`, `daemon:` connected with a
revision, and `hooks: installed templates match`.

## Troubleshooting

**Hooks time out after five seconds; agents look idle while working.** A daemon
from a previous build is still running. Confirm with `ps aux | grep 'amux-rs
daemon'` and compare the process start time against the binary's mtime
(`ls -la ~/.tmux/plugins/amux/bin/amux-rs`). Fix with `amux daemon --stop`, or
`pkill -f 'amux-rs daemon'` if the daemon is too old to answer.

**`daemon: unavailable` with a parse error.** The daemon is not answering
correctly. Stop it and let the next event start a fresh one; state lives on
disk, so nothing is lost.

**Agents show `done` while visibly working.** The Codex hook set is out of date
— most likely `PreToolUse` is not installed. Run `amux doctor`, then
`amux install-hooks --write`. See [events.md](events.md) for the full mapping
and for the one case hooks cannot observe.

**`gh` is missing.** `ensure-runtime.sh` needs the GitHub CLI to download the
release archive. Without it the plugin reports the failure and leaves the
previous binary in place.

## Downgrading

Check out an older plugin tag and let `ensure-runtime.sh` fetch the matching
binary, then re-run `amux install-hooks --write` so the hook files match the
older templates. State is version-one across these releases and is not
rewritten by a downgrade, but hook templates are not versioned — `amux doctor`
compares against whichever templates the checkout ships.
