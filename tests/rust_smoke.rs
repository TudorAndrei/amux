use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

static NEXT: AtomicUsize = AtomicUsize::new(0);
// tmux creates servers beneath a per-user shared directory, so tests that manage
// real servers (or a long-lived daemon) cannot safely overlap.
static REAL_SERVER_TEST_LOCK: Mutex<()> = Mutex::new(());

fn real_server_test_lock() -> MutexGuard<'static, ()> {
    REAL_SERVER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "amux-rs-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn tmux_temp_dir(label: &str) -> PathBuf {
    // Keep tmux's Unix socket path short on macOS while using the standard
    // temporary directory shared by all supported Unix runners.
    let path = Path::new("/tmp").join(format!(
        "amux-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn amux(state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    command
        .env("AMUX_STATE_DIR", state)
        .env("AMUX_NO_DAEMON", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE");
    command
}

#[cfg(unix)]
fn connect_daemon(socket: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return stream,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("cannot connect to daemon {}: {error}", socket.display()),
        }
    }
}

#[cfg(unix)]
fn daemon_request(state: &Path, request: &str) -> Value {
    use std::io::{BufRead, BufReader, Write};
    let socket = state.join("amux.sock");
    let mut stream = connect_daemon(&socket);
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// A daemon subscription plus the daemon's captured stderr, so a closed stream
/// reports why it closed instead of collapsing every cause into one message.
#[cfg(unix)]
struct MonitorSubscription {
    updates: std::sync::mpsc::Receiver<Result<Value, String>>,
    daemon_log: PathBuf,
    state_dir: PathBuf,
}

#[cfg(unix)]
impl MonitorSubscription {
    fn wait_for(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut last = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.updates.recv_timeout(remaining) {
                Ok(Ok(update)) => {
                    let fields = update.as_object().expect("subscription object");
                    assert!(
                        fields
                            .keys()
                            .all(|field| field == "revision" || field == "views")
                    );
                    let health = daemon_request(&self.state_dir, r#"{"kind":"health"}"#);
                    if predicate(&health) {
                        return health;
                    }
                    last = Some(health);
                }
                Ok(Err(reason)) => panic!(
                    "tmux monitor subscription closed: {reason}\nlast update: {last:?}\n{}",
                    self.daemon_stderr()
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
                    "timed out waiting for tmux monitor update: {last:?}\n{}",
                    self.daemon_stderr()
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!(
                    "tmux monitor reader thread stopped\n{}",
                    self.daemon_stderr()
                ),
            }
        }
    }

    fn daemon_stderr(&self) -> String {
        let log = fs::read_to_string(&self.daemon_log).unwrap_or_default();
        if log.trim().is_empty() {
            "daemon stderr: <empty>".to_owned()
        } else {
            format!("daemon stderr:\n{log}")
        }
    }
}

#[cfg(unix)]
fn monitor_updates(
    stream: UnixStream,
    daemon_log: PathBuf,
    state_dir: PathBuf,
) -> MonitorSubscription {
    use std::io::{BufRead, BufReader};
    let (sender, updates) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            let message = match reader.read_line(&mut line) {
                Ok(0) => Err("daemon closed the stream (EOF)".to_owned()),
                Err(error) => Err(format!("socket read error: {error}")),
                Ok(_) => match serde_json::from_str(&line) {
                    Ok(response) => Ok(response),
                    Err(error) => Err(format!("undecodable response {line:?}: {error}")),
                },
            };
            let closed = message.is_err();
            if sender.send(message).is_err() || closed {
                return;
            }
        }
    });
    MonitorSubscription {
        updates,
        daemon_log,
        state_dir,
    }
}

fn fake_tmux(dir: &Path) -> PathBuf {
    let path = dir.join("tmux");
    fs::write(
        &path,
        r##"#!/usr/bin/env bash
case "$1" in
  -V) printf 'tmux 3.5\n' ;;
  display-message)
    case "${!#}" in
      '#{session_name}') printf '%s\n' multi-agent ;;
      '#{window_id}') printf '%s\n' @1 ;;
      '#{pane_id}') printf '%s\n' "${TMUX_PANE:-%20}" ;;
    esac ;;
  list-sessions)
    separator=$'\037'
    printf '500%s%s%s0\n' "$separator" multi-agent "$separator" ;;
  list-panes)
    separator=$'\037'
    printf 'multi-agent%s%%20%scodex%s500%scodex%s/tmp/multi-codex\n' "$separator" "$separator" "$separator" "$separator" "$separator"
    printf 'multi-agent%s%%21%sclaude%s501%sclaude%s/tmp/multi-claude\n' "$separator" "$separator" "$separator" "$separator" "$separator" ;;
  refresh-client) ;;
esac
"##,
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn tmux_command(socket_dir: &Path) -> Command {
    let mut command = Command::new("tmux");
    // Isolate tests from a developer's tmux.conf and installed TPM plugins.
    command
        .env("TMUX_TMPDIR", socket_dir)
        .args(["-f", "/dev/null"]);
    command
}

/// An isolated tmux server owned by a single test.
///
/// `Drop` kills the server and removes its socket directory, so a failed
/// assertion or a panicking helper cannot leave the server behind. Orphans
/// outlive the test run, reparent to init, and break every tool that refuses to
/// act while more than one server is up — tmux-continuum, for one, stops both
/// auto-save and auto-restore.
#[cfg(unix)]
struct TmuxServer {
    tmpdir: PathBuf,
    socket_name: String,
}

