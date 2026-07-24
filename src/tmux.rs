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
    pub sessions: Vec<String>,
    pub panes: Vec<String>,
    pub reconciled_at: i64,
    pub connected: bool,
    pub error: String,
}

trait TopologyProvider {
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Result<Topology, String>> + '_>>;
}

struct ControlProvider<'a> {
    client: &'a TokioClient,
}

impl TopologyProvider for ControlProvider<'_> {
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Result<Topology, String>> + '_>> {
        Box::pin(async move {
            let sessions = self
                .client
                .command("list-sessions -F '#{session_id}|#{session_last_attached}|#{session_name}|#{session_attached}'")
                .await
                .map_err(|error| error.to_string())?;
            let panes = self
                .client
                .command("list-panes -a -F '#{session_id}|#{session_name}|#{window_id}|#{pane_id}|#{pane_current_command}|#{pane_pid}|#{pane_title}|#{pane_current_path}'")
                .await
                .map_err(|error| error.to_string())?;
            Ok(Topology {
                sessions: sessions.lines,
                panes: panes.lines,
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
                    sessions: vec!["$1|1|alpha|0".to_owned()],
                    panes: vec!["$1|alpha|@1|%1|codex|1|codex|/tmp".to_owned()],
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
}
