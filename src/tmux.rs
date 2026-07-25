use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tmuxctl::TokioClient;
use tokio::process::Command;
use tokio::time::{Duration, sleep};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Topology {
    pub sessions: Vec<TmuxSession>,
    pub panes: Vec<Pane>,
    pub reconciled_at: i64,
    pub connected: bool,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TmuxSession {
    pub id: String,
    pub name: String,
    pub last_attached: i64,
    pub attached: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Pane {
    pub session: String,
    pub window: String,
    pub pane: String,
    pub command: String,
    pub title: String,
    pub cwd: String,
}

const FIELD_SEPARATOR: char = '\u{1f}';
const INVOKING_CLIENT_ENV: &str = "AMUX_TMUX_CLIENT";

trait TopologyProvider {
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Result<Topology, String>> + '_>>;
}

struct ControlProvider<'a> {
    client: &'a TokioClient,
}

impl TopologyProvider for ControlProvider<'_> {
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Result<Topology, String>> + '_>> {
        Box::pin(async move {
            let session_format = format!(
                "#{{session_id}}{FIELD_SEPARATOR}#{{session_last_attached}}{FIELD_SEPARATOR}#{{session_name}}{FIELD_SEPARATOR}#{{session_attached}}"
            );
            let pane_format = format!(
                "#{{session_id}}{FIELD_SEPARATOR}#{{session_name}}{FIELD_SEPARATOR}#{{window_id}}{FIELD_SEPARATOR}#{{pane_id}}{FIELD_SEPARATOR}#{{pane_current_command}}{FIELD_SEPARATOR}#{{pane_pid}}{FIELD_SEPARATOR}#{{pane_title}}{FIELD_SEPARATOR}#{{pane_current_path}}"
            );
            let sessions = self
                .client
                .command(&format!("list-sessions -F '{session_format}'"))
                .await
                .map_err(|error| error.to_string())?;
            let panes = self
                .client
                .command(&format!("list-panes -a -F '{pane_format}'"))
                .await
                .map_err(|error| error.to_string())?;
            Ok(Topology {
                sessions: parse_control_sessions(&sessions.lines)?,
                panes: parse_control_panes(&panes.lines)?,
                reconciled_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                connected: true,
                error: String::new(),
            })
        })
    }
}

pub fn direct_topology() -> Topology {
    let session_format = format!(
        "#{{session_last_attached}}{FIELD_SEPARATOR}#{{session_name}}{FIELD_SEPARATOR}#{{session_attached}}"
    );
    let pane_format = format!(
        "#{{session_name}}{FIELD_SEPARATOR}#{{pane_id}}{FIELD_SEPARATOR}#{{pane_current_command}}{FIELD_SEPARATOR}#{{pane_pid}}{FIELD_SEPARATOR}#{{pane_title}}{FIELD_SEPARATOR}#{{pane_current_path}}"
    );
    Topology {
        sessions: parse_direct_sessions(&tmux_lines("list-sessions", &session_format))
            .unwrap_or_default(),
        panes: parse_direct_panes(&tmux_lines("list-panes -a", &pane_format)).unwrap_or_default(),
        ..Topology::default()
    }
}

