use crate::config::Config;
use crate::event::now;
use crate::model::{AgentView, Record, SessionView, State};

fn uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn subagent_record(record: &Record, hide: bool) -> bool {
    if !hide {
        return false;
    }
    let metadata = |key: &str| {
        record
            .raw
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.is_empty())
    };
    uuid_like(&record.tmux_session)
        || metadata("agent_id")
        || metadata("agent_type")
        || metadata("parent_agent_id")
        || metadata("parent_session_id")
        || record
            .raw
            .get("is_subagent")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || (record.tmux_session.is_empty()
            && record.cwd.is_empty()
            && uuid_like(&record.agent_session_id))
}

fn pane_agent(command: &str) -> Option<String> {
    if command.starts_with("codex") {
        Some("codex".to_owned())
    } else if matches!(command, "claude" | "pi" | "opencode") {
        Some(command.to_owned())
    } else {
        None
    }
}

fn status_rank(status: &str) -> i8 {
    match status {
        "attention" => 3,
        "done" => 2,
        "running" => 1,
        "offline" => 0,
        _ => -1,
    }
}
fn agent_rank(status: &str) -> i8 {
    match status {
        "attention" => 3,
        "running" => 2,
        "done" => 1,
        "offline" => 0,
        _ => -1,
    }
}

fn as_agent(
    record: &Record,
    pane: Option<&crate::tmux::Pane>,
    session: &str,
    live: bool,
) -> AgentView {
    AgentView {
        agent: record.agent.clone(),
        agent_session_id: record.agent_session_id.clone(),
        session: session.to_owned(),
        pane: pane
            .map(|item| item.pane.clone())
            .unwrap_or_else(|| record.tmux_pane.clone()),
        command: pane.map(|item| item.command.clone()).unwrap_or_default(),
        title: pane.map(|item| item.title.clone()).unwrap_or_default(),
        cwd: if record.cwd.is_empty() {
            pane.map(|item| item.cwd.clone()).unwrap_or_default()
        } else {
            record.cwd.clone()
        },
        status: record.status.clone(),
        attention: record.attention,
        reason: record.reason.clone(),
        last_event: record.last_event.clone(),
        updated_at: record.updated_at,
        live,
    }
}

pub fn views(config: &Config, state: &State) -> Vec<SessionView> {
    views_with_topology(config, state, &crate::tmux::direct_topology())
}

/// Derive session rows from a daemon-owned tmux control snapshot. Unlike
/// `views`, this performs no tmux subprocess work and is safe for UI and status
/// cache updates.
pub fn views_with_topology(
    config: &Config,
    state: &State,
    topology: &crate::tmux::Topology,
) -> Vec<SessionView> {
    views_from(
        config,
        state,
        topology.sessions.clone(),
        topology.panes.clone(),
    )
}

