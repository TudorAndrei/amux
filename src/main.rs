mod config;
mod daemon;
mod event;
mod hooks;
mod ipc;
mod model;
mod render;
mod sessions;
mod state;
mod tmux;
mod ui;

use clap::{Args, CommandFactory, Parser, Subcommand};
use config::Config;
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{self, Read};
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
    /// Print a compact tmux status segment.
    Status,
    /// Print all tmux sessions with amux status.
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
    Daemon,
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

fn refresh_tmux() {
    if env::var_os("TMUX").is_some() {
        let _ = Command::new("tmux").args(["refresh-client", "-S"]).output();
    }
}

fn cmd_event(config: &Config, args: EventArgs) -> Result<(), String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|error| error.to_string())?;
    if source.trim().is_empty() {
        source = "{}".to_owned();
    }
    let raw: Value =
        serde_json::from_str(&source).map_err(|_| "event input is not valid JSON".to_owned())?;
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
        tmux_pane: if env::var_os("TMUX").is_some() {
            env::var("TMUX_PANE").unwrap_or_default()
        } else {
            String::new()
        },
    };
    let written_by_daemon = if env::var_os("AMUX_NO_DAEMON").is_none() {
        daemon::send_event(config, hook_request.clone()).is_ok()
            || start_daemon(config).is_ok() && send_after_start(config, hook_request.clone())
    } else {
        false
    };
    if !written_by_daemon {
        let (key, record) = event::normalize(
            &args.agent,
            &args.event,
            &args.status,
            &args.attention,
            &args.reason,
            raw,
        );
        let timestamp = record.updated_at;
        let mut event_fields = serde_json::to_value(&record)
            .map_err(|error| error.to_string())?
            .as_object()
            .cloned()
            .unwrap_or_default();
        event_fields.insert("key".to_owned(), Value::String(key.clone()));
        let event_log = Value::Object(event_fields);
        state::write_event(config, key, record, &event_log, timestamp)?;
    }
    refresh_tmux();
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

fn send_after_start(config: &Config, request: ipc::HookRequest) -> bool {
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(15));
        if daemon::send_event(config, request.clone()).is_ok() {
            return true;
        }
    }
    false
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
    println!("rust: ok");
    let mut failure = false;
    if let Some(version) = tmux_version() {
        println!("tmux: ok ({version})");
    } else {
        println!("tmux: missing");
        failure = true;
    }
    if config.state_file().exists() {
        match state::load(config) {
            Ok(state) if state.version == 1 => {
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
    let socket = daemon::socket_path(config);
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
        match daemon::health(config) {
            Ok((revision, topology, _, _)) if topology.connected => println!(
                "monitor: connected (revision {revision}, reconciled {})",
                topology.reconciled_at
            ),
            Ok((revision, topology, _, _)) => {
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
    let target = views
        .iter()
        .flat_map(|session| session.agents.iter())
        .filter(|agent| agent.attention && agent.live)
        .max_by_key(|agent| agent.updated_at);
    if let Some(target) = target {
        if !target.session.is_empty() {
            let _ = Command::new("tmux")
                .args(["switch-client", "-t", &target.session])
                .status();
        }
        if !target.pane.is_empty() {
            let _ = Command::new("tmux")
                .args(["select-pane", "-t", &target.pane])
                .status();
        }
    } else if Command::new("tmux")
        .args(["display-message", "amux: no agents need attention"])
        .status()
        .is_err()
    {
        println!("amux: no agents need attention");
    }
    Ok(())
}

fn cmd_status(config: &Config) -> Result<(), String> {
    let status = daemon::cached_status(config).unwrap_or_else(|_| {
        state::load(config)
            .map(|state| render::status(config, &sessions::views(config, &state)))
            .unwrap_or_default()
    });
    print!("{status}");
    Ok(())
}

fn session_views(config: &Config) -> Result<Vec<model::SessionView>, String> {
    daemon::cached_views(config)
        .or_else(|_| state::load(config).map(|state| sessions::views(config, &state)))
}

fn main() -> ExitCode {
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
        Commands::Status => cmd_status(&config).map(|_| 0),
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
        Commands::Clear => state::clear(&config).map(|_| 0),
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
        Commands::Daemon => daemon::run(config).map(|_| 0),
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