#[cfg(unix)]
impl TmuxServer {
    /// Starts a detached server holding one session named `session`.
    fn start(label: &str, session: &str) -> Self {
        sweep_orphan_tmux_servers();
        let server = Self {
            tmpdir: tmux_temp_dir(label),
            socket_name: format!(
                "amux-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
        };
        server.new_session(session);
        server
    }

    /// Adds a session, and brings the server back after a deliberate kill.
    fn new_session(&self, session: &str) {
        assert!(
            self.command()
                .args(["new-session", "-d", "-s", session])
                .status()
                .unwrap()
                .success()
        );
    }

    /// A `tmux` invocation aimed at this server.
    fn command(&self) -> Command {
        let mut command = tmux_command(&self.tmpdir);
        command.args(["-L", &self.socket_name]);
        command
    }

    /// The socket path tmux resolved, for callers that set `$TMUX`.
    fn socket_path(&self) -> String {
        String::from_utf8(
            self.command()
                .args(["display-message", "-p", "#{socket_path}"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    }
}

/// Kills tmux servers left by earlier runs of this suite.
///
/// `TmuxServer::drop` covers a panicking test, but not a run that a signal ends
/// — Ctrl-C, or a `cargo test` timeout. Every socket directory carries the pid
/// that made it, so a directory whose pid is gone belongs to a dead run and its
/// servers are safe to kill. Runs once per test binary.
#[cfg(unix)]
fn sweep_orphan_tmux_servers() {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    SWEEP.call_once(|| {
        let Ok(entries) = fs::read_dir("/tmp") else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            // `amux-{label}-{pid}-{sequence}`, as `tmux_temp_dir` builds it.
            let Some(rest) = name.strip_prefix("amux-") else {
                continue;
            };
            let mut fields = rest.rsplitn(3, '-');
            let (Some(_sequence), Some(pid), Some(_label)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if pid.parse::<u32>().is_err() || process_is_alive(pid) {
                continue;
            }
            // tmux keeps its sockets one level down, under `tmux-{uid}`.
            for uid_dir in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                for socket in fs::read_dir(uid_dir.path()).into_iter().flatten().flatten() {
                    let _ = Command::new("tmux")
                        .args([
                            "-S".as_ref(),
                            socket.path().as_os_str(),
                            "kill-server".as_ref(),
                        ])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            let _ = fs::remove_dir_all(entry.path());
        }
    });
}

#[cfg(unix)]
fn process_is_alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").status();
        let _ = fs::remove_dir_all(&self.tmpdir);
    }
}

#[test]
fn cli_clear_doctor_and_option_contracts_are_preserved() {
    let state = temp_dir("cli-contract");
    let home = temp_dir("cli-home");
    let fake_bin = temp_dir("cli-tmux");
    fake_tmux(&fake_bin);
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let installed = amux(&state)
        .env("HOME", &home)
        .env("AMUX_ROOT", root)
        .args(["install-hooks", "--write"])
        .output()
        .unwrap();
    assert!(installed.status.success());
    event(
        &state,
        "codex",
        &["--event", "PostToolUse"],
        br#"{"session_id":"clear-me"}"#.to_vec(),
    );
    let listed = amux(&state).args(["list", "--json"]).output().unwrap();
    assert!(listed.status.success());
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["version"], 1);
    assert!(listed["records"].get("codex:clear-me").is_some());
    let doctor = amux(&state)
        .env("PATH", &path)
        .env("HOME", &home)
        .env("AMUX_ROOT", root)
        .arg("doctor")
        .output()
        .unwrap();
    assert!(doctor.status.success());
    let doctor = String::from_utf8(doctor.stdout).unwrap();
    assert!(doctor.contains("tmux: ok (tmux 3.5)"));
    assert!(doctor.contains("state: v1 compatible"));
    assert!(doctor.contains("maintenance: ok"));
    for integration in ["Codex", "Claude", "Pi", "opencode"] {
        assert!(doctor.contains(&format!("hooks {integration}: current")));
    }
    fs::write(
        state.join("maintenance.error"),
        "injected compaction failure\n",
    )
    .unwrap();
    let degraded = amux(&state)
        .env("PATH", &path)
        .arg("doctor")
        .output()
        .unwrap();
    assert!(!degraded.status.success());
    assert!(
        String::from_utf8(degraded.stdout)
            .unwrap()
            .contains("maintenance: degraded (injected compaction failure)")
    );
    fs::remove_file(state.join("maintenance.error")).unwrap();
    let invalid_doctor = amux(&state)
        .env("PATH", &path)
        .env("AMUX_STALE_SECONDS", "-1")
        .env("AMUX_EVENTS_COMPACT_BYTES", "0")
        .env("AMUX_EVENTS_PER_SESSION", "not-a-number")
        .arg("doctor")
        .output()
        .unwrap();
    let invalid_doctor = String::from_utf8(invalid_doctor.stdout).unwrap();
    assert!(invalid_doctor.contains("rejected AMUX_STALE_SECONDS"));
    assert!(invalid_doctor.contains("rejected AMUX_EVENTS_COMPACT_BYTES"));
    assert!(invalid_doctor.contains("rejected AMUX_EVENTS_PER_SESSION"));
    let zero_doctor = amux(&state)
        .env("PATH", &path)
        .env("AMUX_EVENTS_PER_SESSION", "0")
        .arg("doctor")
        .output()
        .unwrap();
    let zero_doctor = String::from_utf8(zero_doctor.stdout).unwrap();
    assert!(!zero_doctor.contains("rejected AMUX_EVENTS_PER_SESSION"));
    assert!(zero_doctor.contains("events per session: 0"));
    let valid_doctor = amux(&state)
        .env("PATH", &path)
        .env("AMUX_EVENTS_PER_SESSION", "17")
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(valid_doctor.stdout)
            .unwrap()
            .contains("events per session: 17")
    );
    let lock_doctor = amux(&state)
        .env("PATH", &path)
        .env("AMUX_LOCK", "off")
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(lock_doctor.stdout)
            .unwrap()
            .contains("locking: disabled")
    );
    assert!(amux(&state).arg("clear").status().unwrap().success());
    assert!(!state.join("state.json").exists());
    assert!(!state.join("events.jsonl").exists());
    assert!(
        !amux(&state)
            .args(["sessions", "--unexpected"])
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(fake_bin).unwrap();
}

#[test]
fn events_cli_filters_bounds_and_sanitizes_retained_history() {
    let state = temp_dir("events-history");
    let empty = amux(&state).args(["events", "--json"]).output().unwrap();
    assert!(empty.status.success());
    let empty: Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(empty, serde_json::json!({"version": 1, "events": []}));

    let events = [
        serde_json::json!({
            "key": "codex:one:%1",
            "agent": "codex",
            "agent_session_id": "agent-one",
            "tmux_session": "one",
            "tmux_pane": "%1",
            "status": "running",
            "last_event": "SessionStart",
            "updated_at": 1,
            "updated_at_iso": "1970-01-01T00:00:01Z",
            "raw": {"session_id": "agent-one"}
        })
        .to_string(),
        "not json".to_owned(),
        serde_json::json!({
            "key": "claude:two:%2",
            "agent": "claude",
            "agent_session_id": "agent-two",
            "tmux_session": "two",
            "tmux_pane": "%2",
            "status": "done",
            "last_event": "Stop",
            "updated_at": 2,
            "updated_at_iso": "1970-01-01T00:00:02Z"
        })
        .to_string(),
        serde_json::json!({
            "key": "codex:one:%1",
            "agent": "codex",
            "agent_session_id": "agent-one",
            "tmux_session": "one",
            "tmux_pane": "%1",
            "cwd": "/tmp/project\u{1b}[2J",
            "status": "attention",
            "attention": true,
            "reason": "wait\u{1b}[2J\nnext",
            "last_event": "PermissionRequest",
            "updated_at": 3,
            "updated_at_iso": "1970-01-01T00:00:03Z",
            "raw": {
                "notification_type": "permission_prompt",
                "secret": "must-not-escape"
            },
            "unknown_top_level": "must-not-escape"
        })
        .to_string(),
        serde_json::json!({
            "key": "codex:agent-only",
            "agent": "codex",
            "agent_session_id": "agent-only",
            "status": "done",
            "last_event": "Stop",
            "updated_at": 4,
            "updated_at_iso": "1970-01-01T00:00:04Z"
        })
        .to_string(),
        serde_json::json!({"key": "missing-record"}).to_string(),
    ]
    .join("\n");
    fs::write(state.join("events.jsonl"), format!("{events}\n")).unwrap();

    let limited = amux(&state)
        .args(["events", "--json", "--limit", "2"])
        .output()
        .unwrap();
    assert!(limited.status.success());
    let limited: Value = serde_json::from_slice(&limited.stdout).unwrap();
    let limited = limited["events"].as_array().unwrap();
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[0]["updated_at"], 3);
    assert_eq!(limited[1]["updated_at"], 4);
    assert!(limited[0]["raw"].get("secret").is_none());
    assert!(limited[0].get("unknown_top_level").is_none());
    assert_eq!(limited[0]["raw"]["notification_type"], "permission_prompt");

