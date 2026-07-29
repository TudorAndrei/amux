use crate::config::Config;
use crate::event;
use crate::ipc::{HookRequest, Request, Response};
use crate::model::{SessionView, State};
use crate::state;
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How often an idle subscription wakes to look for a new revision.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How long a broadcast may block before the subscriber is treated as dead.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
/// How often the daemon checks whether its own executable was replaced.
const BINARY_CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Size and mtime are enough to notice an upgrade: installs replace the file
/// rather than editing it in place.
fn binary_fingerprint(path: &std::path::Path) -> Option<(u64, std::time::SystemTime)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// A socket timeout surfaces as `WouldBlock` on Linux and `TimedOut` on macOS;
/// `Interrupted` means the syscall was cut short by a signal. None of them say
/// anything about the peer, so all three mean "nothing to read yet".
fn would_block(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}

pub fn socket_path(config: &Config) -> PathBuf {
    config.state_dir.join("amux.sock")
}

#[derive(Clone)]
struct Shared {
    revision: u64,
    state: State,
    topology: crate::tmux::Topology,
    views: Vec<SessionView>,
    shutdown: bool,
    monitor_server: Option<PathBuf>,
    monitor_stop: Option<Arc<AtomicBool>>,
    maintenance_active: Arc<AtomicBool>,
}

pub fn run(config: Config) -> Result<(), String> {
    state::ensure_private_dir(&config)?;
    state::adopt_orphaned_log(&config)?;
    let mut state = state::load(&config)?;
    state::compact_events(&config, &state.records.keys().cloned().collect())?;
    state = state::load(&config)?;
    let path = socket_path(&config);
    if path.exists() {
        match UnixStream::connect(&path) {
            Ok(_) => return Err(format!("daemon already running ({})", path.display())),
            Err(_) => {
                let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                if !metadata.file_type().is_socket() {
                    return Err(format!(
                        "refusing to replace non-socket path {}",
                        path.display()
                    ));
                }
                fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
        }
    }
    let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let topology = crate::tmux::Topology::default();
    let views = crate::sessions::views_with_topology(&config, &state, &topology);
    let shared = Arc::new(Mutex::new(Shared {
        revision: 0,
        state,
        topology,
        views,
        shutdown: false,
        monitor_server: None,
        monitor_stop: None,
        maintenance_active: Arc::new(AtomicBool::new(false)),
    }));
    attach_monitor(&config, &shared, crate::tmux::server_from_env());
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    // An upgrade replaces the binary while this process keeps serving the old
    // code, which then disagrees with the new hooks about on-disk formats. Retire
    // ourselves when the executable changes so the next hook starts the new one.
    let executable = std::env::current_exe().ok();
    let installed = executable.as_deref().and_then(binary_fingerprint);
    let mut last_binary_check = Instant::now();
    loop {
        if shared
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .shutdown
        {
            break;
        }
        if last_binary_check.elapsed() >= BINARY_CHECK_INTERVAL {
            last_binary_check = Instant::now();
            if let (Some(path), Some(expected)) = (executable.as_deref(), installed)
                // A failed stat is transient (or a mid-install rename); only a
                // successful, different fingerprint means we are stale.
                && binary_fingerprint(path).is_some_and(|current| current != expected)
            {
                eprintln!(
                    "amux daemon: executable changed on disk; exiting so the next event starts the current build"
                );
                break;
            }
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // BSD and macOS inherit the listener's O_NONBLOCK on the
                // accepted socket; Linux does not. Left inherited, a response
                // larger than the socket send buffer (8 KiB by default on
                // macOS) fails mid-`write_all` with WouldBlock and the client
                // sees a truncated line.
                if let Err(error) = stream.set_nonblocking(false) {
                    eprintln!("amux daemon: cannot restore blocking mode: {error}");
                    continue;
                }
                let config = config.clone();
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &config, shared) {
                        eprintln!("amux daemon: client connection failed: {error}");
                    }
                });
            }
            Err(error) if would_block(&error) => thread::sleep(Duration::from_millis(10)),
            // A connection aborted between the client's connect and this
            // accept says nothing about the listener, so keep serving.
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    if let Ok(guard) = shared.lock()
        && let Some(stop) = &guard.monitor_stop
    {
        stop.store(true, Ordering::Relaxed);
    }
    let _ = fs::remove_file(path);
    Ok(())
}

/// Write one response as a single framed line. Serializing into memory first
/// keeps a failed write from leaving a half-decodable record on the socket.
fn reply(stream: &mut UnixStream, response: &Response) -> Result<(), String> {
    let mut line = serde_json::to_vec(response).map_err(|error| error.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())
}

