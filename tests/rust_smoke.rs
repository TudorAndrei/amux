use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "amux-rs-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
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
fn daemon_request(state: &Path, request: &str) -> Value {
    use std::io::{BufRead, BufReader, Write};
    let socket = state.join("amux.sock");
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[cfg(unix)]
fn wait_for_monitor_update(
    reader: &mut std::io::BufReader<UnixStream>,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    use std::io::BufRead;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => panic!("tmux monitor subscription closed"),
            Ok(_) => {
                let response: Value = serde_json::from_str(&line).unwrap();
                if predicate(&response) {
                    return response;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("failed to read tmux monitor update: {error}"),
        }
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
  list-sessions) printf '500|multi-agent|0\n' ;;
  list-panes)
    printf 'multi-agent|%%20|codex|500|codex|/tmp/multi-codex\n'
    printf 'multi-agent|%%21|claude|501|claude|/tmp/multi-claude\n' ;;
  refresh-client) ;;
esac
"##,
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
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
    assert!(!state.join("state.lock").exists());
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
    assert_eq!(
        amux(&state)
            .env("PATH", &path)
            .env("AMUX_COLOR", "0")
            .arg("status")
            .output()
            .unwrap()
            .stdout,
        b"\xe2\x96\xb2 1"
    );
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(fake_bin).unwrap();
}

#[test]
#[cfg(unix)]
fn lazy_daemon_persists_events_and_serves_revisions() {
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
    subscription.shutdown(std::net::Shutdown::Write).unwrap();
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
    assert_eq!(recovered["records"].as_object().unwrap().len(), 3);
    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    fs::remove_dir_all(state).unwrap();
}

#[test]
#[cfg(unix)]
fn control_monitor_reconciles_an_isolated_tmux_server() {
    use std::io::{BufRead, BufReader, Write};
    let state = temp_dir("monitor");
    let socket_name = format!(
        "amux-monitor-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    assert!(
        Command::new("tmux")
            .args(["-L", &socket_name, "new-session", "-d", "-s", "monitor"])
            .status()
            .unwrap()
            .success()
    );
    let socket = String::from_utf8(
        Command::new("tmux")
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
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    daemon
        .arg("daemon")
        .env("AMUX_STATE_DIR", &state)
        .env("TMUX", format!("{socket},1,1"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = daemon.spawn().unwrap();
    for _ in 0..40 {
        if state.join("amux.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let mut subscription = UnixStream::connect(state.join("amux.sock")).unwrap();
    subscription
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    subscription.write_all(br#"{"kind":"subscribe"}"#).unwrap();
    subscription.write_all(b"\n").unwrap();
    subscription.shutdown(std::net::Shutdown::Write).unwrap();
    let mut subscription = BufReader::new(subscription);
    let mut saw_snapshot = false;
    for _ in 0..4 {
        let mut line = String::new();
        subscription.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        if response["topology"]["connected"] == true
            && response["topology"]["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row.as_str().unwrap().contains("monitor"))
        {
            saw_snapshot = true;
            break;
        }
    }
    assert!(saw_snapshot);
    let pane = String::from_utf8(
        Command::new("tmux")
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
    let cached_status = daemon_request(&state, r#"{"kind":"status"}"#);
    assert!(
        cached_status["status"]
            .as_str()
            .is_some_and(|status| !status.is_empty())
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
        Command::new("tmux")
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
        Command::new("tmux")
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
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.as_str().unwrap().contains("agents"))
            })
    });
    let panes = String::from_utf8(
        Command::new("tmux")
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
        Command::new("tmux")
            .args(["-S", &socket, "new-window", "-d", "-t", "monitor"])
            .status()
            .unwrap()
            .success()
    );
    let mut saw_new_pane = false;
    for _ in 0..4 {
        let mut line = String::new();
        subscription.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        if response["topology"]["panes"]
            .as_array()
            .is_some_and(|rows| rows.len() >= 2)
        {
            saw_new_pane = true;
            break;
        }
    }
    assert!(saw_new_pane);
    assert!(
        Command::new("tmux")
            .args(["-S", &socket, "new-session", "-d", "-s", "lifecycle"])
            .status()
            .unwrap()
            .success()
    );
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.as_str().unwrap().contains("lifecycle"))
            })
    });
    assert!(
        Command::new("tmux")
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
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.as_str().unwrap().contains("renamed"))
            })
    });
    let monitor_window = String::from_utf8(
        Command::new("tmux")
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
        Command::new("tmux")
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
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["panes"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.as_str().unwrap().contains("|renamed|"))
            })
    });
    assert!(
        Command::new("tmux")
            .args(["-S", &socket, "kill-session", "-t", "renamed"])
            .status()
            .unwrap()
            .success()
    );
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["sessions"]
            .as_array()
            .is_some_and(|rows| {
                !rows
                    .iter()
                    .any(|row| row.as_str().unwrap().contains("renamed"))
            })
    });
    assert!(
        Command::new("tmux")
            .args(["-S", &socket, "kill-server"])
            .status()
            .unwrap()
            .success()
    );
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["connected"] == false
    });
    assert!(
        Command::new("tmux")
            .args(["-L", &socket_name, "new-session", "-d", "-s", "recovered"])
            .status()
            .unwrap()
            .success()
    );
    wait_for_monitor_update(&mut subscription, |response| {
        response["topology"]["connected"] == true
            && response["topology"]["sessions"]
                .as_array()
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row.as_str().unwrap().contains("recovered"))
                })
    });
    let _ = daemon_request(&state, r#"{"kind":"shutdown"}"#);
    let _ = daemon.wait();
    let _ = Command::new("tmux")
        .args(["-S", &socket, "kill-server"])
        .status();
    fs::remove_dir_all(state).unwrap();
    fs::remove_dir_all(agent_commands).unwrap();
}

