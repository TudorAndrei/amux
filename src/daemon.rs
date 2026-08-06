mod live_model;
mod maintenance;

use crate::config::Config;
use crate::intake;
use crate::ipc::{self, HookRequest, Request};
use crate::state;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How often an idle subscription wakes to look for a new revision.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How long a broadcast may block before the subscriber is treated as dead.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the daemon checks whether its own executable was replaced.
const BINARY_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long an event waits for the monitor publication that can identify its pane.
const MONITOR_CONTEXT_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a client can hold a connection before it sends its first request.
const INITIAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// Maximum number of daemon client handlers, including subscriptions.
const MAX_ACTIVE_CLIENTS: usize = 64;

type MonitorUpdates = Arc<(Mutex<u64>, Condvar)>;

struct ClientLimiter {
    active: AtomicUsize,
    limit: usize,
}

impl ClientLimiter {
    fn new(limit: usize) -> Self {
        Self {
            active: AtomicUsize::new(0),
            limit,
        }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<ClientPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.limit).then_some(active + 1)
            })
            .ok()
            .map(|_| ClientPermit {
                limiter: Arc::clone(self),
            })
    }
}

struct ClientPermit {
    limiter: Arc<ClientLimiter>,
}

impl Drop for ClientPermit {
    fn drop(&mut self) {
        self.limiter.active.fetch_sub(1, Ordering::AcqRel);
    }
}

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

/// Owns the fresh file description carrying the daemon startup claim. The
/// kernel releases the claim when this value is dropped; the stable claim file
/// itself is deliberately never removed.
struct StartupClaim {
    _file: File,
}

fn acquire_startup_claim(config: &Config) -> Result<StartupClaim, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(config.daemon_startup_claim_file())
        .map_err(|error| error.to_string())?;
    file.lock().map_err(|error| error.to_string())?;
    Ok(StartupClaim { _file: file })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