fn handle(
    mut stream: UnixStream,
    config: &Config,
    shared: Arc<Mutex<Shared>>,
) -> Result<(), String> {
    let reader_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    let bytes = reader
        .by_ref()
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if bytes as u64 == MAX_REQUEST_BYTES && !line.ends_with('\n') {
        reply(
            &mut stream,
            &Response::error("daemon request exceeds 1 MiB"),
        )?;
        return Ok(());
    }
    let request: Request = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(error) => {
            reply(
                &mut stream,
                &Response::error(format!("invalid daemon request: {error}")),
            )?;
            return Ok(());
        }
    };
    match request {
        Request::Ping => reply(
            &mut stream,
            &Response::ok(
                shared
                    .lock()
                    .map_err(|_| "daemon state lock poisoned".to_owned())?
                    .revision,
            ),
        ),
        Request::Shutdown => {
            shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .shutdown = true;
            reply(&mut stream, &Response::ok(0))
        }
        Request::Health => {
            let guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            reply(
                &mut stream,
                &Response::health(guard.revision, guard.topology.clone(), guard.views.clone()),
            )
        }
        Request::Event { request } => {
            let request = *request;
            attach_monitor(config, &shared, request.tmux_server.clone());
            let tmux = context_for_known_pane(&shared, &request.tmux_pane)
                .or_else(|| context_from_request(&request))
                .unwrap_or_default();
            let (key, record) = event::normalize_at(event::NormalizeInput {
                agent: &request.agent,
                event_override: &request.event,
                status_override: &request.status,
                attention_override: &request.attention,
                reason_override: &request.reason,
                raw: request.raw,
                fallback_cwd: request.cwd,
                tmux,
            });
            let mut fields = serde_json::to_value(&record)
                .map_err(|error| error.to_string())?
                .as_object()
                .cloned()
                .unwrap_or_default();
            fields.insert("key".to_owned(), serde_json::Value::String(key.clone()));
            let write = state::write_event(
                config,
                key,
                record,
                &serde_json::Value::Object(fields),
                event::now(),
            )?;
            let mut guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            guard.state = state::load(config)?;
            guard.views =
                crate::sessions::views_with_topology(config, &guard.state, &guard.topology);
            guard.revision += 1;
            let revision = guard.revision;
            let retain_keys = guard.state.records.keys().cloned().collect::<BTreeSet<_>>();
            drop(guard);
            if write.over_compact_threshold {
                schedule_compaction(config.clone(), Arc::clone(&shared), retain_keys);
            }
            reply(&mut stream, &Response::ok(revision))
        }
        Request::Clear => {
            state::clear(config)?;
            let mut guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            guard.state = State::initial();
            guard.views =
                crate::sessions::views_with_topology(config, &guard.state, &guard.topology);
            guard.revision += 1;
            reply(&mut stream, &Response::ok(guard.revision))
        }
        Request::Subscribe => {
            let (mut revision, initial, topology, views) = {
                let guard = shared
                    .lock()
                    .map_err(|_| "daemon state lock poisoned".to_owned())?;
                (
                    guard.revision,
                    guard.state.clone(),
                    guard.topology.clone(),
                    guard.views.clone(),
                )
            };
            reply(
                &mut stream,
                &Response::state(revision, initial, topology, views),
            )?;
            stream
                .set_write_timeout(Some(WRITE_TIMEOUT))
                .map_err(|error| error.to_string())?;
            // The read half is a `try_clone()` of the write half, so both share
            // one file description: `set_nonblocking()` here would also make
            // broadcasts non-blocking and truncate them with `EWOULDBLOCK`. A
            // read timeout is a socket option that only applies to reads, so it
            // paces this loop without touching the write half.
            reader
                .get_mut()
                .set_read_timeout(Some(POLL_INTERVAL))
                .map_err(|error| error.to_string())?;
            loop {
                let mut probe = [0_u8; 1];
                match reader.read(&mut probe) {
                    Ok(0) => return Ok(()),
                    Ok(_) => return Err("unexpected data from daemon subscriber".to_owned()),
                    Err(error) if would_block(&error) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::ConnectionAborted
                                | std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::NotConnected
                        ) =>
                    {
                        return Ok(());
                    }
                    Err(error) => return Err(error.to_string()),
                }
                let response = {
                    let guard = shared
                        .lock()
                        .map_err(|_| "daemon state lock poisoned".to_owned())?;
                    if guard.shutdown {
                        return Ok(());
                    }
                    if guard.revision == revision {
                        None
                    } else {
                        revision = guard.revision;
                        Some(Response::state(
                            revision,
                            guard.state.clone(),
                            guard.topology.clone(),
                            guard.views.clone(),
                        ))
                    }
                };
                if let Some(response) = response {
                    reply(&mut stream, &response)?;
                }
            }
        }
    }
}

