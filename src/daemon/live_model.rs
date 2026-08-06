use crate::config::Config;
use crate::model::{SessionView, State};
use crate::tmux::Topology;

#[derive(Debug)]
pub(super) struct HealthSnapshot {
    pub revision: u64,
    pub topology: Topology,
    pub views: Vec<SessionView>,
}

#[derive(Debug)]
pub(super) struct ViewRevision {
    pub revision: u64,
    pub views: Vec<SessionView>,
}

/// Owns the daemon's observable invariant:
///
/// `state + topology -> views + revision`
///
/// Batch transform contract:
///
/// - Inputs are one owned `State` record map and one owned `Topology` with flat
///   session and pane arrays.
/// - Each publish replaces one input and rebuilds the complete `Vec<SessionView>`;
///   one session is the batch path with a count of one.
/// - `LiveModel` owns all inputs and cached views for its daemon lifetime.
///   Health and subscription outputs own their copies after the lock is released.
/// - Revisions start at zero and increase to `u64::MAX`; overflow fails loudly.
///   A subscriber revision above the current revision is an invariant failure.
///
/// Operational facts such as monitor identity, shutdown, and maintenance are
/// deliberately kept out of this model.
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

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn health_snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            revision: self.revision,
            topology: self.topology.clone(),
            views: self.views.clone(),
        }
    }

    /// Returns the current views for a new subscriber, or only when the model
    /// has advanced for an existing subscriber. The daemon owns
    /// `last_revision`, so a value ahead of the live model is an invariant
    /// failure rather than wire input to recover from.
    pub fn subscription_update(&self, last_revision: Option<u64>) -> Option<ViewRevision> {
        if let Some(last_revision) = last_revision {
            assert!(
                last_revision <= self.revision,
                "subscriber revision cannot exceed the live revision"
            );
            if last_revision == self.revision {
                return None;
            }
        }
        Some(ViewRevision {
            revision: self.revision,
            views: self.views.clone(),
        })
    }

    pub fn topology(&self) -> &Topology {
        &self.topology
    }

    fn publish(&mut self, config: &Config) -> u64 {
        self.views = crate::sessions::views_with_topology(config, &self.state, &self.topology);
        self.revision = self
            .revision
            .checked_add(1)
            .expect("live model revision exhausted");
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

    fn representative_model() -> LiveModel {
        let topology = Topology {
            sessions: (0..100)
                .map(|index| TmuxSession {
                    id: format!("${index}"),
                    name: format!("session-{index}"),
                    last_attached: index,
                    attached: false,
                })
                .collect(),
            panes: (0..200)
                .map(|index| Pane {
                    session: format!("session-{}", index % 100),
                    pane: format!("%{index}"),
                    command: "codex".to_owned(),
                    ..Pane::default()
                })
                .collect(),
            connected: true,
            ..Topology::default()
        };
        let state = State {
            version: 1,
            records: (0..200)
                .map(|index| {
                    (
                        format!("codex:{index}"),
                        Record {
                            agent: "codex".to_owned(),
                            tmux_session: format!("session-{}", index % 100),
                            tmux_pane: format!("%{index}"),
                            updated_at: 1,
                            ..Record::default()
                        },
                    )
                })
                .collect(),
        };
        LiveModel::new(&config(), state, topology)
    }

    #[test]
    fn measures_idle_clones_and_subscription_bytes() {
        let model = representative_model();
        let idle_polls = 20;
        let mut changed_updates = 0;
        for _ in 0..idle_polls {
            let legacy_snapshot = (
                model.revision,
                model.state.clone(),
                model.topology.clone(),
                model.views.clone(),
            );
            assert_eq!(legacy_snapshot.0, 0);
            std::hint::black_box(legacy_snapshot);
            changed_updates += usize::from(model.subscription_update(Some(0)).is_some());
        }
        let state_bytes = serde_json::to_vec(&model.state).unwrap().len();
        let topology_bytes = serde_json::to_vec(&model.topology).unwrap().len();
        let views_bytes = serde_json::to_vec(&model.views).unwrap().len();
        let legacy_snapshot_bytes = state_bytes + topology_bytes + views_bytes;
        let legacy_bytes_per_revision = serde_json::to_vec(&serde_json::json!({
            "revision": model.revision,
            "state": &model.state,
            "topology": &model.topology,
            "views": &model.views,
        }))
        .unwrap()
        .len()
            + 1;
        let update = model.subscription_update(None).unwrap();
        let bytes_per_revision = serde_json::to_vec(&serde_json::json!({
            "revision": update.revision,
            "views": update.views,
        }))
        .unwrap()
        .len()
            + 1;
        eprintln!(
            "legacy subscription: idle_checks={idle_polls}, state_clones={idle_polls}, topology_clones={idle_polls}, view_clones={idle_polls}, snapshot_bytes={legacy_snapshot_bytes}, bytes_per_revision={legacy_bytes_per_revision}"
        );
        eprintln!(
            "current subscription: idle_checks={idle_polls}, state_clones=0, topology_clones=0, view_clones={changed_updates}, snapshot_bytes={views_bytes}, bytes_per_revision={bytes_per_revision}"
        );
        assert_eq!(changed_updates, 0);
        assert!(views_bytes < legacy_snapshot_bytes);
        assert!(bytes_per_revision < legacy_bytes_per_revision);
    }

    #[test]
    fn event_and_clear_publish_coherent_snapshots() {
        let config = config();
        let mut model = LiveModel::new(&config, State::initial(), topology("initial"));
        assert_eq!(model.revision(), 0);

        assert_eq!(model.apply_event_state(&config, event_state("running")), 1);
        let event = model.subscription_update(Some(0)).unwrap();
        assert_eq!(event.revision, 1);
        assert_eq!(event.views[0].agents[0].status, "running");
        assert_eq!(event.views[0].agents[0].title, "initial");
        assert_eq!(model.state.records.len(), 1);

        assert_eq!(model.clear(&config), 2);
        let cleared = model.subscription_update(Some(1)).unwrap();
        assert_eq!(cleared.revision, 2);
        assert_eq!(cleared.views[0].agent_count, 1);
        assert_eq!(cleared.views[0].agents[0].status, "running");
        assert!(model.state.records.is_empty());
    }

    #[test]
    fn topology_only_publishes_when_changed() {
        let config = config();
        let original = topology("original");
        let mut model = LiveModel::new(&config, event_state("attention"), original.clone());

        assert!(!model.apply_topology(&config, original));
        assert_eq!(model.revision(), 0);
        assert!(model.subscription_update(Some(0)).is_none());

        assert!(model.apply_topology(&config, topology("changed")));
        let changed = model.health_snapshot();
        assert_eq!(changed.revision, 1);
        assert_eq!(changed.topology.panes[0].title, "changed");
        assert_eq!(changed.views[0].agents[0].title, "changed");
    }
}
