use serde::{Deserialize, Serialize};
use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn tmux_value(format: &str, pane: &str) -> String {
    if env::var_os("TMUX").is_none() {
        return String::new();
    }
    let mut command = Command::new("tmux");
    command.arg("display-message").arg("-p");
    if !pane.is_empty() {
        command.arg("-t").arg(pane);
    }
    command.arg(format);
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TmuxContext {
    pub pane: String,
    pub session: String,
    pub window: String,
}

pub fn current_tmux_context() -> TmuxContext {
    if env::var_os("TMUX").is_none() {
        return TmuxContext::default();
    }
    let pane = env::var("TMUX_PANE").unwrap_or_else(|_| tmux_value("#{pane_id}", ""));
    TmuxContext {
        session: tmux_value("#{session_name}", &pane),
        window: tmux_value("#{window_id}", &pane),
        pane,
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