/// A just-started monitor may not have published its initial snapshot when the
/// hook that attached it arrives. Wait briefly for that in-memory snapshot; this
/// is still daemon-local and never reintroduces per-hook tmux subprocesses.
fn context_for_known_pane(
    shared: &Arc<Mutex<Shared>>,
    pane_id: &str,
) -> Option<crate::event::TmuxContext> {
    if pane_id.is_empty() {
        return None;
    }
    for attempt in 0..10 {
        let topology = shared.lock().ok()?.topology.clone();
        if let Some(context) = context_for_pane(&topology, pane_id) {
            return Some(context);
        }
        if attempt != 9 {
            thread::sleep(Duration::from_millis(10));
        }
    }
    None
}

fn schedule_compaction(config: Config, shared: Arc<Mutex<Shared>>, retain_keys: BTreeSet<String>) {
    let active = match shared.lock() {
        Ok(guard) => Arc::clone(&guard.maintenance_active),
        Err(_) => return,
    };
    if active.swap(true, Ordering::AcqRel) {
        return;
    }
    thread::spawn(move || {
        let _ = state::compact_events(&config, &retain_keys);
        active.store(false, Ordering::Release);
    });
}

fn attach_monitor(config: &Config, shared: &Arc<Mutex<Shared>>, server: Option<PathBuf>) {
    let Some(server) = server else {
        return;
    };
    let stop = {
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        if guard.monitor_server.as_ref() == Some(&server) {
            return;
        }
        if let Some(previous) = guard.monitor_stop.replace(Arc::new(AtomicBool::new(false))) {
            previous.store(true, Ordering::Relaxed);
        }
        guard.monitor_server = Some(server.clone());
        guard
            .monitor_stop
            .as_ref()
            .expect("monitor stop signal")
            .clone()
    };
    let shared_for_monitor = Arc::clone(shared);
    let config_for_monitor = config.clone();
    crate::tmux::spawn(stop, Some(server.clone()), move |topology| {
        if let Ok(mut guard) = shared_for_monitor.lock()
            && guard.monitor_server.as_ref() == Some(&server)
            && guard.topology != topology
        {
            guard.topology = topology;
            guard.views = crate::sessions::views_with_topology(
                &config_for_monitor,
                &guard.state,
                &guard.topology,
            );
            guard.revision += 1;
        }
    });
}

pub fn send_event(config: &Config, request: HookRequest) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path(config)).map_err(|error| error.to_string())?;
    serde_json::to_writer(
        &mut stream,
        &Request::Event {
            request: Box::new(request),
        },
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    let response: Response = serde_json::from_str(&response).map_err(|error| error.to_string())?;
    response.error.map_or(Ok(()), Err)
}

/// Ask a running daemon to exit. A daemon that is not running is the desired
/// end state, so a failed connect succeeds. `Request::Shutdown` predates the
/// versions this needs to retire, so an older daemon understands it too.
pub fn stop(config: &Config) -> Result<(), String> {
    let Ok(mut stream) = UnixStream::connect(socket_path(config)) else {
        return Ok(());
    };
    serde_json::to_writer(&mut stream, &Request::Shutdown).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    let _ = BufReader::new(stream).read_line(&mut response);
    Ok(())
}

pub fn clear(config: &Config) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path(config)).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &Request::Clear).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    let response: Response = serde_json::from_str(&response).map_err(|error| error.to_string())?;
    response.error.map_or(Ok(()), Err)
}

pub fn subscribe(config: &Config) -> Result<mpsc::Receiver<Vec<SessionView>>, String> {
    let mut stream = UnixStream::connect(socket_path(config)).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &Request::Subscribe).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => match serde_json::from_str::<Response>(&line) {
                    Ok(Response {
                        views: Some(views), ..
                    }) => {
                        if sender.send(views).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                },
            }
        }
    });
    Ok(receiver)
}

pub fn health(config: &Config) -> Result<(u64, crate::tmux::Topology, Vec<SessionView>), String> {
    let mut stream = UnixStream::connect(socket_path(config)).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &Request::Health).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let response: Response = serde_json::from_str(&line).map_err(|error| error.to_string())?;
    let topology = response
        .topology
        .ok_or_else(|| "daemon did not return monitor health".to_owned())?;
    Ok((
        response.revision,
        topology,
        response.views.unwrap_or_default(),
    ))
}

pub fn cached_views(config: &Config) -> Result<Vec<SessionView>, String> {
    health(config).map(|(_, _, views)| views)
}

fn context_for_pane(
    topology: &crate::tmux::Topology,
    pane_id: &str,
) -> Option<crate::event::TmuxContext> {
    topology.panes.iter().find_map(|pane| {
        (pane.pane == pane_id).then(|| crate::event::TmuxContext {
            session: pane.session.clone(),
            window: pane.window.clone(),
            pane: pane.pane.clone(),
        })
    })
}

