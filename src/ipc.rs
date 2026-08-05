use crate::model::SessionView;
use crate::tmux::Topology;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub(crate) const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    Event { request: Box<HookRequest> },
    Clear,
    Subscribe,
    Ping,
    Shutdown,
    Health,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HookRequest {
    pub agent: String,
    pub event: String,
    pub status: String,
    pub attention: String,
    pub reason: String,
    pub raw: Value,
    pub cwd: String,
    pub tmux_pane: String,
    pub tmux_session: String,
    pub tmux_window: String,
    pub tmux_server: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgement {
    pub revision: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HealthResponse {
    revision: u64,
    topology: Topology,
    views: Vec<SessionView>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Rejection {
    revision: u64,
    error: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireResult<T> {
    Success(T),
    Rejected(Rejection),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    Unavailable(String),
    Rejected(String),
    Protocol(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "daemon unavailable: {message}"),
            Self::Rejected(message) => write!(formatter, "daemon rejected request: {message}"),
            Self::Protocol(message) => write!(formatter, "daemon protocol error: {message}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionUpdate {
    pub revision: u64,
    pub views: Vec<SessionView>,
}

pub fn socket_path(config: &crate::config::Config) -> PathBuf {
    config.state_dir.join("amux.sock")
}

fn unavailable(error: std::io::Error) -> ClientError {
    ClientError::Unavailable(error.to_string())
}

fn protocol(error: impl ToString) -> ClientError {
    ClientError::Protocol(error.to_string())
}

fn decode_response<T: DeserializeOwned>(line: &str) -> Result<T, ClientError> {
    match serde_json::from_str::<WireResult<T>>(line).map_err(protocol)? {
        WireResult::Success(response) => Ok(response),
        WireResult::Rejected(Rejection { revision: 0, error }) => Err(ClientError::Rejected(error)),
        WireResult::Rejected(Rejection { revision, .. }) => Err(ClientError::Protocol(format!(
            "rejection response used nonzero revision {revision}"
        ))),
    }
}

fn one_shot<T: DeserializeOwned>(
    config: &crate::config::Config,
    request: &Request,
) -> Result<T, ClientError> {
    let stream = UnixStream::connect(socket_path(config)).map_err(unavailable)?;
    one_shot_stream(stream, request, ONE_SHOT_TIMEOUT)
}

fn one_shot_stream<T: DeserializeOwned>(
    mut stream: UnixStream,
    request: &Request,
    timeout: Duration,
) -> Result<T, ClientError> {
    stream.set_read_timeout(Some(timeout)).map_err(protocol)?;
    stream.set_write_timeout(Some(timeout)).map_err(protocol)?;
    serde_json::to_writer(&mut stream, request).map_err(protocol)?;
    stream.write_all(b"\n").map_err(protocol)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(protocol)?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(protocol)?;
    if line.is_empty() {
        return Err(ClientError::Protocol(
            "daemon disconnected before a response".to_owned(),
        ));
    }
    decode_response(&line)
}

pub fn send_event(config: &crate::config::Config, request: HookRequest) -> Result<(), ClientError> {
    one_shot::<Acknowledgement>(
        config,
        &Request::Event {
            request: Box::new(request),
        },
    )
    .map(|_| ())
}

pub fn clear(config: &crate::config::Config) -> Result<(), ClientError> {
    one_shot::<Acknowledgement>(config, &Request::Clear).map(|_| ())
}

pub fn stop(config: &crate::config::Config) -> Result<(), ClientError> {
    match one_shot::<Acknowledgement>(config, &Request::Shutdown) {
        Ok(_) | Err(ClientError::Unavailable(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn health(
    config: &crate::config::Config,
) -> Result<(u64, Topology, Vec<SessionView>), ClientError> {
    let response = one_shot::<HealthResponse>(config, &Request::Health)?;
    Ok((response.revision, response.topology, response.views))
}

pub fn cached_views(config: &crate::config::Config) -> Result<Vec<SessionView>, ClientError> {
    health(config).map(|(_, _, views)| views)
}

pub fn subscribe(
    config: &crate::config::Config,
) -> Result<mpsc::Receiver<Result<SubscriptionUpdate, ClientError>>, ClientError> {
    let stream = UnixStream::connect(socket_path(config)).map_err(unavailable)?;
    subscription_stream(stream)
}

pub fn subscribe_with_retry(
    config: &crate::config::Config,
    attempts: usize,
    delay: Duration,
) -> Result<mpsc::Receiver<Result<SubscriptionUpdate, ClientError>>, ClientError> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match subscribe(config) {
            Ok(receiver) => return Ok(receiver),
            Err(ClientError::Unavailable(_)) if attempt + 1 < attempts => thread::sleep(delay),
            Err(error) => return Err(error),
        }
    }
    unreachable!("at least one subscription attempt is always made")
}

fn subscription_stream(
    mut stream: UnixStream,
) -> Result<mpsc::Receiver<Result<SubscriptionUpdate, ClientError>>, ClientError> {
    serde_json::to_writer(&mut stream, &Request::Subscribe).map_err(protocol)?;
    stream.write_all(b"\n").map_err(protocol)?;
    stream.flush().map_err(protocol)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut last_revision = None;
        loop {
            let mut line = String::new();
            let result = match reader.read_line(&mut line) {
                Ok(0) => Err(ClientError::Protocol(
                    "daemon subscription disconnected".to_owned(),
                )),
                Err(error) => Err(protocol(error)),
                Ok(_) => decode_response::<SubscriptionUpdate>(&line).and_then(|update| {
                    if last_revision.is_some_and(|revision| update.revision <= revision) {
                        Err(ClientError::Protocol(format!(
                            "subscription revision {} did not advance past {}",
                            update.revision,
                            last_revision.unwrap_or_default()
                        )))
                    } else {
                        last_revision = Some(update.revision);
                        Ok(update)
                    }
                }),
            };
            let closed = result.is_err();
            if sender.send(result).is_err() || closed {
                return;
            }
        }
    });
    Ok(receiver)
}

#[derive(Debug)]
pub(crate) enum ServerReadError {
    Oversized,
    Invalid(String),
    Io(String),
}

pub(crate) fn read_request(reader: &mut impl BufRead) -> Result<Request, ServerReadError> {
    let mut line = String::new();
    let bytes = reader
        .by_ref()
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line)
        .map_err(|error| ServerReadError::Io(error.to_string()))?;
    if bytes as u64 == MAX_REQUEST_BYTES && !line.ends_with('\n') {
        return Err(ServerReadError::Oversized);
    }
    serde_json::from_str(&line).map_err(|error| ServerReadError::Invalid(error.to_string()))
}

fn write_response(stream: &mut UnixStream, response: &impl Serialize) -> Result<(), String> {
    let mut line = serde_json::to_vec(response).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())
}

pub(crate) fn write_acknowledgement(stream: &mut UnixStream, revision: u64) -> Result<(), String> {
    write_response(stream, &Acknowledgement { revision })
}

pub(crate) fn write_health(
    stream: &mut UnixStream,
    revision: u64,
    topology: Topology,
    views: Vec<SessionView>,
) -> Result<(), String> {
    write_response(
        stream,
        &HealthResponse {
            revision,
            topology,
            views,
        },
    )
}

pub(crate) fn write_subscription(
    stream: &mut UnixStream,
    revision: u64,
    views: Vec<SessionView>,
) -> Result<(), String> {
    write_response(stream, &SubscriptionUpdate { revision, views })
}

pub(crate) fn write_rejection(
    stream: &mut UnixStream,
    message: impl Into<String>,
) -> Result<(), String> {
    write_response(
        stream,
        &Rejection {
            revision: 0,
            error: message.into(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::model::SessionView;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn config() -> Config {
        let state_dir = std::env::temp_dir().join(format!(
            "amux-ipc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&state_dir).unwrap();
        Config {
            state_dir,
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        }
    }

    fn acknowledgement_pair(
        serve: impl FnOnce(UnixStream) + Send + 'static,
        timeout: Duration,
    ) -> Result<Acknowledgement, ClientError> {
        let (client, server) = UnixStream::pair().unwrap();
        thread::spawn(move || serve(server));
        one_shot_stream(client, &Request::Ping, timeout)
    }

    fn health_pair(
        serve: impl FnOnce(UnixStream) + Send + 'static,
        timeout: Duration,
    ) -> Result<HealthResponse, ClientError> {
        let (client, server) = UnixStream::pair().unwrap();
        thread::spawn(move || serve(server));
        one_shot_stream(client, &Request::Health, timeout)
    }

    fn read_request_line(stream: &UnixStream) {
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn one_shot_reads_a_complete_large_response() {
        let views = (0..4_000)
            .map(|index| SessionView {
                session: format!("session-{index}"),
                last_attached: index,
                attached: true,
                status: "running".to_owned(),
                attention: false,
                agent_count: 1,
                live_agent_count: 1,
                agents: Vec::new(),
                pane: format!("%{index}"),
                reason: "large health response".to_owned(),
                cwd: "/tmp/a-response-larger-than-the-socket-buffer".to_owned(),
                updated_at: index,
            })
            .collect::<Vec<_>>();
        let response = health_pair(
            move |mut server| {
                read_request_line(&server);
                write_health(&mut server, 9, Topology::default(), views).unwrap();
            },
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(response.revision, 9);
        assert_eq!(response.views.len(), 4_000);
    }

    #[test]
    fn one_shot_distinguishes_disconnect_malformed_rejection_and_timeout() {
        let disconnected =
            acknowledgement_pair(|server| read_request_line(&server), Duration::from_secs(1))
                .unwrap_err();
        assert!(
            matches!(disconnected, ClientError::Protocol(message) if message.contains("disconnected"))
        );

        let malformed = acknowledgement_pair(
            |mut server| {
                read_request_line(&server);
                server.write_all(b"not json\n").unwrap();
            },
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(matches!(malformed, ClientError::Protocol(_)));

        let rejected = acknowledgement_pair(
            |mut server| {
                read_request_line(&server);
                write_rejection(&mut server, "invalid event").unwrap();
            },
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(rejected, ClientError::Rejected("invalid event".to_owned()));

        let timed_out = acknowledgement_pair(
            |server| {
                read_request_line(&server);
                thread::sleep(Duration::from_millis(100));
            },
            Duration::from_millis(10),
        )
        .unwrap_err();
        assert!(matches!(timed_out, ClientError::Protocol(_)));
    }

    #[test]
    fn each_request_accepts_only_its_success_shape() {
        let acknowledgement = serde_json::to_string(&Acknowledgement { revision: 7 }).unwrap();
        let health = serde_json::to_string(&HealthResponse {
            revision: 7,
            topology: Topology::default(),
            views: Vec::new(),
        })
        .unwrap();
        let subscription = serde_json::to_string(&SubscriptionUpdate {
            revision: 7,
            views: Vec::new(),
        })
        .unwrap();

        assert!(decode_response::<Acknowledgement>(&acknowledgement).is_ok());
        assert!(decode_response::<Acknowledgement>(&health).is_err());
        assert!(decode_response::<HealthResponse>(&health).is_ok());
        assert!(decode_response::<HealthResponse>(&acknowledgement).is_err());
        assert!(decode_response::<SubscriptionUpdate>(&subscription).is_ok());
        assert!(decode_response::<SubscriptionUpdate>(&health).is_err());
    }

    #[test]
    fn server_reader_rejects_oversized_and_malformed_requests() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let writer = thread::spawn(move || {
            client
                .write_all(&vec![b'x'; MAX_REQUEST_BYTES as usize])
                .unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
        });
        assert!(matches!(
            read_request(&mut BufReader::new(server)),
            Err(ServerReadError::Oversized)
        ));
        writer.join().unwrap();

        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(b"not json\n").unwrap();
        assert!(matches!(
            read_request(&mut BufReader::new(server)),
            Err(ServerReadError::Invalid(_))
        ));
    }

    #[test]
    fn subscription_exposes_revision_and_views_without_one_shot_timeouts() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let updates = subscription_stream(client).unwrap();
        read_request_line(&server);
        write_subscription(&mut server, 3, Vec::new()).unwrap();
        write_subscription(&mut server, 4, Vec::new()).unwrap();
        assert_eq!(updates.recv().unwrap().unwrap().revision, 3);
        assert_eq!(updates.recv().unwrap().unwrap().revision, 4);
        drop(server);
        assert!(matches!(
            updates.recv().unwrap(),
            Err(ClientError::Protocol(message)) if message.contains("disconnected")
        ));
    }

    #[test]
    fn subscription_preserves_large_updates_for_a_briefly_slow_consumer() {
        let (client, mut server) = UnixStream::pair().unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let updates = subscription_stream(client).unwrap();
        read_request_line(&server);
        let views = (0..400)
            .map(|index| SessionView {
                session: format!("session-{index}"),
                last_attached: index,
                attached: true,
                status: "running".to_owned(),
                attention: false,
                agent_count: 1,
                live_agent_count: 1,
                agents: Vec::new(),
                pane: format!("%{index}"),
                reason: "large subscription snapshot".repeat(4),
                cwd: "/tmp/large-subscription-snapshot".to_owned(),
                updated_at: index,
            })
            .collect::<Vec<_>>();
        let writer = thread::spawn(move || {
            for revision in 1..=3 {
                write_subscription(&mut server, revision, views.clone()).unwrap();
            }
        });
        thread::sleep(Duration::from_millis(100));
        for revision in 1..=3 {
            let update = updates.recv().unwrap().unwrap();
            assert_eq!(update.revision, revision);
            assert_eq!(update.views.len(), 400);
        }
        writer.join().unwrap();
    }

    #[test]
    fn subscription_rejects_non_monotonic_revisions() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let updates = subscription_stream(client).unwrap();
        read_request_line(&server);
        write_subscription(&mut server, 3, Vec::new()).unwrap();
        write_subscription(&mut server, 3, Vec::new()).unwrap();
        assert_eq!(updates.recv().unwrap().unwrap().revision, 3);
        assert!(matches!(
            updates.recv().unwrap(),
            Err(ClientError::Protocol(message)) if message.contains("did not advance")
        ));
    }

    #[test]
    fn absent_shutdown_succeeds_but_connected_rejection_is_validated() {
        let config = config();
        assert!(stop(&config).is_ok());

        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request_line(&stream);
            write_rejection(&mut stream, "shutdown denied").unwrap();
        });
        assert_eq!(
            stop(&config).unwrap_err(),
            ClientError::Rejected("shutdown denied".to_owned())
        );
        server.join().unwrap();
        fs::remove_dir_all(config.state_dir).unwrap();
    }
}
