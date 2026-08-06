use crate::config::Config;
use crate::{daemon, event, hooks, intake, ipc, model, render, sessions, state, tmux, ui};
use clap::{Args, CommandFactory, Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "amux", version, about = "Agent multiplexer for tmux")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Read raw hook JSON from stdin and update amux state.
    Event(EventArgs),
    /// Print all tmux sessions with amux state.
    Sessions {
        /// Emit the session model as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print tracked agent records.
    List {
        /// Emit the persisted state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print retained lifecycle transitions.
    Events(EventsArgs),
    /// Stream live session revisions.
    Watch(WatchArgs),
    /// Remove amux state and its event log.
    Clear,
    /// Open the native amux picker.
    Picker {
        /// Print deterministic picker rows without opening a terminal UI.
        #[arg(long)]
        rows: bool,
    },
    /// Switch to the most recently updated agent needing attention.
    #[command(name = "next-attention")]
    NextAttention,
    /// Run the persistent amux daemon.
    Daemon {
        /// Ask a running daemon to exit instead of starting one.
        #[arg(long)]
        stop: bool,
    },
    /// Check local dependencies and state paths.
    Doctor,
    /// Install global agent hooks. The default is read-only dry-run mode.
    #[command(name = "install-hooks")]
    InstallHooks(HookMode),
    /// Remove global amux hooks. The default is read-only dry-run mode.
    #[command(name = "uninstall-hooks")]
    UninstallHooks(HookMode),
}

#[derive(Debug, Args)]
struct EventArgs {
    /// Agent product that emitted the hook event.
    #[arg(long)]
    agent: String,
    /// Explicit hook event name.
    #[arg(long, default_value = "")]
    event: String,
    /// Explicit derived status override.
    #[arg(long, default_value = "")]
    status: String,
    /// Explicit attention override (0, 1, false, or true).
    #[arg(long, default_value = "")]
    attention: String,
    /// Explicit short status reason.
    #[arg(long, default_value = "")]
    reason: String,
}

#[derive(Debug, Args)]
struct EventsArgs {
    /// Keep events from this agent product only.
    #[arg(long)]
    agent: Option<String>,
    /// Keep events from this tmux or agent session only.
    #[arg(long)]
    session: Option<String>,
    /// Keep events from this tmux pane only.
    #[arg(long)]
    pane: Option<String>,
    /// Return at most this many newest matches, in chronological order.
    #[arg(long, default_value_t = 100, value_parser = parse_event_limit)]
    limit: usize,
    /// Emit a versioned JSON object instead of tab-separated text.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct WatchArgs {
    /// Emit one newline-delimited JSON object per revision.
    #[arg(long, required = true)]
    json: bool,
}

fn parse_event_limit(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(value @ 1..=state::MAX_EVENT_HISTORY_LIMIT) => Ok(value),
        _ => Err(format!(
            "limit must be between 1 and {}",
            state::MAX_EVENT_HISTORY_LIMIT
        )),
    }
}

#[derive(Debug, Args)]
struct HookMode {
    /// Write configuration files. Without this flag, no files are modified.
    #[arg(long, conflicts_with = "dry_run")]
    write: bool,
    /// Preview changes without modifying files (the default).
    #[arg(long, conflicts_with = "write")]
    dry_run: bool,
}

impl HookMode {
    fn mode(&self) -> hooks::Mode {
        if self.write {
            hooks::Mode::Write
        } else {
            hooks::Mode::DryRun
        }
    }
}

fn die(message: impl AsRef<str>) -> ExitCode {
    eprintln!("amux: {}", message.as_ref());
    ExitCode::FAILURE
}

fn cmd_event(config: &Config, args: EventArgs) -> Result<(), String> {
    let raw = intake::read_hook_json(io::stdin().lock())?;
    // The daemon already owns a live tmux topology. Passing just the pane id
    // avoids two `tmux display-message` processes for every hook. The direct
    // fallback still resolves the full context below.
    let tmux_pane = env::var("TMUX_PANE").unwrap_or_default();
    let hook_request = ipc::HookRequest {
        agent: args.agent.clone(),
        event: args.event.clone(),
        status: args.status.clone(),
        attention: args.attention.clone(),
        reason: args.reason.clone(),
        raw: raw.clone(),
        cwd: env::current_dir()
            .ok()
            .and_then(|path| path.into_os_string().into_string().ok())
            .unwrap_or_default(),
        tmux_pane,
        tmux_session: String::new(),
        tmux_window: String::new(),
        tmux_server: tmux::server_from_env(),
    };
    let written_by_daemon = if env::var_os("AMUX_NO_DAEMON").is_none() {
        match ipc::send_event(config, hook_request.clone()) {
            Ok(()) => true,
            Err(ipc::ClientError::Unavailable(_)) => {
                start_daemon(config).is_ok() && send_after_start(config, hook_request.clone())?
            }
            Err(error) => return Err(error.to_string()),
        }
    } else {
        false
    };
    if !written_by_daemon {
        intake::persist(config, hook_request, event::current_tmux_context())?;
    }
    Ok(())
}