fn views_from(
    config: &Config,
    state: &State,
    tmux_sessions: Vec<crate::tmux::TmuxSession>,
    panes: Vec<crate::tmux::Pane>,
) -> Vec<SessionView> {
    let cutoff = now() - config.stale_seconds;
    let records: Vec<Record> = state
        .records
        .values()
        .filter(|record| {
            record.updated_at >= cutoff
                && !record.tmux_session.is_empty()
                && !subagent_record(record, config.hide_subagents)
        })
        .cloned()
        .collect();
    let sessions: Vec<_> = tmux_sessions
        .into_iter()
        .filter(|session| !config.hide_subagents || !uuid_like(&session.name))
        .collect();
    let mut output = Vec::new();
    for session in sessions {
        let agent_panes: Vec<_> = panes
            .iter()
            .filter(|pane| pane.session == session.name)
            .filter_map(|pane| pane_agent(&pane.command).map(|agent| (pane, agent)))
            .collect();
        let session_records: Vec<_> = records
            .iter()
            .filter(|record| record.tmux_session == session.name)
            .collect();
        let mut agents = Vec::new();
        for (pane, agent) in &agent_panes {
            let selected = session_records
                .iter()
                .filter(|record| record.tmux_pane == pane.pane && record.agent == *agent)
                .max_by_key(|record| record.updated_at);
            if let Some(record) = selected {
                agents.push(as_agent(record, Some(pane), &session.name, true));
            } else {
                agents.push(AgentView {
                    agent: agent.clone(),
                    agent_session_id: String::new(),
                    session: session.name.clone(),
                    pane: pane.pane.clone(),
                    command: pane.command.clone(),
                    title: pane.title.clone(),
                    cwd: pane.cwd.clone(),
                    status: "running".to_owned(),
                    attention: false,
                    reason: pane.title.clone(),
                    last_event: String::new(),
                    updated_at: session.last_attached,
                    live: true,
                });
            }
        }
        if agent_panes.is_empty()
            && let Some(record) = session_records
                .iter()
                .max_by_key(|record| record.updated_at)
        {
            let mut offline = as_agent(record, None, &session.name, false);
            offline.status = "offline".to_owned();
            offline.attention = false;
            offline.reason = "offline".to_owned();
            agents.push(offline);
        }
        let status = if agents.iter().any(|agent| agent.status == "attention") {
            "attention"
        } else if agents.iter().any(|agent| agent.status == "running") {
            "running"
        } else if agents.iter().any(|agent| agent.status == "done") {
            "done"
        } else if agents.iter().any(|agent| agent.status == "offline") {
            "offline"
        } else {
            "none"
        }
        .to_owned();
        agents.sort_by(|left, right| {
            agent_rank(&right.status)
                .cmp(&agent_rank(&left.status))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        let target = agents.first();
        let updated_at = agents
            .iter()
            .map(|agent| agent.updated_at)
            .max()
            .unwrap_or(session.last_attached);
        output.push(SessionView {
            session: session.name,
            last_attached: session.last_attached,
            attached: session.attached,
            status: status.clone(),
            attention: status == "attention",
            agent_count: agents.len(),
            live_agent_count: agents.iter().filter(|agent| agent.live).count(),
            pane: agents
                .iter()
                .find(|agent| agent.live)
                .map(|agent| agent.pane.clone())
                .unwrap_or_default(),
            reason: target.map(|agent| agent.reason.clone()).unwrap_or_default(),
            cwd: target.map(|agent| agent.cwd.clone()).unwrap_or_default(),
            updated_at,
            agents,
        });
    }
    output.sort_by(|left, right| {
        status_rank(&right.status)
            .cmp(&status_rank(&left.status))
            .then_with(|| right.last_attached.cmp(&left.last_attached))
    });
    output
}

pub fn list_records(config: &Config, state: &State) -> Vec<Record> {
    let cutoff = now() - config.stale_seconds;
    let mut records: Vec<_> = state
        .records
        .values()
        .filter(|record| record.updated_at >= cutoff)
        .cloned()
        .collect();
    records.sort_by(|left, right| {
        (!left.attention, &left.agent, left.updated_at)
            .cmp(&(!right.attention, &right.agent, right.updated_at))
            .reverse()
    });
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn cached_topology_keeps_attached_sessions() {
        let config = Config {
            state_dir: PathBuf::new(),
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        };
        let mut records = BTreeMap::new();
        records.insert(
            "codex:attached|pipe:%1".to_owned(),
            Record {
                agent: "codex".to_owned(),
                tmux_session: "attached|pipe".to_owned(),
                tmux_pane: "%1".to_owned(),
                status: "attention".to_owned(),
                attention: true,
                updated_at: now(),
                ..Record::default()
            },
        );
        let topology = crate::tmux::Topology {
            sessions: vec![crate::tmux::TmuxSession {
                id: "$0".to_owned(),
                name: "attached|pipe".to_owned(),
                last_attached: 42,
                attached: true,
            }],
            panes: vec![crate::tmux::Pane {
                session: "attached|pipe".to_owned(),
                window: "@0".to_owned(),
                pane: "%1".to_owned(),
                command: "codex".to_owned(),
                title: "codex".to_owned(),
                cwd: "/tmp".to_owned(),
            }],
            connected: true,
            ..crate::tmux::Topology::default()
        };
        let views = views_with_topology(
            &config,
            &State {
                version: 1,
                records,
            },
            &topology,
        );
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].session, "attached|pipe");
        assert!(views[0].attached);
        assert_eq!(views[0].status, "attention");
    }

    #[test]
    fn projection_at_100_sessions_200_panes_200_records_is_measured() {
        use std::time::Instant;
        let config = Config {
            state_dir: PathBuf::new(),
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        };
        let sessions = (0..100)
            .map(|index| crate::tmux::TmuxSession {
                id: format!("${index}"),
                name: format!("session-{index}"),
                last_attached: index,
                attached: false,
            })
            .collect::<Vec<_>>();
        let panes = (0..200)
            .map(|index| crate::tmux::Pane {
                session: format!("session-{}", index % 100),
                pane: format!("%{index}"),
                command: "codex".to_owned(),
                ..crate::tmux::Pane::default()
            })
            .collect::<Vec<_>>();
        let records = (0..200)
            .map(|index| {
                (
                    format!("codex:{index}"),
                    Record {
                        agent: "codex".to_owned(),
                        tmux_session: format!("session-{}", index % 100),
                        tmux_pane: format!("%{index}"),
                        updated_at: now(),
                        ..Record::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let started = Instant::now();
        let views = views_from(
            &config,
            &State {
                version: 1,
                records,
            },
            sessions,
            panes,
        );
        let elapsed = started.elapsed();
        eprintln!("projection 100 sessions / 200 panes / 200 records: {elapsed:?}");
        assert_eq!(views.len(), 100);
    }

    #[test]
    fn offline_session_does_not_target_a_stale_agent_pane() {
        let config = Config {
            state_dir: PathBuf::new(),
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        };
        let mut records = BTreeMap::new();
        records.insert(
            "codex:shell:%gone".to_owned(),
            Record {
                agent: "codex".to_owned(),
                tmux_session: "shell".to_owned(),
                tmux_pane: "%gone".to_owned(),
                status: "done".to_owned(),
                updated_at: now(),
                ..Record::default()
            },
        );
        let topology = crate::tmux::Topology {
            sessions: vec![crate::tmux::TmuxSession {
                id: "$1".to_owned(),
                name: "shell".to_owned(),
                last_attached: 42,
                attached: false,
            }],
            panes: vec![crate::tmux::Pane {
                session: "shell".to_owned(),
                pane: "%live".to_owned(),
                command: "zsh".to_owned(),
                cwd: "/tmp".to_owned(),
                ..crate::tmux::Pane::default()
            }],
            connected: true,
            ..crate::tmux::Topology::default()
        };

        let views = views_with_topology(
            &config,
            &State {
                version: 1,
                records,
            },
            &topology,
        );

        assert_eq!(views[0].status, "offline");
        assert_eq!(views[0].live_agent_count, 0);
        assert_eq!(views[0].pane, "");
    }
}