fn socket_identity(path: &std::path::Path) -> Result<SocketIdentity, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    Ok(SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn remove_owned_socket(path: &std::path::Path, owned: SocketIdentity) {
    if socket_identity(path).ok() == Some(owned) {
        let _ = fs::remove_file(path);
    }
}

struct Shared {
    model: live_model::LiveModel,
    mutations: Arc<Mutex<()>>,
    shutdown: bool,
    monitor_server: Option<PathBuf>,
    monitor_stop: Option<Arc<AtomicBool>>,
    monitor_updates: MonitorUpdates,
    maintenance: maintenance::Maintenance,
}

pub fn run(config: Config) -> Result<(), String> {
    state::ensure_private_dir(&config)?;
    let startup_claim = acquire_startup_claim(&config)?;
    let path = ipc::socket_path(&config);
    // This check must happen after acquiring the startup claim: a waiting
    // starter may have observed a stale path before the winner completed bind.
    if UnixStream::connect(&path).is_ok() {
        return Err(format!("daemon already running ({})", path.display()));
    }
    if path.exists() {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "refusing to replace non-socket path {}",
                path.display()
            ));
        }
        fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    let state = state::recover_startup(&config)?;
    let listener = UnixListener::bind(&path).map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let owned_socket = socket_identity(&path)?;
    drop(startup_claim);
    let topology = crate::tmux::Topology::default();
    let shared = Arc::new(Mutex::new(Shared {
        model: live_model::LiveModel::new(&config, state, topology),
        mutations: Arc::new(Mutex::new(())),
        shutdown: false,
        monitor_server: None,
        monitor_stop: None,
        monitor_updates: Arc::new((Mutex::new(0), Condvar::new())),
        maintenance: maintenance::Maintenance::default(),
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
    let clients = Arc::new(ClientLimiter::new(MAX_ACTIVE_CLIENTS));
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
                let Some(permit) = clients.try_acquire() else {
                    reject_saturated_client(stream);
                    continue;
                };
                let config = config.clone();
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _permit = permit;
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
    if let Ok(guard) = shared.lock() {
        let _ = guard.maintenance.wait_idle();
    }
    remove_owned_socket(&path, owned_socket);
    Ok(())
}

fn reject_saturated_client(mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(INITIAL_REQUEST_TIMEOUT));
    let _ = ipc::write_rejection(&mut stream, "daemon client limit reached");
}

fn handle(stream: UnixStream, config: &Config, shared: Arc<Mutex<Shared>>) -> Result<(), String> {
    handle_with_timeout(stream, config, shared, INITIAL_REQUEST_TIMEOUT)
}

fn handle_with_timeout(
    mut stream: UnixStream,
    config: &Config,
    shared: Arc<Mutex<Shared>>,
    initial_timeout: Duration,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(initial_timeout))
        .map_err(|error| error.to_string())?;
    let reader_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(reader_stream);
    let request = match ipc::read_request(&mut reader) {
        Ok(request) => request,
        Err(ipc::ServerReadError::Oversized) => {
            ipc::write_rejection(&mut stream, "daemon request exceeds 1 MiB")?;
            return Ok(());
        }
        Err(ipc::ServerReadError::Invalid(error)) => {
            ipc::write_rejection(&mut stream, format!("invalid daemon request: {error}"))?;
            return Ok(());
        }
        Err(ipc::ServerReadError::Io(error)) => return Err(error),
    };
    match request {
        Request::Ping => ipc::write_acknowledgement(
            &mut stream,
            shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .model
                .revision(),
        ),
        Request::Shutdown => {
            // Publish shutdown only after the reply is in the socket buffer.
            // Otherwise the main thread can exit the process while this
            // handler is still writing, and the client sees ConnectionReset.
            ipc::write_acknowledgement(&mut stream, 0)?;
            shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .shutdown = true;
            Ok(())
        }
        Request::Health => {
            let snapshot = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .model
                .health_snapshot();
            ipc::write_health(
                &mut stream,
                snapshot.revision,
                snapshot.topology,
                snapshot.views,
            )
        }
        Request::Event { request } => {
            let request = *request;
            attach_monitor(config, &shared, request.tmux_server.clone());
            let tmux = context_for_known_pane(&shared, &request.tmux_pane)
                .or_else(|| context_from_request(&request))
                .unwrap_or_default();
            let revision = mutate_event_with_checkpoint(config, &shared, request, tmux, || {})?;
            ipc::write_acknowledgement(&mut stream, revision)
        }
        Request::Clear => {
            let revision = mutate_clear_with_checkpoint(config, &shared, || {})?;
            ipc::write_acknowledgement(&mut stream, revision)
        }
        Request::Subscribe => {
            let initial = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .model
                .subscription_update(None)
                .expect("a new subscriber always receives the current revision");
            let mut revision = initial.revision;
            ipc::write_subscription(&mut stream, revision, initial.views)?;
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
                    if let Some(update) = guard.model.subscription_update(Some(revision)) {
                        revision = update.revision;
                        Some(update)
                    } else {
                        None
                    }
                };
                if let Some(update) = response {
                    ipc::write_subscription(&mut stream, update.revision, update.views)?;
                }
            }
        }
    }
}

fn mutation_coordinator(shared: &Arc<Mutex<Shared>>) -> Result<Arc<Mutex<()>>, String> {
    shared
        .lock()
        .map_err(|_| "daemon state lock poisoned".to_owned())
        .map(|guard| Arc::clone(&guard.mutations))
}

fn mutate_event_with_checkpoint(
    config: &Config,
    shared: &Arc<Mutex<Shared>>,
    request: HookRequest,
    tmux: crate::event::TmuxContext,
    checkpoint: impl FnOnce(),
) -> Result<u64, String> {
    let mutations = mutation_coordinator(shared)?;
    let _mutation = mutations
        .lock()
        .map_err(|_| "daemon mutation lock poisoned".to_owned())?;
    let commit = intake::persist(config, request, tmux)?;
    checkpoint();
    let mut guard = shared
        .lock()
        .map_err(|_| "daemon state lock poisoned".to_owned())?;
    let revision = guard.model.apply_event_state(config, commit.state);
    let maintenance = guard.maintenance.clone();
    drop(guard);
    if commit.over_compact_threshold {
        maintenance.schedule(config.clone());
    }
    Ok(revision)
}