fn start_daemon(config: &Config) -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    Command::new(executable)
        .arg("daemon")
        .env("AMUX_STATE_DIR", &config.state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn send_after_start(config: &Config, request: ipc::HookRequest) -> Result<bool, String> {
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(15));
        match ipc::send_event(config, request.clone()) {
            Ok(()) => return Ok(true),
            Err(ipc::ClientError::Unavailable(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(false)
}

fn watch_subscription(
    config: &Config,
) -> Result<std::sync::mpsc::Receiver<Result<ipc::SubscriptionUpdate, ipc::ClientError>>, String> {
    match ipc::subscribe(config) {
        Ok(receiver) => Ok(receiver),
        Err(ipc::ClientError::Unavailable(_)) => {
            start_daemon(config)?;
            ipc::subscribe_with_retry(config, 20, Duration::from_millis(15))
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn cmd_watch(config: &Config, args: WatchArgs) -> Result<(), String> {
    debug_assert!(args.json);
    let updates = watch_subscription(config)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    for update in updates {
        let update = update.map_err(|error| error.to_string())?;
        let mut line = serde_json::to_vec(&update).map_err(|error| error.to_string())?;
        line.push(b'\n');
        if let Err(error) = stdout.write_all(&line).and_then(|_| stdout.flush()) {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(error.to_string());
        }
    }
    Err("daemon subscription ended unexpectedly".to_owned())
}

fn cmd_doctor(config: &Config) -> i32 {
    let binary = env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    println!("binary: {binary}");
    println!(
        "amux root: {}",
        env::var("AMUX_ROOT").unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned())
    );
    println!("state dir: {}", config.state_dir.display());
    println!("stale seconds: {}", config.stale_seconds);
    println!("events compact bytes: {}", config.events_compact_bytes);
    println!(
        "locking: {}",
        if config.locking_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("events per session: {}", config.events_per_session);
    for override_name in &config.rejected_overrides {
        println!("config: rejected {override_name}; using default");
    }
    println!("rust: ok");
    let mut failure = false;
    if let Some(version) = tmux_version() {
        println!("tmux: ok ({version})");
    } else {
        println!("tmux: missing");
        failure = true;
    }
    if config.state_file().exists() {
        match state::load_for_inspection(config) {
            Ok(state) if state.version == state::STATE_VERSION => {
                println!("state: v1 compatible ({})", config.state_file().display());
            }
            Ok(state) => {
                println!("state: unsupported version {}", state.version);
                failure = true;
            }
            Err(error) => {
                println!("state: invalid ({error})");
                failure = true;
            }
        }
    } else {
        println!("state: not created yet");
    }
    if config.maintenance_diagnostic_file().exists() {
        let detail = fs::read_to_string(config.maintenance_diagnostic_file())
            .unwrap_or_else(|error| format!("unreadable diagnostic: {error}"));
        println!("maintenance: degraded ({})", detail.trim());
        failure = true;
    } else {
        println!("maintenance: ok");
    }
    let socket = ipc::socket_path(config);
    if socket.exists() {
        #[cfg(unix)]
        match fs::symlink_metadata(&socket) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && metadata.permissions().mode() & 0o777 == 0o600 =>
            {
                println!("daemon socket: private ({})", socket.display());
            }
            Ok(_) => {
                println!("daemon socket: unsafe ({})", socket.display());
                failure = true;
            }
            Err(error) => {
                println!("daemon socket: unreadable ({error})");
                failure = true;
            }
        }
        match ipc::health(config) {
            Ok((revision, topology, _)) if topology.connected => println!(
                "monitor: connected (revision {revision}, reconciled {})",
                topology.reconciled_at
            ),
            Ok((revision, topology, _)) => {
                println!("monitor: idle (revision {revision}: {})", topology.error)
            }
            Err(error) => {
                println!("daemon: unavailable ({error})");
                failure = true;
            }
        }
    } else {
        println!("daemon socket: not running");
        println!("monitor: unavailable until a daemon is started");
    }
    match hooks::inspect() {
        Ok(reports) => {
            for report in reports {
                match report.result {
                    Ok(drifts) if drifts.is_empty() => {
                        println!("hooks {}: current", report.name);
                    }
                    Ok(drifts) => {
                        for drift in drifts {
                            println!("hooks {drift}; run `amux install-hooks --write`");
                        }
                    }
                    Err(error) => {
                        println!("hooks {}: unverified ({error})", report.name);
                    }
                }
            }
        }
        Err(error) => println!("hooks: could not resolve integration paths ({error})"),
    }
    if failure { 1 } else { 0 }
}

fn tmux_version() -> Option<String> {
    Command::new("tmux")
        .arg("-V")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn cmd_next_attention(config: &Config) -> Result<(), String> {
    let views = session_views(config)?;
    let target = sessions::newest_live_attention(&views);
    if let Some(target) = target {
        tmux::switch_client(&target.session, &target.pane)?;
    } else if Command::new("tmux")
        .args(["display-message", "amux: no agents need attention"])
        .status()
        .is_err()
    {
        println!("amux: no agents need attention");
    }
    Ok(())
}

fn session_views(config: &Config) -> Result<Vec<model::SessionView>, String> {
    ipc::cached_views(config)
        .map_err(|error| error.to_string())
        .or_else(|_| state::load(config).map(|state| sessions::views(config, &state)))
}

pub fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return if code == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    let Some(command) = cli.command else {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::SUCCESS;
    };
    let config = Config::from_env();
    let result = match command {
        Commands::Event(args) => cmd_event(&config, args).map(|_| 0),
        Commands::List { json: false } => state::load(&config).map(|state| {
            let text = render::list(&sessions::list_records(&config, &state));
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }),
        Commands::List { json: true } => state::load(&config).and_then(|state| {
            println!(
                "{}",
                serde_json::to_string(&state).map_err(|error| error.to_string())?
            );
            Ok(0)
        }),
        Commands::Sessions { json: false } => session_views(&config).map(|views| {
            let text = render::sessions(&views);
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }),
        Commands::Sessions { json: true } => session_views(&config).and_then(|views| {
            println!(
                "{}",
                serde_json::to_string(&views).map_err(|error| error.to_string())?
            );
            Ok(0)
        }),
        Commands::Events(args) => {
            let filter = state::EventHistoryFilter {
                agent: args.agent.as_deref(),
                session: args.session.as_deref(),
                pane: args.pane.as_deref(),
            };
            state::event_history(&config, filter, args.limit).and_then(|events| {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "version": 1,
                            "events": events,
                        }))
                        .map_err(|error| error.to_string())?
                    );
                } else {
                    let text = render::events(&events);
                    if !text.is_empty() {
                        println!("{text}");
                    }
                }
                Ok(0)
            })
        }
        Commands::Watch(args) => cmd_watch(&config, args).map(|_| 0),
        Commands::Clear => match ipc::clear(&config) {
            Ok(()) => Ok(0),
            Err(ipc::ClientError::Unavailable(_)) => state::clear(&config).map(|_| 0),
            Err(error) => Err(error.to_string()),
        },
        Commands::Picker { rows: false } => {
            let _ = start_daemon(&config);
            ui::run(config).map(|_| 0)
        }
        Commands::Picker { rows: true } => ui::rows(&config).map(|rows| {
            if !rows.is_empty() {
                println!("{rows}");
            }
            0
        }),
        Commands::NextAttention => cmd_next_attention(&config).map(|_| 0),
        Commands::Daemon { stop: true } => ipc::stop(&config)
            .map(|_| 0)
            .map_err(|error| error.to_string()),
        Commands::Daemon { stop: false } => daemon::run(config).map(|_| 0),
        Commands::Doctor => Ok(cmd_doctor(&config)),
        Commands::InstallHooks(mode) => hooks::install(mode.mode()).map(|_| 0),
        Commands::UninstallHooks(mode) => hooks::uninstall(mode.mode()).map(|_| 0),
    };
    match result {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => die(error),
    }
}
