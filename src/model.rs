use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct State {
    pub version: u8,
    #[serde(default)]
    pub records: BTreeMap<String, Record>,
}

impl State {
    pub fn initial() -> Self {
        Self {
            version: 1,
            records: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Record {
    pub agent: String,
    #[serde(default)]
    pub agent_session_id: String,
    #[serde(default)]
    pub tmux_session: String,
    #[serde(default)]
    pub tmux_window: String,
    #[serde(default)]
    pub tmux_pane: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub attention: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub last_event: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub updated_at_iso: String,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentView {
    pub agent: String,
    pub agent_session_id: String,
    pub session: String,
    pub pane: String,
    pub command: String,
    pub title: String,
    pub cwd: String,
    pub status: String,
    pub attention: bool,
    pub reason: String,
    pub last_event: String,
    pub updated_at: i64,
    pub live: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionView {
    pub session: String,
    pub last_attached: i64,
    pub attached: bool,
    pub status: String,
    pub attention: bool,
    pub agent_count: usize,
    pub live_agent_count: usize,
    pub agents: Vec<AgentView>,
    pub pane: String,
    pub reason: String,
    pub cwd: String,
    pub updated_at: i64,
}
