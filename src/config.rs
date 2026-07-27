use std::env;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub state_dir: PathBuf,
    pub stale_seconds: i64,
    pub events_per_session: usize,
    pub events_compact_bytes: u64,
    pub hide_subagents: bool,
    pub use_color: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let state_dir = env::var_os("AMUX_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("XDG_STATE_HOME")
                    .map(|home| PathBuf::from(home).join("amux"))
                    .unwrap_or_else(|| {
                        PathBuf::from(env::var_os("HOME").unwrap_or_default())
                            .join(".local/state/amux")
                    })
            });
        let stale_seconds = env::var("AMUX_STALE_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(86_400);
        let events_per_session = env::var("AMUX_EVENTS_PER_SESSION")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(200);
        let events_compact_bytes = env::var("AMUX_EVENTS_COMPACT_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8 * 1024 * 1024);
        let hide_subagents = !matches!(
            env::var("AMUX_HIDE_SUBAGENTS").as_deref(),
            Ok("0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF")
        );
        let use_color = env::var("AMUX_COLOR").unwrap_or_else(|_| "1".to_owned()) != "0"
            && env::var("AMUX_PLAIN").unwrap_or_else(|_| "0".to_owned()) != "1"
            && env::var_os("NO_COLOR").is_none();
        Self {
            state_dir,
            stale_seconds,
            events_per_session,
            events_compact_bytes,
            hide_subagents,
            use_color,
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }
    pub fn events_file(&self) -> PathBuf {
        self.state_dir.join("events.jsonl")
    }
    pub fn lock_dir(&self) -> PathBuf {
        self.state_dir.join("state.lock")
    }
    pub fn compacting_events_file(&self) -> PathBuf {
        self.state_dir.join("events.jsonl.compacting")
    }
    pub fn retained_events_file(&self) -> PathBuf {
        self.state_dir.join("events.jsonl.retained")
    }
}