fn tmux_lines(command: &str, format: &str) -> Vec<String> {
    std::process::Command::new("tmux")
        .args(command.split(' '))
        .arg("-F")
        .arg(format)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_direct_sessions(lines: &[String]) -> Result<Vec<TmuxSession>, String> {
    lines
        .iter()
        .map(|line| {
            let fields: Vec<_> = line.split(FIELD_SEPARATOR).collect();
            if fields.len() != 3 || fields[1].is_empty() {
                return Err(format!("invalid tmux session record: {line:?}"));
            }
            Ok(TmuxSession {
                id: String::new(),
                last_attached: fields[0].parse().unwrap_or(0),
                name: fields[1].to_owned(),
                attached: fields[2] == "1",
            })
        })
        .collect()
}

fn parse_control_sessions(lines: &[String]) -> Result<Vec<TmuxSession>, String> {
    lines
        .iter()
        .map(|line| {
            let fields: Vec<_> = line.split(FIELD_SEPARATOR).collect();
            if fields.len() != 4 || fields[2].is_empty() {
                return Err(format!("invalid tmux control session record: {line:?}"));
            }
            Ok(TmuxSession {
                id: fields[0].to_owned(),
                last_attached: fields[1].parse().unwrap_or(0),
                name: fields[2].to_owned(),
                attached: fields[3] == "1",
            })
        })
        .collect()
}

fn parse_direct_panes(lines: &[String]) -> Result<Vec<Pane>, String> {
    lines
        .iter()
        .map(|line| {
            let fields: Vec<_> = line.split(FIELD_SEPARATOR).collect();
            if fields.len() != 6 {
                return Err(format!("invalid tmux pane record: {line:?}"));
            }
            Ok(Pane {
                session: fields[0].to_owned(),
                pane: fields[1].to_owned(),
                command: fields[2].to_owned(),
                title: fields[4].to_owned(),
                cwd: fields[5].to_owned(),
                ..Pane::default()
            })
        })
        .collect()
}

fn parse_control_panes(lines: &[String]) -> Result<Vec<Pane>, String> {
    lines
        .iter()
        .map(|line| {
            let fields: Vec<_> = line.split(FIELD_SEPARATOR).collect();
            if fields.len() != 8 {
                return Err(format!("invalid tmux control pane record: {line:?}"));
            }
            Ok(Pane {
                session: fields[1].to_owned(),
                window: fields[2].to_owned(),
                pane: fields[3].to_owned(),
                command: fields[4].to_owned(),
                title: fields[6].to_owned(),
                cwd: fields[7].to_owned(),
            })
        })
        .collect()
}

pub fn server_from_env() -> Option<PathBuf> {
    env::var_os("TMUX")
        .and_then(|value| value.into_string().ok())
        .and_then(|value| {
            value
                .split(',')
                .next()
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
}

/// Move the invoking tmux client directly to a pane (or, if no pane is known,
/// to a session). A pane target makes tmux change the session, window, and
/// active pane atomically, which is required when the picker runs in a popup.
pub fn switch_client(session: &str, pane: &str) -> Result<(), String> {
    let targets = switch_targets(session, pane);
    if targets.is_empty() {
        return Ok(());
    }
    let client = env::var(INVOKING_CLIENT_ENV)
        .ok()
        .filter(|client| !client.is_empty());
    let mut failure = None;
    for target in targets {
        let output = switch_client_command(target, client.as_deref())
            .output()
            .map_err(|error| format!("could not switch tmux client: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        failure = Some(if detail.is_empty() {
            format!("tmux could not switch to {target}")
        } else {
            format!("tmux could not switch to {target}: {detail}")
        });
    }
    Err(failure.expect("a non-empty switch target"))
}

fn switch_targets<'a>(session: &'a str, pane: &'a str) -> Vec<&'a str> {
    if pane.is_empty() {
        (!session.is_empty())
            .then_some(session)
            .into_iter()
            .collect()
    } else if session.is_empty() || session == pane {
        vec![pane]
    } else {
        vec![pane, session]
    }
}

fn switch_client_command(target: &str, client: Option<&str>) -> std::process::Command {
    let mut command = std::process::Command::new("tmux");
    command.arg("switch-client");
    if let Some(client) = client {
        command.args(["-c", client]);
    }
    command.args(["-t", target]);
    command
}

pub fn spawn(
    stop: Arc<AtomicBool>,
    server: Option<PathBuf>,
    publish: impl Fn(Topology) + Send + Sync + 'static,
) {
    let publish: Arc<dyn Fn(Topology) + Send + Sync> = Arc::new(publish);
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return,
        };
        runtime.block_on(async move {
            let mut backoff = Duration::from_millis(100);
            while !stop.load(Ordering::Relaxed) {
                let result =
                    monitor_once(Arc::clone(&stop), server.as_deref(), Arc::clone(&publish)).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(error) = result {
                    publish(Topology {
                        connected: false,
                        error,
                        ..Topology::default()
                    });
                }
                sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        });
    });
}

async fn monitor_once(
    stop: Arc<AtomicBool>,
    server: Option<&std::path::Path>,
    publish: Arc<dyn Fn(Topology) + Send + Sync>,
) -> Result<(), String> {
    let mut command = Command::new("tmux");
    if let Some(server) = server {
        command.arg("-S").arg(server);
    }
    command
        .arg("-C")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "tmux control stdout was not piped".to_owned())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "tmux control stdin was not piped".to_owned())?;
    let mut client = TokioClient::with_transport(stdout, stdin);
    let mut notifications = client
        .events()
        .ok_or_else(|| "tmux control event stream already taken".to_owned())?;
    client
        .command("refresh-client -f no-output")
        .await
        .map_err(|error| error.to_string())?;
    let provider = ControlProvider { client: &client };
    reconcile(&provider, &publish).await?;
    let mut periodic = tokio::time::interval(Duration::from_secs(30));
    periodic.tick().await;
    loop {
        tokio::select! {
            _ = sleep(Duration::from_secs(1)) => {
                if stop.load(Ordering::Relaxed) { let _ = child.kill().await; return Ok(()); }
            }
            notice = notifications.recv() => {
                if notice.is_none() { return Err("tmux control connection closed".to_owned()); }
                sleep(Duration::from_millis(20)).await;
                loop {
                    match notifications.try_recv() {
                        Ok(_) => {}
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            return Err("tmux control connection closed".to_owned());
                        }
                    }
                }
                reconcile(&provider, &publish).await?;
            }
            _ = periodic.tick() => reconcile(&provider, &publish).await?,
        }
    }
}

