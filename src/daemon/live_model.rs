use crate::config::Config;
use crate::model::{SessionView, State};
use crate::tmux::Topology;

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub revision: u64,
    pub state: State,
    pub topology: Topology,
    pub views: Vec<SessionView>,
}

/// Owns the daemon's observable invariant:
///
/// `state + topology -> views + revision`
///
/// Operational facts such as monitor identity, shutdown, and maintenance are
/// deliberately kept out of this model.
#[derive(Clone)]
pub(super) struct LiveModel {
    revision: u64,
    state: State,
    topology: Topology,
    views: Vec<SessionView>,
}

impl LiveModel {
    pub fn new(config: &Config, state: State, topology: Topology) -> Self {
        let views = crate::sessions::views_with_topology(config, &state, &topology);
        Self {
            revision: 0,
            state,
            topology,
            views,
        }
    }

    pub fn apply_event_state(&mut self, config: &Config, state: State) -> u64 {
        self.state = state;
        self.publish(config)
    }

    pub fn clear(&mut self, config: &Config) -> u64 {
        self.state = State::initial();
        self.publish(config)
    }

    pub fn apply_topology(&mut self, config: &Config, topology: Topology) -> bool {
        if self.topology == topology {
            return false;
        }
        self.topology = topology;
        self.publish(config);
        true
    }

    pub fn response_snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            state: self.state.clone(),
            topology: self.topology.clone(),
            views: self.views.clone(),
        }
    }

    pub fn topology(&self) -> &Topology {
        &self.topology
    }

    pub fn retain_keys(&self) -> std::collections::BTreeSet<String> {
        self.state.records.keys().cloned().collect()
    }

    fn publish(&mut self, config: &Config) -> u64 {
        self.views = crate::sessions::views_with_topology(config, &self.state, &self.topology);
        self.revision += 1;
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;
    use crate::tmux::{Pane, TmuxSession};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn config() -> Config {
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

    fn topology(title: &str) -> Topology {
        Topology {
            sessions: vec![TmuxSession {
                name: "work".to_owned(),
                ..TmuxSession::default()
            }],
            panes: vec![Pane {
                session: "work".to_owned(),
                pane: "%1".to_owned(),
                command: "codex".to_owned(),
                title: title.to_owned(),
                ..Pane::default()
            }],
            connected: true,
            ..Topology::default()
        }
    }

    fn event_state(status: &str) -> State {
        State {
            version: 1,
            records: BTreeMap::from([(
                "codex:work:%1".to_owned(),
                Record {
                    agent: "codex".to_owned(),
                    tmux_session: "work".to_owned(),
                    tmux_pane: "%1".to_owned(),
                    status: status.to_owned(),
                    updated_at: crate::event::now(),
                    ..Record::default()
                },
            )]),
        }
    }

    #[test]
    fn event_and_clear_publish_coherent_snapshots() {
        let config = config();
        let mut model = LiveModel::new(&config, State::initial(), topology("initial"));
        assert_eq!(model.response_snapshot().revision, 0);

        assert_eq!(model.apply_event_state(&config, event_state("running")), 1);
        let event = model.response_snapshot();
        assert_eq!(event.revision, 1);
        assert_eq!(event.state.records.len(), 1);
        assert_eq!(event.views[0].agents[0].status, "running");
        assert_eq!(event.views[0].agents[0].title, "initial");

        assert_eq!(model.clear(&config), 2);
        let cleared = model.response_snapshot();
        assert_eq!(cleared.revision, 2);
        assert!(cleared.state.records.is_empty());
        assert_eq!(cleared.views[0].agent_count, 1);
        assert_eq!(cleared.views[0].agents[0].status, "running");
    }

    #[test]
    fn topology_only_publishes_when_changed() {
        let config = config();
        let original = topology("original");
        let mut model = LiveModel::new(&config, event_state("attention"), original.clone());

        assert!(!model.apply_topology(&config, original));
        assert_eq!(model.response_snapshot().revision, 0);

        assert!(model.apply_topology(&config, topology("changed")));
        let changed = model.response_snapshot();
        assert_eq!(changed.revision, 1);
        assert_eq!(changed.topology.panes[0].title, "changed");
        assert_eq!(changed.views[0].agents[0].title, "changed");
        assert_eq!(changed.state.records.len(), 1);
    }
}
