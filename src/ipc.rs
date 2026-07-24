use crate::model::SessionView;
use crate::model::State;
use crate::tmux::Topology;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    Event { request: Box<HookRequest> },
    Subscribe,
    Ping,
    Shutdown,
    Status,
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
    pub status: Option<String>,
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
            status: None,
            error: None,
        }
    }
    pub fn state(
        revision: u64,
        state: State,
        topology: Topology,
        views: Vec<SessionView>,
        status: String,
    ) -> Self {
        Self {
            revision,
            state: Some(state),
            topology: Some(topology),
            views: Some(views),
            status: Some(status),
            error: None,
        }
    }

    pub fn status(revision: u64, status: String) -> Self {
        Self {
            revision,
            state: None,
            topology: None,
            views: None,
            status: Some(status),
            error: None,
        }
    }

    pub fn health(
        revision: u64,
        topology: Topology,
        views: Vec<SessionView>,
        status: String,
    ) -> Self {
        Self {
            revision,
            state: None,
            topology: Some(topology),
            views: Some(views),
            status: Some(status),
            error: None,
        }
    }
}