async fn reconcile(
    provider: &impl TopologyProvider,
    publish: &Arc<dyn Fn(Topology) + Send + Sync>,
) -> Result<(), String> {
    publish(provider.snapshot().await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticProvider(Topology);

    impl TopologyProvider for StaticProvider {
        fn snapshot(&self) -> Pin<Box<dyn Future<Output = Result<Topology, String>> + '_>> {
            Box::pin(async move { Ok(self.0.clone()) })
        }
    }

    #[test]
    fn provider_test_double_returns_a_complete_snapshot() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let topology = runtime
            .block_on(
                StaticProvider(Topology {
                    sessions: vec![TmuxSession {
                        id: "$1".to_owned(),
                        name: "alpha".to_owned(),
                        last_attached: 1,
                        attached: false,
                    }],
                    panes: vec![Pane {
                        session: "alpha".to_owned(),
                        window: "@1".to_owned(),
                        pane: "%1".to_owned(),
                        command: "codex".to_owned(),
                        title: "codex".to_owned(),
                        cwd: "/tmp".to_owned(),
                    }],
                    reconciled_at: 1,
                    connected: true,
                    error: String::new(),
                })
                .snapshot(),
            )
            .unwrap();
        assert_eq!(topology.sessions.len(), 1);
        assert_eq!(topology.panes.len(), 1);
    }

    #[test]
    fn parser_keeps_pipes_inside_tmux_metadata() {
        let sessions = parse_control_sessions(&[format!(
            "$1{FIELD_SEPARATOR}1{FIELD_SEPARATOR}alpha|beta{FIELD_SEPARATOR}1"
        )])
        .unwrap();
        let panes = parse_control_panes(&[format!(
            "$1{FIELD_SEPARATOR}alpha|beta{FIELD_SEPARATOR}@1{FIELD_SEPARATOR}%1{FIELD_SEPARATOR}codex|agent{FIELD_SEPARATOR}1{FIELD_SEPARATOR}title|detail{FIELD_SEPARATOR}/tmp/a|b"
        )])
        .unwrap();
        assert_eq!(sessions[0].name, "alpha|beta");
        assert_eq!(panes[0].session, "alpha|beta");
        assert_eq!(panes[0].command, "codex|agent");
        assert_eq!(panes[0].title, "title|detail");
        assert_eq!(panes[0].cwd, "/tmp/a|b");
    }

    #[test]
    fn switch_targets_the_client_that_opened_the_picker() {
        let command = switch_client_command("%9", Some("/dev/ttys001"));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(args, ["switch-client", "-c", "/dev/ttys001", "-t", "%9"]);
    }

    #[test]
    fn switch_falls_back_to_the_session_when_a_pane_is_stale() {
        assert_eq!(switch_targets("shell", "%gone"), ["%gone", "shell"]);
        assert_eq!(switch_targets("shell", ""), ["shell"]);
    }
}