#[test]
#[cfg(unix)]
fn tmux_plugin_loads_native_picker_and_status_commands() {
    let socket_name = format!(
        "amux-plugin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    assert!(
        Command::new("tmux")
            .args(["-L", &socket_name, "new-session", "-d", "-s", "plugin"])
            .status()
            .unwrap()
            .success()
    );
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("amux.tmux");
    let legacy_status = plugin.parent().unwrap().join("scripts/status.sh");
    assert!(
        Command::new("tmux")
            .args(["-L", &socket_name, "set-option", "-g", "@amux-status", "on"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("tmux")
            .args([
                "-L",
                &socket_name,
                "set-option",
                "-g",
                "@amux-status-command",
                legacy_status.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("tmux")
            .args([
                "-L",
                &socket_name,
                "set-option",
                "-g",
                "status-right",
                &format!("#({}) host", legacy_status.display()),
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("tmux")
            .args(["-L", &socket_name, "run-shell", plugin.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let mut picker = String::new();
    for _ in 0..20 {
        picker = String::from_utf8(
            Command::new("tmux")
                .args(["-L", &socket_name, "list-keys", "-T", "prefix"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        if picker.contains("bin/amux picker") {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        picker.contains("bin/amux picker"),
        "unexpected picker binding: {picker}"
    );
    let status_command = String::from_utf8(
        Command::new("tmux")
            .args([
                "-L",
                &socket_name,
                "show-option",
                "-gqv",
                "@amux-status-command",
            ])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(status_command.contains("bin/amux status"));
    let status_right = String::from_utf8(
        Command::new("tmux")
            .args(["-L", &socket_name, "show-option", "-gqv", "status-right"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(status_right.contains("bin/amux status"));
    assert!(!status_right.contains("scripts/status.sh"));
    assert!(status_right.contains("host"));
    let _ = Command::new("tmux")
        .args(["-L", &socket_name, "kill-server"])
        .status();
}
