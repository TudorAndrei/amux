use crate::model::{SessionView, State};
use crate::tmux::Topology;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

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
pub struct Response {
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology: Option<Topology>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<Vec<SessionView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(revision: u64) -> Self {
        Self {
            revision,
            state: None,
            topology: None,
            views: None,
            error: None,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            revision: 0,
            state: None,
            topology: None,
            views: None,
            error: Some(message.into()),
        }
    }
    pub fn state(revision: u64, state: State, topology: Topology, views: Vec<SessionView>) -> Self {
        Self {
            revision,
            state: Some(state),
            topology: Some(topology),
            views: Some(views),
            error: None,
        }
    }
    pub fn health(revision: u64, topology: Topology, views: Vec<SessionView>) -> Self {
        Self {
            revision,
            state: None,
            topology: Some(topology),
            views: Some(views),
            error: None,
        }
    }
}