fn mutate_clear_with_checkpoint(
    config: &Config,
    shared: &Arc<Mutex<Shared>>,
    checkpoint: impl FnOnce(),
) -> Result<u64, String> {
    let mutations = mutation_coordinator(shared)?;
    let _mutation = mutations
        .lock()
        .map_err(|_| "daemon mutation lock poisoned".to_owned())?;
    let maintenance = shared
        .lock()
        .map_err(|_| "daemon state lock poisoned".to_owned())?
        .maintenance
        .clone();
    maintenance.clear(config)?;
    checkpoint();
    let mut guard = shared
        .lock()
        .map_err(|_| "daemon state lock poisoned".to_owned())?;
    Ok(guard.model.clear(config))
}

/// A just-started monitor can publish a transient error before its first useful
/// snapshot. Wait through monitor publications until the pane appears or the
/// bounded deadline expires. This stays daemon-local and does not add per-hook
/// tmux subprocesses.
fn context_for_known_pane(
    shared: &Arc<Mutex<Shared>>,
    pane_id: &str,
) -> Option<crate::event::TmuxContext> {
    if pane_id.is_empty() {
        return None;
    }
    let updates = shared.lock().ok()?.monitor_updates.clone();
    let deadline = Instant::now() + MONITOR_CONTEXT_TIMEOUT;
    let mut observed = *updates.0.lock().ok()?;
    let (revision, changed) = &*updates;
    loop {
        let topology = shared.lock().ok()?.model.topology().clone();
        if let Some(context) = context_for_pane(&topology, pane_id) {
            return Some(context);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let current = revision.lock().ok()?;
        if *current != observed {
            observed = *current;
            continue;
        }
        let (current, timeout) = changed
            .wait_timeout_while(current, remaining, |revision| *revision == observed)
            .ok()?;
        observed = *current;
        let timed_out = timeout.timed_out();
        drop(current);
        if timed_out {
            let topology = shared.lock().ok()?.model.topology().clone();
            return context_for_pane(&topology, pane_id);
        }
    }
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
        publish_monitor_topology(&config_for_monitor, &shared_for_monitor, &server, topology);
    });
}

