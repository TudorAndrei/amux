use crate::config::Config;
use crate::event;
use crate::ipc::{HookRequest, Request, Response};
use crate::model::{SessionView, State};
use crate::state;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn socket_path(config: &Config) -> PathBuf {
    config.state_dir.join("amux.sock")
}

#[derive(Clone)]
struct Shared {
    revision: u64,
    state: State,
    topology: crate::tmux::Topology,
    views: Vec<SessionView>,
    status: String,
    shutdown: bool,
    monitor_server: Option<PathBuf>,
    monitor_stop: Option<Arc<AtomicBool>>,
}

pub fn run(config: Config) -> Result<(), String> {
    fs::create_dir_all(&config.state_dir).map_err(|error| error.to_string())?;
    fs::set_permissions(&config.state_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
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
    let state = state::load(&config)?;
    let topology = crate::tmux::Topology::default();
    let views = crate::sessions::views_with_topology(&config, &state, &topology);
    let status = crate::render::status(&config, &views);
    let shared = Arc::new(Mutex::new(Shared {
        revision: 0,
        state,
        topology,
        views,
        status,
        shutdown: false,
        monitor_server: None,
        monitor_stop: None,
    }));
    attach_monitor(&config, &shared, crate::tmux::server_from_env());
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    loop {
        if shared
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .shutdown
        {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let config = config.clone();
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = handle(stream, &config, shared);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
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

fn reply(stream: &mut UnixStream, response: &Response) -> Result<(), String> {
    serde_json::to_writer(&mut *stream, response).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
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
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let request: Request =
        serde_json::from_str(&line).map_err(|error| format!("invalid daemon request: {error}"))?;
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
        Request::Status => {
            let guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            reply(
                &mut stream,
                &Response::status(guard.revision, guard.status.clone()),
            )
        }
        Request::Health => {
            let guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            reply(
                &mut stream,
                &Response::health(
                    guard.revision,
                    guard.topology.clone(),
                    guard.views.clone(),
                    guard.status.clone(),
                ),
            )
        }
        Request::Event { request } => {
            let request = *request;
            attach_monitor(config, &shared, request.tmux_server.clone());
            let topology = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .topology
                .clone();
            let tmux = context_for_pane(&topology, &request.tmux_pane)
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
            state::write_event(
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
            guard.status = crate::render::status(config, &guard.views);
            guard.revision += 1;
            reply(&mut stream, &Response::ok(guard.revision))
        }
        Request::Clear => {
            state::clear(config)?;
            let mut guard = shared
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            guard.state = State::initial();
            guard.views =
                crate::sessions::views_with_topology(config, &guard.state, &guard.topology);
            guard.status = crate::render::status(config, &guard.views);
            guard.revision += 1;
            reply(&mut stream, &Response::ok(guard.revision))
        }
        Request::Subscribe => {
            let (mut revision, initial, topology, views, status) = {
                let guard = shared
                    .lock()
                    .map_err(|_| "daemon state lock poisoned".to_owned())?;
                (
                    guard.revision,
                    guard.state.clone(),
                    guard.topology.clone(),
                    guard.views.clone(),
                    guard.status.clone(),
                )
            };
            reply(
                &mut stream,
                &Response::state(revision, initial, topology, views, status),
            )?;
            loop {
                thread::sleep(Duration::from_millis(50));
                let guard = shared
                    .lock()
                    .map_err(|_| "daemon state lock poisoned".to_owned())?;
                if guard.shutdown {
                    return Ok(());
                }
                if guard.revision != revision {
                    revision = guard.revision;
                    reply(
                        &mut stream,
                        &Response::state(
                            revision,
                            guard.state.clone(),
                            guard.topology.clone(),
                            guard.views.clone(),
                            guard.status.clone(),
                        ),
                    )?;
                }
            }
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
            guard.status = crate::render::status(&config_for_monitor, &guard.views);
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

pub fn cached_status(config: &Config) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket_path(config)).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &Request::Status).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| error.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let response: Response = serde_json::from_str(&line).map_err(|error| error.to_string())?;
    response
        .status
        .ok_or_else(|| "daemon did not return cached status".to_owned())
}

pub fn health(
    config: &Config,
) -> Result<(u64, crate::tmux::Topology, Vec<SessionView>, String), String> {
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
        response.status.unwrap_or_default(),
    ))
}

pub fn cached_views(config: &Config) -> Result<Vec<SessionView>, String> {
    health(config).map(|(_, _, views, _)| views)
}

fn context_for_pane(
    topology: &crate::tmux::Topology,
    pane_id: &str,
) -> Option<crate::event::TmuxContext> {
    topology.panes.iter().find_map(|line| {
        let fields: Vec<_> = line.split('|').collect();
        (fields.len() >= 4 && fields[3] == pane_id).then(|| crate::event::TmuxContext {
            session: fields[1].to_owned(),
            window: fields[2].to_owned(),
            pane: fields[3].to_owned(),
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
