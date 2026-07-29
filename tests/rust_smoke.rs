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
}

#[cfg(unix)]
impl MonitorSubscription {
    fn wait_for(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut last = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.updates.recv_timeout(remaining) {
                Ok(Ok(response)) if predicate(&response) => return response,
                Ok(Ok(response)) => last = Some(response),
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
fn monitor_updates(stream: UnixStream, daemon_log: PathBuf) -> MonitorSubscription {
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

#[test]
fn cli_clear_doctor_and_option_contracts_are_preserved() {
    let state = temp_dir("cli-contract");
    let fake_bin = temp_dir("cli-tmux");
    fake_tmux(&fake_bin);
    let path = format!("{}:{}", fake_bin.display(), std::env::var("PATH").unwrap());
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
        .arg("doctor")
        .output()
        .unwrap();
    assert!(doctor.status.success());
    let doctor = String::from_utf8(doctor.stdout).unwrap();
    assert!(doctor.contains("tmux: ok (tmux 3.5)"));
    assert!(doctor.contains("state: v1 compatible"));
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
fn fixture_normalization_uses_the_v1_schema() {
    let state = temp_dir("fixtures");
    event(&state, "codex", &[], fixture("codex-permission.json"));
    event(&state, "claude", &[], fixture("claude-stop.json"));
    event(&state, "opencode", &[], fixture("opencode-idle.json"));
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
    assert!(
        value["records"]
            .as_object()
            .unwrap()
            .values()
            .any(|record| record["agent"] == "opencode" && record["attention"] == true)
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
    assert_eq!(
        serde_json::from_str::<Value>(&initial).unwrap()["revision"],
        1
    );
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
    assert_eq!(update["state"]["records"].as_object().unwrap().len(), 2);
    assert!(amux(&state).arg("clear").status().unwrap().success());
    let mut cleared = String::new();
    subscription.read_line(&mut cleared).unwrap();
    let cleared: Value = serde_json::from_str(&cleared).unwrap();
    assert_eq!(cleared["revision"], 3);
    assert!(cleared["state"]["records"].as_object().unwrap().is_empty());
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
fn control_monitor_reconciles_an_isolated_tmux_server() {
    let _lock = real_server_test_lock();
    use std::io::Write;
    let state = temp_dir("monitor");
    let tmux_tmpdir = tmux_temp_dir("monitor");
    let socket_name = format!(
        "amux-monitor-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    assert!(
        tmux_command(&tmux_tmpdir)
            .args(["-L", &socket_name, "new-session", "-d", "-s", "monitor"])
            .status()
            .unwrap()
            .success()
    );
    let socket = String::from_utf8(
        tmux_command(&tmux_tmpdir)
            .args([
                "-L",
                &socket_name,
                "display-message",
                "-p",
                "#{socket_path}",
            ])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let socket = socket.trim().to_owned();
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
    let subscription = monitor_updates(subscription, daemon_log);
    let pane = String::from_utf8(
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
                "list-panes",
                "-t",
                "monitor",
                "-F",
                "#{pane_id}",
            ])
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
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
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
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
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
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
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
    assert!(
        tmux_command(&tmux_tmpdir)
            .args(["-S", &socket, "new-window", "-d", "-t", "monitor"])
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
        tmux_command(&tmux_tmpdir)
            .args(["-S", &socket, "new-session", "-d", "-s", "lifecycle"])
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
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
                "rename-session",
                "-t",
                "lifecycle",
                "renamed",
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
                    .any(|row| row["name"].as_str() == Some("renamed"))
            })
    });
    let monitor_window = String::from_utf8(
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
                "list-windows",
                "-t",
                "monitor",
                "-F",
                "#{window_id}",
            ])
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
        tmux_command(&tmux_tmpdir)
            .args([
                "-S",
                &socket,
                "link-window",
                "-s",
                &monitor_window,
                "-t",
                "renamed",
            ])
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
        tmux_command(&tmux_tmpdir)
            .args(["-S", &socket, "kill-session", "-t", "renamed"])
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
        tmux_command(&tmux_tmpdir)
            .args(["-S", &socket, "kill-server"])
            .status()
            .unwrap()
            .success()
    );
    subscription.wait_for(|response| response["topology"]["connected"] == false);
    assert!(
        tmux_command(&tmux_tmpdir)
            .args(["-L", &socket_name, "new-session", "-d", "-s", "recovered"])
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
    let _ = tmux_command(&tmux_tmpdir)
        .args(["-S", &socket, "kill-server"])
        .status();
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(tmux_tmpdir).unwrap();
    fs::remove_dir_all(agent_commands).unwrap();
}

#[test]
#[cfg(unix)]
fn tmux_plugin_loads_native_picker_without_status_wiring() {
    let _lock = real_server_test_lock();
    let tmux_tmpdir = tmux_temp_dir("plugin");
    let socket_name = format!("amux-plugin-{}", std::process::id());
    assert!(
        tmux_command(&tmux_tmpdir)
            .args(["-L", &socket_name, "new-session", "-d", "-s", "plugin"])
            .status()
            .unwrap()
            .success()
    );
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("amux.tmux");
    assert!(
        tmux_command(&tmux_tmpdir)
            .args([
                "-L",
                &socket_name,
                "set-option",
                "-g",
                "@amux-next-attention-key",
                "N"
            ])
            .status()
            .unwrap()
            .success()
    );
    let _ = tmux_command(&tmux_tmpdir)
        .args([
            "-L",
            &socket_name,
            "set-option",
            "-gu",
            "@amux-status-command",
        ])
        .status();
    assert!(
        tmux_command(&tmux_tmpdir)
            .args(["-L", &socket_name, "run-shell", plugin.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let picker = String::from_utf8(
        tmux_command(&tmux_tmpdir)
            .args(["-L", &socket_name, "list-keys", "-T", "prefix"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(picker.contains("bin/amux picker"));
    assert!(picker.contains("bin/amux next-attention"));
    let status_option = tmux_command(&tmux_tmpdir)
        .args([
            "-L",
            &socket_name,
            "show-option",
            "-gqv",
            "@amux-status-command",
        ])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&status_option.stdout).contains("bin/amux status"));
    let _ = tmux_command(&tmux_tmpdir)
        .args(["-L", &socket_name, "kill-server"])
        .status();
    fs::remove_dir_all(tmux_tmpdir).unwrap();
}