fn publish_monitor_topology(
    config: &Config,
    shared: &Arc<Mutex<Shared>>,
    server: &std::path::Path,
    topology: crate::tmux::Topology,
) {
    let updates = {
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        if guard.monitor_server.as_deref() != Some(server) {
            return;
        }
        guard.model.apply_topology(config, topology);
        guard.monitor_updates.clone()
    };
    let (revision, changed) = &*updates;
    if let Ok(mut revision) = revision.lock() {
        *revision = revision
            .checked_add(1)
            .expect("monitor publication revision exhausted");
        changed.notify_all();
    }
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
    use std::sync::mpsc;

    static NEXT_MUTATION_TEST: AtomicUsize = AtomicUsize::new(0);

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
        let config = subscriber_config();
        shared_for(&config)
    }

    fn shared_for(config: &Config) -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared {
            model: live_model::LiveModel::new(
                config,
                crate::model::State::initial(),
                crate::tmux::Topology::default(),
            ),
            mutations: Arc::new(Mutex::new(())),
            shutdown: false,
            monitor_server: None,
            monitor_stop: None,
            monitor_updates: Arc::new((Mutex::new(0), Condvar::new())),
            maintenance: maintenance::Maintenance::default(),
        }))
    }

    fn mutation_config() -> Config {
        Config {
            state_dir: std::env::temp_dir().join(format!(
                "amux-daemon-mutation-{}-{}",
                std::process::id(),
                NEXT_MUTATION_TEST.fetch_add(1, Ordering::Relaxed)
            )),
            events_compact_bytes: u64::MAX,
            ..subscriber_config()
        }
    }

    fn mutation_request(session_id: &str) -> HookRequest {
        HookRequest {
            agent: "codex".to_owned(),
            event: "PostToolUse".to_owned(),
            status: String::new(),
            attention: String::new(),
            reason: String::new(),
            raw: serde_json::json!({"session_id": session_id}),
            cwd: "/workspace".to_owned(),
            tmux_pane: String::new(),
            tmux_session: String::new(),
            tmux_window: String::new(),
            tmux_server: None,
        }
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

    #[test]
    fn client_limit_never_exceeds_the_configured_bound() {
        let limiter = Arc::new(ClientLimiter::new(2));
        let first = limiter.try_acquire().unwrap();
        let second = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());
        assert_eq!(limiter.active.load(Ordering::Acquire), 2);

        drop(first);
        let third = limiter.try_acquire().unwrap();
        assert_eq!(limiter.active.load(Ordering::Acquire), 2);
        drop(second);
        drop(third);
        assert_eq!(limiter.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn saturated_client_receives_a_protocol_rejection() {
        let (client, server) = UnixStream::pair().unwrap();
        reject_saturated_client(server);
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["revision"], 0);
        assert_eq!(response["error"], "daemon client limit reached");
    }

    #[test]
    fn silent_client_releases_its_handler_after_the_initial_deadline() {
        let (_client, server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || {
            handle_with_timeout(
                server,
                &subscriber_config(),
                shared(),
                Duration::from_millis(20),
            )
        });

        let error = worker.join().unwrap().unwrap_err();
        assert!(
            error.contains("timed out") || error.contains("temporarily unavailable"),
            "{error}"
        );
    }

    #[test]
    fn event_publication_cannot_pass_an_earlier_durable_commit() {
        let config = mutation_config();
        let shared = shared_for(&config);
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let first_config = config.clone();
        let first_shared = Arc::clone(&shared);
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let first = thread::spawn(move || {
            mutate_event_with_checkpoint(
                &first_config,
                &first_shared,
                mutation_request("first"),
                crate::event::TmuxContext::default(),
                || {
                    first_entered.wait();
                    first_release.wait();
                },
            )
        });
        entered.wait();

        let (published, observed) = mpsc::channel();
        let second_config = config.clone();
        let second_shared = Arc::clone(&shared);
        let second = thread::spawn(move || {
            mutate_event_with_checkpoint(
                &second_config,
                &second_shared,
                mutation_request("second"),
                crate::event::TmuxContext::default(),
                || published.send(()).unwrap(),
            )
        });
        assert!(observed.recv_timeout(Duration::from_millis(50)).is_err());
        release.wait();

        assert_eq!(first.join().unwrap().unwrap(), 1);
        assert_eq!(second.join().unwrap().unwrap(), 2);
        assert_eq!(crate::state::load(&config).unwrap().records.len(), 2);
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn clear_and_event_publish_in_their_durable_order() {
        let config = mutation_config();
        let shared = shared_for(&config);
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let event_config = config.clone();
        let event_shared = Arc::clone(&shared);
        let event_entered = Arc::clone(&entered);
        let event_release = Arc::clone(&release);
        let event = thread::spawn(move || {
            mutate_event_with_checkpoint(
                &event_config,
                &event_shared,
                mutation_request("before-clear"),
                crate::event::TmuxContext::default(),
                || {
                    event_entered.wait();
                    event_release.wait();
                },
            )
        });
        entered.wait();

        let (cleared, observed) = mpsc::channel();
        let clear_config = config.clone();
        let clear_shared = Arc::clone(&shared);
        let clear = thread::spawn(move || {
            mutate_clear_with_checkpoint(&clear_config, &clear_shared, || {
                cleared.send(()).unwrap();
            })
        });
        assert!(observed.recv_timeout(Duration::from_millis(50)).is_err());
        release.wait();

        assert_eq!(event.join().unwrap().unwrap(), 1);
        assert_eq!(clear.join().unwrap().unwrap(), 2);
        assert!(!config.state_file().exists());
        assert!(!config.events_file().exists());
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn event_context_waits_through_monitor_errors_for_a_snapshot() {
        let shared = shared();
        let server = PathBuf::from("/tmp/amux-test-monitor.sock");
        shared.lock().unwrap().monitor_server = Some(server.clone());
        let publisher = Arc::clone(&shared);
        let config = subscriber_config();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            publish_monitor_topology(
                &config,
                &publisher,
                &server,
                crate::tmux::Topology {
                    error: "tmux is not ready".to_owned(),
                    ..crate::tmux::Topology::default()
                },
            );
            thread::sleep(Duration::from_millis(300));
            publish_monitor_topology(
                &config,
                &publisher,
                &server,
                crate::tmux::Topology {
                    panes: vec![crate::tmux::Pane {
                        session: "monitor".to_owned(),
                        window: "@1".to_owned(),
                        pane: "%0".to_owned(),
                        ..crate::tmux::Pane::default()
                    }],
                    connected: true,
                    ..crate::tmux::Topology::default()
                },
            );
        });

        let context = context_for_known_pane(&shared, "%0");

        worker.join().unwrap();
        assert_eq!(context.unwrap().session, "monitor");
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
            let _ = handle_with_timeout(
                server,
                &subscriber_config(),
                shared,
                Duration::from_millis(20),
            );
        });
        client.write_all(br#"{"kind":"subscribe"}"#).unwrap();
        client.write_all(b"\n").unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut initial = String::new();
        reader.read_line(&mut initial).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&initial).unwrap()["revision"],
            0
        );
        thread::sleep(Duration::from_millis(50));
        {
            let mut guard = published.lock().unwrap();
            let topology = crate::tmux::Topology {
                sessions: (0..2_000)
                    .map(|index| crate::tmux::TmuxSession {
                        id: format!("${index}"),
                        name: format!("session-{index}"),
                        last_attached: index,
                        attached: true,
                    })
                    .collect(),
                panes: (0..2_000)
                    .map(|index| crate::tmux::Pane {
                        session: format!("session-{index}"),
                        pane: format!("%{index}"),
                        command: "codex".to_owned(),
                        cwd: "/tmp/amux-subscriber-broadcast".to_owned(),
                        ..crate::tmux::Pane::default()
                    })
                    .collect(),
                connected: true,
                ..crate::tmux::Topology::default()
            };
            guard.model.apply_topology(&subscriber_config(), topology);
        }
        let mut update = String::new();
        reader.read_line(&mut update).unwrap();
        assert!(
            update.len() > 256 * 1024,
            "the broadcast should exceed a socket buffer: {} bytes",
            update.len()
        );
        let update: serde_json::Value =
            serde_json::from_str(&update).expect("a complete broadcast");
        assert_eq!(update["revision"], 1);
        assert_eq!(update["views"].as_array().unwrap().len(), 2_000);
        assert!(update.get("state").is_none());
        assert!(update.get("topology").is_none());
    }

    #[test]
    fn oversized_request_is_rejected_without_poisoning_shared_state() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = shared();
        let worker_shared = Arc::clone(&shared);
        let config = subscriber_config();
        let worker = thread::spawn(move || handle(server, &config, worker_shared));
        client
            .write_all(&vec![b'x'; ipc::MAX_REQUEST_BYTES as usize])
            .unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("exceeds 1 MiB")
        );
        assert!(worker.join().unwrap().is_ok());

        let (mut client, server) = UnixStream::pair().unwrap();
        let config = subscriber_config();
        let worker = thread::spawn(move || handle(server, &config, shared));
        client.write_all(b"{\"kind\":\"ping\"}\n").unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap()["revision"],
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
                serde_json::from_str::<serde_json::Value>(&initial).unwrap()["revision"],
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

    #[test]
    fn shutdown_is_not_published_before_its_acknowledgement() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(b"{\"kind\":\"shutdown\"}\n").unwrap();

        // Fill the response buffer so the acknowledgement cannot be sent. This
        // makes the ordering observable without depending on thread timing.
        server.set_nonblocking(true).unwrap();
        let block = [b'x'; 16 * 1024];
        loop {
            match server.write(&block) {
                Ok(_) => {}
                Err(error) if would_block(&error) => break,
                Err(error) => panic!("cannot fill daemon response buffer: {error}"),
            }
        }

        let shared = shared();
        assert!(handle(server, &subscriber_config(), Arc::clone(&shared)).is_err());
        assert!(
            !shared.lock().unwrap().shutdown,
            "the daemon must stay alive when the shutdown reply was not sent"
        );
    }
}