fn context_from_request(request: &HookRequest) -> Option<crate::event::TmuxContext> {
    (!request.tmux_pane.is_empty()).then(|| crate::event::TmuxContext {
        pane: request.tmux_pane.clone(),
        session: request.tmux_session.clone(),
        window: request.tmux_window.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// The upgrade check is only as good as this fingerprint: an install that
    /// replaces the file must read as different, and an untouched file must not.
    #[test]
    fn binary_fingerprint_tracks_replacement() {
        let dir = std::env::temp_dir().join(format!("amux-fingerprint-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("amux-rs");
        fs::write(&path, b"first build").unwrap();
        let original = binary_fingerprint(&path).expect("fingerprint of an existing file");
        assert_eq!(binary_fingerprint(&path), Some(original));

        // Installs replace rather than edit, so compare against a renamed file.
        let replacement = dir.join("amux-rs.new");
        fs::write(&replacement, b"second build, a different length").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert_ne!(binary_fingerprint(&path), Some(original));

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(binary_fingerprint(&path), None, "a missing file has none");
    }

    fn shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared {
            revision: 0,
            state: State::initial(),
            topology: crate::tmux::Topology::default(),
            views: Vec::new(),
            shutdown: false,
            monitor_server: None,
            monitor_stop: None,
            maintenance_active: Arc::new(AtomicBool::new(false)),
        }))
    }

    fn subscriber_config() -> Config {
        Config {
            state_dir: PathBuf::new(),
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

    /// A broadcast larger than the socket buffer must arrive as one intact
    /// line. The read half is a `try_clone()` of the write half, so marking it
    /// non-blocking used to make the write half non-blocking too and cut the
    /// record short with `EWOULDBLOCK` (`os error 11` on Linux).
    #[test]
    fn a_large_broadcast_reaches_the_subscriber_intact() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = shared();
        let published = Arc::clone(&shared);
        thread::spawn(move || {
            let _ = handle(server, &subscriber_config(), shared);
        });
        client.write_all(br#"{"kind":"subscribe"}"#).unwrap();
        client.write_all(b"\n").unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut initial = String::new();
        reader.read_line(&mut initial).unwrap();
        assert_eq!(
            serde_json::from_str::<Response>(&initial).unwrap().revision,
            0
        );
        {
            let mut guard = published.lock().unwrap();
            for index in 0..2_000 {
                guard.state.records.insert(
                    format!("codex:session-{index}"),
                    crate::model::Record {
                        agent: "codex".to_owned(),
                        cwd: "/tmp/amux-subscriber-broadcast".to_owned(),
                        ..crate::model::Record::default()
                    },
                );
            }
            guard.revision = 1;
        }
        let mut update = String::new();
        reader.read_line(&mut update).unwrap();
        assert!(
            update.len() > 256 * 1024,
            "the broadcast should exceed a socket buffer: {} bytes",
            update.len()
        );
        let update: Response = serde_json::from_str(&update).expect("a complete broadcast");
        assert_eq!(update.revision, 1);
        assert_eq!(update.state.unwrap().records.len(), 2_000);
    }

    #[test]
    fn oversized_request_is_rejected_without_poisoning_shared_state() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = shared();
        let worker_shared = Arc::clone(&shared);
        let config = subscriber_config();
        let worker = thread::spawn(move || handle(server, &config, worker_shared));
        client
            .write_all(&vec![b'x'; MAX_REQUEST_BYTES as usize])
            .unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        assert!(
            serde_json::from_str::<Response>(&response)
                .unwrap()
                .error
                .is_some_and(|error| error.contains("exceeds 1 MiB"))
        );
        assert!(worker.join().unwrap().is_ok());

        let (mut client, server) = UnixStream::pair().unwrap();
        let config = subscriber_config();
        let worker = thread::spawn(move || handle(server, &config, shared));
        client.write_all(b"{\"kind\":\"ping\"}\n").unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<Response>(&response)
                .unwrap()
                .revision,
            0
        );
        assert!(worker.join().unwrap().is_ok());
    }

    #[test]
    fn subscription_exits_when_the_client_disconnects() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let config = Config {
            state_dir: PathBuf::new(),
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            sender.send(handle(server, &config, shared())).unwrap();
        });
        client.write_all(br#"{"kind":"subscribe"}"#).unwrap();
        client.write_all(b"\n").unwrap();
        {
            let mut initial = String::new();
            BufReader::new(client.try_clone().unwrap())
                .read_line(&mut initial)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Response>(&initial).unwrap().revision,
                0
            );
        }
        drop(client);
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("subscription handler should exit promptly")
                .is_ok()
        );
    }
}