    let filtered = amux(&state)
        .args([
            "events",
            "--json",
            "--agent",
            "codex",
            "--session",
            "one",
            "--pane",
            "%1",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    let filtered: Value = serde_json::from_slice(&filtered.stdout).unwrap();
    assert_eq!(filtered["events"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["events"][0]["updated_at"], 3);

    let agent_session = amux(&state)
        .args(["events", "--json", "--session", "agent-only"])
        .output()
        .unwrap();
    let agent_session: Value = serde_json::from_slice(&agent_session.stdout).unwrap();
    assert_eq!(agent_session["events"].as_array().unwrap().len(), 1);
    assert_eq!(agent_session["events"][0]["updated_at"], 4);

    let plain = amux(&state)
        .args(["events", "--agent", "codex", "--session", "one"])
        .output()
        .unwrap();
    assert!(plain.status.success());
    let plain = String::from_utf8(plain.stdout).unwrap();
    assert!(!plain.contains('\u{1b}'));
    assert!(plain.contains("wait^[[2J^Jnext"));
    assert!(plain.contains("/tmp/project^[[2J"));

    let unbounded_retention = amux(&state)
        .env("AMUX_EVENTS_PER_SESSION", "0")
        .args(["events", "--json", "--limit", "1"])
        .output()
        .unwrap();
    assert!(unbounded_retention.status.success());
    let unbounded_retention: Value = serde_json::from_slice(&unbounded_retention.stdout).unwrap();
    assert_eq!(unbounded_retention["events"][0]["updated_at"], 4);

    let excessive = amux(&state)
        .args(["events", "--limit", "1001"])
        .output()
        .unwrap();
    assert!(!excessive.status.success());
    assert!(
        String::from_utf8(excessive.stderr)
            .unwrap()
            .contains("between 1 and 1000")
    );
    fs::remove_dir_all(state).unwrap();
}

fn event(state: &Path, agent: &str, extra: &[&str], input: Vec<u8>) {
    let mut output = amux(state)
        .arg("event")
        .arg("--agent")
        .arg(agent)
        .args(extra)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = output.stdin.take().unwrap();
    use std::io::Write;
    stdin.write_all(&input).unwrap();
    drop(stdin);
    assert!(output.wait_with_output().unwrap().status.success());
}

#[test]
fn rendered_codex_hook_commands_produce_the_declared_statuses() {
    use std::io::Write;

    let state = temp_dir("rendered-codex-hooks");
    let home = temp_dir("rendered-codex-home");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let install = amux(&state)
        .env("HOME", &home)
        .env("AMUX_ROOT", root)
        .args(["install-hooks", "--write"])
        .output()
        .unwrap();
    assert!(install.status.success());
    let hooks: Value =
        serde_json::from_slice(&fs::read(home.join(".codex/hooks.json")).unwrap()).unwrap();
    let expected = [
        ("SessionStart", "running", false),
        ("UserPromptSubmit", "running", false),
        ("PreToolUse", "running", false),
        ("PermissionRequest", "attention", true),
        ("PostToolUse", "running", false),
        ("PreCompact", "running", false),
        ("PostCompact", "running", false),
        ("Stop", "done", false),
        ("SessionEnd", "offline", false),
    ];
    for (event_name, status, attention) in expected {
        let command = hooks["hooks"][event_name][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let mut child = Command::new("sh")
            .args(["-c", command])
            .env("AMUX_STATE_DIR", &state)
            .env("AMUX_NO_DAEMON", "1")
            .env_remove("TMUX")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(br#"{"session_id":"rendered-codex"}"#)
            .unwrap();
        assert!(
            child.wait_with_output().unwrap().status.success(),
            "{event_name}"
        );
        let listed = amux(&state).args(["list", "--json"]).output().unwrap();
        let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
        assert_eq!(listed["records"]["codex:rendered-codex"]["status"], status);
        assert_eq!(
            listed["records"]["codex:rendered-codex"]["attention"],
            attention
        );
    }
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn rendered_claude_hooks_classify_completion_notifications_in_both_event_paths() {
    use std::io::Write;

    let home = temp_dir("rendered-claude-home");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let setup_state = temp_dir("rendered-claude-setup");
    let install = amux(&setup_state)
        .env("HOME", &home)
        .env("AMUX_ROOT", root)
        .args(["install-hooks", "--write"])
        .output()
        .unwrap();
    assert!(install.status.success());
    let hooks: Value =
        serde_json::from_slice(&fs::read(home.join(".claude/settings.json")).unwrap()).unwrap();
    let notification = hooks["hooks"]["Notification"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!notification.contains("--attention"));
    let stop = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!stop.contains("--status"));
    assert!(!stop.contains("--attention"));
    let rendered_command = |command: &str| {
        command.replacen(
            &format!("'{}'", root.join("bin/amux").display()),
            &format!("'{}'", env!("CARGO_BIN_EXE_amux-rs")),
            1,
        )
    };

    for daemonless in [true, false] {
        let state = temp_dir(if daemonless {
            "rendered-claude-daemonless"
        } else {
            "rendered-claude-daemon"
        });
        for (command, payload) in [
            (
                stop.as_str(),
                br#"{"session_id":"rendered-claude"}"#.as_slice(),
            ),
            (
                notification.as_str(),
                br#"{"session_id":"rendered-claude","notification_type":"idle_prompt"}"#.as_slice(),
            ),
        ] {
            let command = rendered_command(command);
            let mut invocation = Command::new("sh");
            invocation
                .args(["-c", command.as_str()])
                .env("AMUX_STATE_DIR", &state)
                .env_remove("TMUX")
                .stdin(Stdio::piped());
            if daemonless {
                invocation.env("AMUX_NO_DAEMON", "1");
            }
            let mut child = invocation.spawn().unwrap();
            child.stdin.as_mut().unwrap().write_all(payload).unwrap();
            assert!(child.wait_with_output().unwrap().status.success());
        }
        let listed = amux(&state).args(["list", "--json"]).output().unwrap();
        let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
        let record = &listed["records"]["claude:rendered-claude"];
        assert_eq!(record["status"], "done");
        assert_eq!(record["attention"], false);

        let mut invocation = Command::new("sh");
        let command = rendered_command(&notification);
        invocation
            .args(["-c", command.as_str()])
            .env("AMUX_STATE_DIR", &state)
            .env_remove("TMUX")
            .stdin(Stdio::piped());
        if daemonless {
            invocation.env("AMUX_NO_DAEMON", "1");
        }
        let mut child = invocation.spawn().unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(
                br#"{"session_id":"rendered-claude","notification_type":"permission_prompt"}"#,
            )
            .unwrap();
        assert!(child.wait_with_output().unwrap().status.success());
        let listed = amux(&state).args(["list", "--json"]).output().unwrap();
        let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
        let record = &listed["records"]["claude:rendered-claude"];
        assert_eq!(record["status"], "attention");
        assert_eq!(record["attention"], true);

        if !daemonless {
            let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
        }
        fs::remove_dir_all(state).unwrap();
    }
    fs::remove_dir_all(setup_state).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
#[cfg(unix)]
fn pi_and_opencode_lifecycles_match_with_and_without_the_daemon() {
    use std::io::Write;

    for daemonless in [true, false] {
        let state = if daemonless {
            temp_dir("integration-lifecycle-daemonless")
        } else {
            tmux_temp_dir("int-life")
        };

        for (agent, event_name, fixture_name, key, status, attention) in [
            (
                "pi",
                "session_start",
                "pi-session-start.json",
                "pi:pi-session-1",
                "running",
                false,
            ),
            (
                "pi",
                "agent_start",
                "pi-agent-start.json",
                "pi:pi-session-1",
                "running",
                false,
            ),
            (
                "pi",
                "agent_settled",
                "pi-agent-settled.json",
                "pi:pi-session-1",
                "done",
                false,
            ),
            (
                "pi",
                "session_shutdown",
                "pi-session-shutdown.json",
                "pi:pi-session-1",
                "offline",
                false,
            ),
            (
                "opencode",
                "session.created",
                "opencode-session-created.json",
                "opencode:opencode-session-1",
                "running",
                false,
            ),
            (
                "opencode",
                "session.status",
                "opencode-session-busy.json",
                "opencode:opencode-session-1",
                "running",
                false,
            ),
            (
                "opencode",
                "permission.asked",
                "opencode-permission-asked.json",
                "opencode:opencode-session-1",
                "attention",
                true,
            ),
            (
                "opencode",
                "permission.replied",
                "opencode-permission-replied.json",
                "opencode:opencode-session-1",
                "running",
                false,
            ),
            (
                "opencode",
                "session.status",
                "opencode-session-idle.json",
                "opencode:opencode-session-1",
                "done",
                false,
            ),
            (
                "opencode",
                "session.deleted",
                "opencode-session-deleted.json",
                "opencode:opencode-session-1",
                "offline",
                false,
            ),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
            command
                .args(["event", "--agent", agent, "--event", event_name])
                .env("AMUX_STATE_DIR", &state)
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .stdin(Stdio::piped());
            if daemonless {
                command.env("AMUX_NO_DAEMON", "1");
            } else {
                command.env_remove("AMUX_NO_DAEMON");
            }
            let mut command = command.spawn().unwrap();
            command
                .stdin
                .as_mut()
                .unwrap()
                .write_all(&fixture(fixture_name))
                .unwrap();
            assert!(command.wait_with_output().unwrap().status.success());

            let listed = amux(&state).args(["list", "--json"]).output().unwrap();
            let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
            assert_eq!(listed["records"][key]["status"], status, "{event_name}");
            assert_eq!(
                listed["records"][key]["attention"], attention,
                "{event_name}"
            );
        }

        if !daemonless {
            let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
        }
        fs::remove_dir_all(state).unwrap();
    }
}

#[test]
#[cfg(unix)]
fn daemon_adopts_an_orphaned_event_log_before_compacting() {
    let _lock = real_server_test_lock();
    let state = temp_dir("orphaned-event-log");
    event(
        &state,
        "codex",
        &["--event", "PreToolUse"],
        br#"{"session_id":"orphaned-old"}"#.to_vec(),
    );
    fs::rename(
        state.join("events.jsonl"),
        state.join("events.jsonl.compacting"),
    )
    .unwrap();
    event(
        &state,
        "codex",
        &["--event", "PostToolUse"],
        br#"{"session_id":"orphaned-live"}"#.to_vec(),
    );
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    daemon
        .arg("daemon")
        .env("AMUX_STATE_DIR", &state)
        .env("AMUX_EVENTS_PER_SESSION", "200")
        .env_remove("TMUX")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = daemon.spawn().unwrap();
    for _ in 0..40 {
        if state.join("amux.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(state.join("amux.sock").exists());
    let log = fs::read_to_string(state.join("events.jsonl")).unwrap();
    assert!(log.contains("orphaned-old"));
    assert!(log.contains("orphaned-live"));
    assert!(!state.join("events.jsonl.compacting").exists());
    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    let _ = daemon.wait();
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn daemon_rejection_never_falls_back_to_direct_persistence() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let state = temp_dir("daemon-rejection");
    let listener = UnixListener::bind(state.join("amux.sock")).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&request).unwrap()["kind"],
            "event"
        );
        stream
            .write_all(br#"{"revision":0,"error":"event rejected by policy"}"#)
            .unwrap();
        stream.write_all(b"\n").unwrap();
    });
    let mut event = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    event
        .args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &state)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());
    let mut event = event.spawn().unwrap();
    event
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"must-not-persist"}"#)
        .unwrap();
    let output = event.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("daemon rejected request"));
    server.join().unwrap();
    assert!(!state.join("state.json").exists());
    assert!(!state.join("events.jsonl").exists());
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn oversized_event_input_is_rejected_before_persistence() {
    use std::io::Write;

    let state = temp_dir("oversized-event");
    let mut event = amux(&state);
    event
        .args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());
    let mut event = event.spawn().unwrap();
    event
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&vec![b'x'; 256 * 1024 + 1])
        .unwrap();
    let output = event.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds 256 KiB"));
    assert!(!state.join("state.json").exists());
    assert!(!state.join("events.jsonl").exists());
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn daemon_and_daemonless_events_use_equivalent_minimized_intake() {
    use std::io::Write;

    let direct = temp_dir("intake-direct");
    let daemon = temp_dir("intake-daemon");
    let payload = br#"{
        "session_id":"equivalent-session",
        "cwd":"/workspace",
        "agent_id":"child-agent",
        "is_subagent":true,
        "tool_input":{"command":"sensitive command"},
        "message":"sensitive prompt",
        "unknown":"sensitive unknown"
    }"#;
    event(
        &direct,
        "codex",
        &["--event", "PostToolUse"],
        payload.to_vec(),
    );

    let mut hook = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    hook.args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &daemon)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(Stdio::piped());
    let mut hook = hook.spawn().unwrap();
    hook.stdin.as_mut().unwrap().write_all(payload).unwrap();
    assert!(hook.wait_with_output().unwrap().status.success());

    let normalized_record = |state_dir: &Path| {
        let state: Value =
            serde_json::from_slice(&fs::read(state_dir.join("state.json")).unwrap()).unwrap();
        let mut record = state["records"]["codex:equivalent-session"].clone();
        record["updated_at"] = Value::from(0);
        record["updated_at_iso"] = Value::from("");
        record
    };
    let direct_record = normalized_record(&direct);
    let daemon_record = normalized_record(&daemon);
    assert_eq!(direct_record, daemon_record);
    assert_eq!(direct_record["raw"]["agent_id"], "child-agent");
    for sensitive in ["tool_input", "message", "unknown"] {
        assert!(direct_record["raw"].get(sensitive).is_none());
    }
    assert!(
        !fs::read_to_string(direct.join("events.jsonl"))
            .unwrap()
            .contains("sensitive")
    );
    assert!(
        !fs::read_to_string(daemon.join("events.jsonl"))
            .unwrap()
            .contains("sensitive")
    );

    let _ = daemon_request(&daemon, r#"{"kind":"shutdown"}"#);
    fs::remove_dir_all(direct).unwrap();
    fs::remove_dir_all(daemon).unwrap();
}

#[test]
#[cfg(unix)]
fn daemon_clear_waits_for_maintenance_and_history_stays_absent() {
    use std::io::Write;

    let state = temp_dir("maintenance-clear");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    daemon
        .arg("daemon")
        .env("AMUX_STATE_DIR", &state)
        .env("AMUX_EVENTS_COMPACT_BYTES", "1")
        .env("AMUX_EVENTS_PER_SESSION", "2")
        .env_remove("TMUX")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = daemon.spawn().unwrap();
    connect_daemon(&state.join("amux.sock"));

    for index in 0..20 {
        let mut hook = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
        hook.args(["event", "--agent", "codex", "--event", "PostToolUse"])
            .env("AMUX_STATE_DIR", &state)
            .env_remove("AMUX_NO_DAEMON")
            .env_remove("TMUX")
            .stdin(Stdio::piped());
        let mut hook = hook.spawn().unwrap();
        hook.stdin
            .as_mut()
            .unwrap()
            .write_all(format!(r#"{{"session_id":"maintenance-{index}"}}"#).as_bytes())
            .unwrap();
        assert!(hook.wait_with_output().unwrap().status.success());
    }
    assert!(state.join("events.jsonl").exists() || state.join("events.jsonl.compacting").exists());

    assert_eq!(
        daemon_request(&state, r#"{"kind":"clear"}"#)["error"],
        Value::Null
    );
    thread::sleep(Duration::from_millis(100));
    for name in [
        "state.json",
        "events.jsonl",
        "events.jsonl.compacting",
        "events.jsonl.retained",
    ] {
        assert!(!state.join(name).exists(), "{name} reappeared after clear");
    }

    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    assert!(daemon.wait().unwrap().success());
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn concurrent_starters_serialize_stale_socket_recovery() {
    let state = temp_dir("daemon-starters");
    let socket = state.join("amux.sock");
    // Dropping a bound Unix listener leaves a real stale socket inode behind.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());

    let start = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
        command
            .arg("daemon")
            .env("AMUX_STATE_DIR", &state)
            .env_remove("TMUX")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command.spawn().unwrap()
    };
    let mut first = start();
    let mut second = start();

    let response = daemon_request(&state, r#"{"kind":"ping"}"#);
    assert_eq!(response["revision"], 0);

    let deadline = Instant::now() + Duration::from_secs(2);
    let loser = loop {
        if let Some(status) = first.try_wait().unwrap() {
            assert!(!status.success());
            break 1;
        }
        if let Some(status) = second.try_wait().unwrap() {
            assert!(!status.success());
            break 2;
        }
        assert!(
            Instant::now() < deadline,
            "both daemon starters stayed alive"
        );
        thread::sleep(Duration::from_millis(20));
    };

    // The losing starter has completed all cleanup; the winner must remain
    // reachable and its live listener must still occupy the socket path.
    assert_eq!(daemon_request(&state, r#"{"kind":"ping"}"#)["revision"], 0);
    assert!(socket.exists());
    assert_eq!(
        daemon_request(&state, r#"{"kind":"shutdown"}"#)["error"],
        Value::Null
    );

    let winner = if loser == 1 { &mut second } else { &mut first };
    assert!(winner.wait().unwrap().success());
    assert!(!socket.exists());
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn fixture_normalization_uses_the_v1_schema() {
    let state = temp_dir("fixtures");
    event(&state, "codex", &[], fixture("codex-permission.json"));
    event(&state, "claude", &[], fixture("claude-stop.json"));
    event(
        &state,
        "opencode",
        &[],
        fixture("opencode-session-idle.json"),
    );
    event(&state, "pi", &[], fixture("pi-tool-call.json"));
    event(&state, "codex", &[], fixture("codex-subagent.json"));
    event(
        &state,
        "codex",
        &[],
        fixture("codex-subagent-empty-cwd.json"),
    );
    event(
        &state,
        "codex",
        &["--event", "UserPromptSubmit"],
        br#"{"session_id":"prompt-session"}"#.to_vec(),
    );
    let output = amux(&state).args(["list", "--json"]).output().unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["records"].as_object().unwrap().len(), 7);
    assert_eq!(
        value["records"]["codex:codex-session-1"]["status"],
        "attention"
    );
    assert_eq!(
        value["records"]["claude:claude-session-1"]["status"],
        "done"
    );
    assert_eq!(
        value["records"]["opencode:opencode-session-1"]["status"],
        "done"
    );
    assert_eq!(
        value["records"]["opencode:opencode-session-1"]["attention"],
        false
    );
    assert_eq!(
        value["records"]["codex:prompt-session"]["status"],
        "running"
    );
    assert!(
        value["records"]
            .as_object()
            .unwrap()
            .values()
            .any(|record| record["raw"]["agent_id"] == "codex-subagent-1")
    );
    assert_eq!(
        fs::read_to_string(state.join("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        7
    );
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn list_honors_stale_records_and_preserves_documented_json_contract() {
    let state = temp_dir("stale-contract");
    fs::write(
        state.join("state.json"),
        r#"{
          "version": 1,
          "records": {
            "expired": {
              "agent": "codex",
              "agent_session_id": "expired",
              "tmux_session": "old",
              "tmux_pane": "%1",
              "cwd": "/tmp/old",
              "status": "attention",
              "attention": true,
              "reason": "old",
              "last_event": "PermissionRequest",
              "updated_at": 1,
              "updated_at_iso": "1970-01-01T00:00:01Z",
              "raw": {}
            }
          }
        }"#,
    )
    .unwrap();
    let output = amux(&state)
        .env("AMUX_STALE_SECONDS", "1")
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let state_json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(state_json["version"], 1);
    // `list --json` intentionally exposes the on-disk v1 document unchanged;
    // stale filtering is a rendering concern for list text, sessions, and status.
    assert!(state_json["records"]["expired"].is_object());
    assert!(
        amux(&state)
            .env("AMUX_STALE_SECONDS", "1")
            .arg("list")
            .output()
            .unwrap()
            .stdout
            .is_empty()
    );
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn version_one_state_loads_without_migration_and_prunes_stale_records_on_write() {
    let state = temp_dir("v1-state");
    fs::write(
        state.join("state.json"),
        r#"{
          "version": 1,
          "records": {
            "legacy": {
              "agent": "codex",
              "agent_session_id": "legacy-session",
              "tmux_session": "legacy",
              "tmux_pane": "%9",
              "cwd": "/tmp/legacy",
              "status": "done",
              "attention": false,
              "reason": "old record",
              "last_event": "Stop",
              "updated_at": 1,
              "raw": {"source": "v1"}
            }
          }
        }"#,
    )
    .unwrap();
    let loaded = amux(&state).args(["list", "--json"]).output().unwrap();
    assert!(loaded.status.success());
    let loaded: Value = serde_json::from_slice(&loaded.stdout).unwrap();
    assert_eq!(loaded["version"], 1);
    assert_eq!(
        loaded["records"]["legacy"]["agent_session_id"],
        "legacy-session"
    );
    assert_eq!(loaded["records"]["legacy"]["raw"]["source"], "v1");
    event(
        &state,
        "claude",
        &["--event", "PostToolUse"],
        br#"{"session_id":"fresh"}"#.to_vec(),
    );
    let rewritten: Value =
        serde_json::from_slice(&fs::read(state.join("state.json")).unwrap()).unwrap();
    assert_eq!(rewritten["version"], 1);
    assert!(rewritten["records"].get("legacy").is_none());
    assert!(rewritten["records"].get("claude:fresh").is_some());
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn concurrent_event_writes_leave_valid_state_and_log() {
    let state = temp_dir("race");
    let mut children = Vec::new();
    for index in 0..40 {
        let mut child = amux(&state);
        child
            .args(["event", "--agent", "codex", "--event", "PostToolUse"])
            .stdin(Stdio::piped());
        let mut child = child.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!(r#"{{"session_id":"race-{index}"}}"#).as_bytes())
            .unwrap();
        children.push(child);
    }
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let value: Value =
        serde_json::from_slice(&fs::read(state.join("state.json")).unwrap()).unwrap();
    assert_eq!(value["records"].as_object().unwrap().len(), 40);
    assert_eq!(
        fs::read_to_string(state.join("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        40
    );
    assert!(state.join("state.lock").is_file());
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn direct_clear_and_event_leave_matching_state_and_log_files() {
    use std::io::Write;

    let state = temp_dir("clear-race");
    for index in 0..20 {
        let mut event_child = amux(&state);
        event_child
            .args(["event", "--agent", "codex", "--event", "PostToolUse"])
            .stdin(Stdio::piped());
        let mut event_child = event_child.spawn().unwrap();
        event_child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!(r#"{{"session_id":"clear-race-{index}"}}"#).as_bytes())
            .unwrap();
        let clear_child = amux(&state).arg("clear").spawn().unwrap();
        assert!(event_child.wait_with_output().unwrap().status.success());
        assert!(clear_child.wait_with_output().unwrap().status.success());
        let state_exists = state.join("state.json").exists();
        let log_exists = state.join("events.jsonl").exists();
        assert_eq!(state_exists, log_exists);
        if state_exists {
            let value: Value =
                serde_json::from_slice(&fs::read(state.join("state.json")).unwrap()).unwrap();
            assert!(!value["records"].as_object().unwrap().is_empty());
            assert!(
                !fs::read_to_string(state.join("events.jsonl"))
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>()
                    .is_empty()
            );
        }
    }
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn fallback_writes_create_private_state_storage() {
    let state = temp_dir("private-storage");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    event(
        &state,
        "codex",
        &["--event", "PostToolUse"],
        br#"{"session_id":"private-storage"}"#.to_vec(),
    );
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for path in [state.join("state.json"), state.join("events.jsonl")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(state).unwrap();
}

#[test]
fn tmux_sessions_aggregate_agents_and_choose_the_highest_priority_pane() {
    let state = temp_dir("tmux");
    let fake_bin = temp_dir("fake-tmux");
    fake_tmux(&fake_bin);
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
    for (agent, pane, event_name, payload) in [
        (
            "codex",
            "%20",
            "PostToolUse",
            br#"{"session_id":"codex-1","cwd":"/tmp/multi-codex"}"#.as_slice(),
        ),
        (
            "claude",
            "%21",
            "Notification",
            br#"{"session_id":"claude-1","cwd":"/tmp/multi-claude"}"#.as_slice(),
        ),
    ] {
        let mut child = amux(&state);
        child
            .env("PATH", &path)
            .env("TMUX", "fake")
            .env("TMUX_PANE", pane)
            .args(["event", "--agent", agent, "--event", event_name])
            .stdin(Stdio::piped());
        let mut child = child.spawn().unwrap();
        use std::io::Write;
        child.stdin.as_mut().unwrap().write_all(payload).unwrap();
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let output = amux(&state)
        .env("PATH", &path)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let sessions: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["session"], "multi-agent");
    assert_eq!(sessions[0]["status"], "attention");
    assert_eq!(sessions[0]["pane"], "%21");
    assert_eq!(sessions[0]["agent_count"], 2);
    assert_eq!(sessions[0]["live_agent_count"], 2);
    assert!(
        sessions[0]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| agent["agent"] == "codex" && agent["status"] == "running")
    );
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(fake_bin).unwrap();
}

#[test]
#[cfg(unix)]
fn lazy_daemon_persists_events_and_serves_revisions() {
    let _lock = real_server_test_lock();
    let state = temp_dir("daemon");
    let mut command = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    command
        .args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &state)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .stdin(Stdio::piped());
    let mut child = command.spawn().unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"daemon-one"}"#)
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    for _ in 0..20 {
        if state.join("amux.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(state.join("amux.sock").exists());
    assert_eq!(
        fs::metadata(state.join("amux.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(daemon_request(&state, r#"{"kind":"ping"}"#)["revision"], 1);
    use std::io::{BufRead, BufReader};
    let mut subscription = UnixStream::connect(state.join("amux.sock")).unwrap();
    subscription.write_all(br#"{"kind":"subscribe"}"#).unwrap();
    subscription.write_all(b"\n").unwrap();
    let mut subscription = BufReader::new(subscription);
    let mut initial = String::new();
    subscription.read_line(&mut initial).unwrap();
    let initial: Value = serde_json::from_str(&initial).unwrap();
    assert_eq!(initial["revision"], 1);
    assert!(initial.get("state").is_none());
    assert!(initial.get("topology").is_none());
    let mut second = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    second
        .args(["event", "--agent", "claude", "--event", "Stop"])
        .env("AMUX_STATE_DIR", &state)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .stdin(Stdio::piped());
    let mut second = second.spawn().unwrap();
    second
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"daemon-two"}"#)
        .unwrap();
    assert!(second.wait_with_output().unwrap().status.success());
    let mut update = String::new();
    subscription.read_line(&mut update).unwrap();
    let update: Value = serde_json::from_str(&update).unwrap();
    assert_eq!(update["revision"], 2);
    assert!(update.get("state").is_none());
    assert!(update.get("topology").is_none());
    assert!(amux(&state).arg("clear").status().unwrap().success());
    let mut cleared = String::new();
    subscription.read_line(&mut cleared).unwrap();
    let cleared: Value = serde_json::from_str(&cleared).unwrap();
    assert_eq!(cleared["revision"], 3);
    assert!(cleared.get("state").is_none());
    assert!(cleared.get("topology").is_none());
    let listed: Value = serde_json::from_slice(
        &amux(&state)
            .args(["list", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(listed["records"].as_object().unwrap().is_empty());
    assert!(
        amux(&state)
            .args(["picker", "--rows"])
            .output()
            .unwrap()
            .stdout
            .is_empty()
    );
    assert!(!state.join("state.json").exists());
    assert!(!state.join("events.jsonl").exists());
    assert_eq!(
        daemon_request(&state, r#"{"kind":"shutdown"}"#)["revision"],
        0
    );
    for _ in 0..20 {
        if !state.join("amux.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(!state.join("amux.sock").exists());
    let mut restarted = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    restarted
        .args(["event", "--agent", "pi", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &state)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .stdin(Stdio::piped());
    let mut restarted = restarted.spawn().unwrap();
    restarted
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"daemon-recovered"}"#)
        .unwrap();
    assert!(restarted.wait_with_output().unwrap().status.success());
    for _ in 0..20 {
        if state.join("amux.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(15));
    }
    assert!(state.join("amux.sock").exists());
    let recovered: Value = serde_json::from_slice(
        &amux(&state)
            .args(["list", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(recovered["records"].as_object().unwrap().len(), 1);
    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn watch_streams_initial_and_large_revisions_then_reports_disconnect() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let state = temp_dir("watch-large");
    let listener = UnixListener::bind(state.join("amux.sock")).unwrap();
    let missing_json = amux(&state).arg("watch").output().unwrap();
    assert!(!missing_json.status.success());
    assert!(
        String::from_utf8(missing_json.stderr)
            .unwrap()
            .contains("--json")
    );
    let views = (0..700)
        .map(|index| {
            serde_json::json!({
                "session": format!("session-{index}"),
                "last_attached": index,
                "attached": true,
                "status": "running",
                "attention": false,
                "agent_count": 1,
                "live_agent_count": 1,
                "agents": [],
                "pane": format!("%{index}"),
                "reason": "large watch snapshot payload ".repeat(8),
                "cwd": "/tmp/watch-large",
                "updated_at": index
            })
        })
        .collect::<Vec<_>>();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(request.trim(), r#"{"kind":"subscribe"}"#);
        for response in [
            serde_json::json!({"revision": 0, "views": []}),
            serde_json::json!({"revision": 1, "views": views.clone()}),
            serde_json::json!({"revision": 2, "views": views}),
        ] {
            serde_json::to_writer(&mut stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }
    });

    let child = amux(&state)
        .args(["watch", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the server to finish before reading stdout. This fills the
    // process pipe with a large snapshot and exercises a temporarily slow
    // consumer without exceeding the daemon's two-second write timeout.
    server.join().unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let lines = BufReader::new(output.stdout.as_slice())
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(lines.len(), 3);
    let snapshots = lines
        .iter()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(snapshots[0]["revision"], 0);
    assert_eq!(snapshots[1]["revision"], 1);
    assert_eq!(snapshots[2]["revision"], 2);
    assert_eq!(snapshots[1]["views"].as_array().unwrap().len(), 700);
    assert!(lines[1].len() > 64 * 1024);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("daemon subscription disconnected"));
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn watch_lazily_starts_the_daemon_and_ctrl_c_is_quiet() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::process::ExitStatusExt;

    let _lock = real_server_test_lock();
    let state = temp_dir("watch-start");
    let mut child = amux(&state)
        .args(["watch", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut initial = String::new();
    stdout.read_line(&mut initial).unwrap();
    let initial: Value = serde_json::from_str(&initial).unwrap();
    assert_eq!(initial["revision"], 0);
    assert_eq!(initial["views"], serde_json::json!([]));

    let mut hook = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    hook.args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &state)
        .env_remove("AMUX_NO_DAEMON")
        .env_remove("TMUX")
        .stdin(Stdio::piped());
    let mut hook = hook.spawn().unwrap();
    hook.stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"watch-session"}"#)
        .unwrap();
    assert!(hook.wait().unwrap().success());
    let mut update = String::new();
    stdout.read_line(&mut update).unwrap();
    let update: Value = serde_json::from_str(&update).unwrap();
    assert_eq!(update["revision"], 1);

    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(2));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty());
    assert_eq!(
        daemon_request(&state, r#"{"kind":"shutdown"}"#)["error"],
        Value::Null
    );
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn control_monitor_reconciles_an_isolated_tmux_server() {
    let _lock = real_server_test_lock();
    use std::io::Write;
    let state = temp_dir("monitor");
    let server = TmuxServer::start("monitor", "monitor");
    let socket = server.socket_path();
    let daemon_log = state.join("daemon.log");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    daemon
        .arg("daemon")
        .env("AMUX_STATE_DIR", &state)
        .env_remove("TMUX")
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&daemon_log).unwrap()));
    let mut daemon = daemon.spawn().unwrap();
    let mut subscription = connect_daemon(&state.join("amux.sock"));
    subscription.write_all(br#"{"kind":"subscribe"}"#).unwrap();
    subscription.write_all(b"\n").unwrap();
    let subscription = monitor_updates(subscription, daemon_log, state.clone());
    let pane = String::from_utf8(
        server
            .command()
            .args(["list-panes", "-t", "monitor", "-F", "#{pane_id}"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let mut hook = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    hook.args(["event", "--agent", "codex", "--event", "PostToolUse"])
        .env("AMUX_STATE_DIR", &state)
        .env("TMUX", format!("{socket},1,1"))
        .env("TMUX_PANE", pane.trim())
        .stdin(Stdio::piped());
    let mut hook = hook.spawn().unwrap();
    hook.stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"session_id":"monitor-codex"}"#)
        .unwrap();
    assert!(hook.wait_with_output().unwrap().status.success());
    subscription.wait_for(|response| {
        response["topology"]["connected"] == true
            && response["topology"]["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"].as_str() == Some("monitor"))
    });
    let records: Value = serde_json::from_slice(
        &Command::new(env!("CARGO_BIN_EXE_amux-rs"))
            .args(["list", "--json"])
            .env("AMUX_STATE_DIR", &state)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(
        records["records"]
            .as_object()
            .unwrap()
            .values()
            .any(|record| {
                record["agent"] == "codex"
                    && record["tmux_session"] == "monitor"
                    && record["tmux_pane"] == pane.trim()
            })
    );
    let agent_commands = temp_dir("monitor-agents");
    let agent_source = agent_commands.join("agent.rs");
    fs::write(
        &agent_source,
        "fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }",
    )
    .unwrap();
    let codex_command = agent_commands.join("codex");
    assert!(
        Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()))
            .args([
                agent_source.to_str().unwrap(),
                "-o",
                codex_command.to_str().unwrap()
            ])
            .status()
            .unwrap()
            .success()
    );
    let claude_command = agent_commands.join("claude");
    fs::copy(&codex_command, &claude_command).unwrap();
    assert!(
        server
            .command()
            .args([
                "new-session",
                "-d",
                "-s",
                "agents",
                codex_command.to_str().unwrap(),
                "60",
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        server
            .command()
            .args([
                "split-window",
                "-d",
                "-t",
                "agents",
                claude_command.to_str().unwrap(),
                "60",
            ])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["name"].as_str() == Some("agents"))
            })
    });
    let panes = String::from_utf8(
        server
            .command()
            .args([
                "list-panes",
                "-t",
                "agents",
                "-F",
                "#{pane_id}|#{pane_current_command}",
            ])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    for (agent, event, session_id) in [
        ("codex", "PostToolUse", "agent-codex"),
        ("claude", "Notification", "agent-claude"),
    ] {
        let pane = panes.lines().find_map(|line| {
            let (pane, command) = line.split_once('|')?;
            (command == agent).then_some(pane)
        });
        assert!(
            pane.is_some(),
            "tmux did not report {agent} as the pane command: {panes}"
        );
        let pane = pane.unwrap();
        let mut agent_hook = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
        agent_hook
            .args(["event", "--agent", agent, "--event", event])
            .env("AMUX_STATE_DIR", &state)
            .env("TMUX", format!("{socket},1,1"))
            .env("TMUX_PANE", pane)
            .stdin(Stdio::piped());
        let mut agent_hook = agent_hook.spawn().unwrap();
        agent_hook
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!(r#"{{"session_id":"{session_id}"}}"#).as_bytes())
            .unwrap();
        assert!(agent_hook.wait_with_output().unwrap().status.success());
    }
    let agent_sessions: Value = serde_json::from_slice(
        &Command::new(env!("CARGO_BIN_EXE_amux-rs"))
            .args(["sessions", "--json"])
            .env("AMUX_STATE_DIR", &state)
            .env_remove("TMUX")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let agents = agent_sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session"] == "agents")
        .unwrap();
    assert_eq!(agents["agent_count"], 2);
    assert_eq!(agents["live_agent_count"], 2);
    assert_eq!(agents["status"], "attention");
    assert_eq!(agents["agents"][0]["agent"], "claude");
    let expected_pane = agents["agents"][0]["pane"].as_str().unwrap().to_owned();

    // Keep a real tmux client attached to the older session, then execute the
    // command bound by the plugin and prove it selects the newest live target.
    let mut client_command = server.command();
    client_command
        .args(["-C", "attach-session", "-t", "monitor"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut client = client_command.spawn().unwrap();
    let client_name = {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let output = server
                .command()
                .args(["list-clients", "-F", "#{client_name}|#{session_name}"])
                .output()
                .unwrap();
            let clients = String::from_utf8_lossy(&output.stdout);
            if let Some(name) = clients.lines().find_map(|line| {
                let (name, session) = line.split_once('|')?;
                (session == "monitor").then(|| name.to_owned())
            }) {
                break name;
            }
            assert!(
                Instant::now() < deadline,
                "control client did not attach: {clients}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    };
    let switched = Command::new(env!("CARGO_BIN_EXE_amux-rs"))
        .arg("next-attention")
        .env("AMUX_STATE_DIR", &state)
        .env("TMUX", format!("{socket},1,1"))
        .env("AMUX_TMUX_CLIENT", &client_name)
        .output()
        .unwrap();
    assert!(
        switched.status.success(),
        "next-attention failed: {}",
        String::from_utf8_lossy(&switched.stderr)
    );
    let client_target = String::from_utf8(
        server
            .command()
            .args([
                "display-message",
                "-p",
                "-c",
                &client_name,
                "#{session_name}|#{pane_id}",
            ])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(client_target.trim(), format!("agents|{expected_pane}"));
    let _ = client.stdin.take();
    let _ = client.kill();
    let _ = client.wait();
    assert!(
        server
            .command()
            .args(["new-window", "-d", "-t", "monitor"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["panes"]
            .as_array()
            .is_some_and(|rows| rows.len() >= 2)
    });
    assert!(
        server
            .command()
            .args(["new-session", "-d", "-s", "lifecycle"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["name"].as_str() == Some("lifecycle"))
            })
    });
    assert!(
        server
            .command()
            .args(["rename-session", "-t", "lifecycle", "renamed"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["name"].as_str() == Some("renamed"))
            })
    });
    let monitor_window = String::from_utf8(
        server
            .command()
            .args(["list-windows", "-t", "monitor", "-F", "#{window_id}"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .lines()
    .next()
    .unwrap()
    .to_owned();
    assert!(
        server
            .command()
            .args(["link-window", "-s", &monitor_window, "-t", "renamed"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["panes"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["session"].as_str() == Some("renamed"))
            })
    });
    assert!(
        server
            .command()
            .args(["kill-session", "-t", "renamed"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                !rows
                    .iter()
                    .any(|row| row["name"].as_str() == Some("renamed"))
            })
    });
    assert!(
        server
            .command()
            .args(["kill-server"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| response["topology"]["connected"] == false);
    assert!(
        server
            .command()
            .args(["new-session", "-d", "-s", "recovered"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| {
        response["topology"]["connected"] == true
            && response["topology"]["sessions"]
                .as_array()
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["name"].as_str() == Some("recovered"))
                })
    });
    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    let _ = daemon.wait();
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(agent_commands).unwrap();
}

#[test]
#[cfg(unix)]
fn tmux_plugin_loads_native_picker_without_status_wiring() {
    let _lock = real_server_test_lock();
    let server = TmuxServer::start("plugin", "plugin");
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("amux.tmux");
    assert!(
        server
            .command()
            .args(["set-option", "-g", "@amux-next-attention-key", "N"])
            .status()
            .unwrap()
            .success()
    );
    let _ = server
        .command()
        .args(["set-option", "-gu", "@amux-status-command"])
        .status();
    assert!(
        server
            .command()
            .args(["run-shell", plugin.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let picker = String::from_utf8(
        server
            .command()
            .args(["list-keys", "-T", "prefix"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(
        picker.contains("bin/amux"),
        "unexpected picker binding: {picker}"
    );
    assert!(
        picker.contains("run-shell"),
        "unexpected picker binding: {picker}"
    );
    assert!(
        picker.contains("AMUX_TMUX_CLIENT=#{client_tty}"),
        "unexpected picker binding: {picker}"
    );
    assert!(picker.contains("bin/amux next-attention"));
    let status_option = server
        .command()
        .args(["show-option", "-gqv", "@amux-status-command"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&status_option.stdout).contains("bin/amux status"));
}

/// A response larger than the socket send buffer must arrive whole. macOS and
/// BSD inherit the listener's O_NONBLOCK on accepted sockets, which truncated
/// anything past 8 KiB mid-`write_all`; Linux does not, so this only fails on
/// the platforms that inherit.
#[cfg(unix)]
#[test]
fn a_response_larger_than_the_socket_buffer_arrives_intact() {
    let _lock = real_server_test_lock();
    let state = temp_dir("daemon-large-response");
    let server = TmuxServer::start("large-response", "large-response");
    for index in 0..64 {
        assert!(
            server
                .command()
                .args([
                    "new-session",
                    "-d",
                    "-t",
                    "large-response",
                    "-s",
                    &format!("large-response-linked-session-{index:02}"),
                ])
                .status()
                .unwrap()
                .success()
        );
    }
    let socket = server.socket_path();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    daemon
        .arg("daemon")
        .env("AMUX_STATE_DIR", &state)
        .env("TMUX", format!("{socket},1,1"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = daemon.spawn().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let response = loop {
        let response = daemon_request(&state, r#"{"kind":"health"}"#);
        if response["topology"]["sessions"]
            .as_array()
            .is_some_and(|sessions| sessions.len() >= 65)
        {
            break response;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not publish the large topology: {response}"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(
        response.get("error").is_none(),
        "health reported an error: {response}"
    );
    let encoded = response.to_string();
    assert!(
        encoded.len() > 8192,
        "probe did not exceed the 8 KiB socket buffer ({} bytes); the test would \
         pass even with the truncation bug",
        encoded.len()
    );
    assert!(response.get("state").is_none());
    assert!(response["topology"]["sessions"].as_array().unwrap().len() >= 65);

    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    let _ = daemon.wait();
    fs::remove_dir_all(state).unwrap();
}
