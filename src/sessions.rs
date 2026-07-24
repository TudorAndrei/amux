use crate::config::Config;
use crate::event::now;
use crate::model::{AgentView, Record, SessionView, State};
use std::process::Command;

#[derive(Clone, Debug)]
struct TmuxSession {
    name: String,
    last_attached: i64,
    attached: bool,
}
#[derive(Clone, Debug)]
struct Pane {
    session: String,
    pane: String,
    command: String,
    title: String,
    cwd: String,
}

fn tmux_lines(args: &[&str]) -> String {
    Command::new("tmux")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn tmux_sessions() -> Vec<TmuxSession> {
    tmux_lines(&[
        "list-sessions",
        "-F",
        "#{session_last_attached}|#{session_name}|#{session_attached}",
    ])
    .lines()
    .filter_map(|line| {
        let parts: Vec<_> = line.split('|').collect();
        (parts.len() >= 3 && !parts[1].is_empty()).then(|| TmuxSession {
            name: parts[1].to_owned(),
            last_attached: parts[0].parse().unwrap_or(0),
            attached: parts[2] == "1",
        })
    })
    .collect()
}

fn panes() -> Vec<Pane> {
    tmux_lines(&["list-panes", "-a", "-F", "#{session_name}|#{pane_id}|#{pane_current_command}|#{pane_pid}|#{pane_title}|#{pane_current_path}"])
        .lines().filter_map(|line| { let parts: Vec<_> = line.split('|').collect(); (parts.len() >= 6).then(|| Pane { session: parts[0].to_owned(), pane: parts[1].to_owned(), command: parts[2].to_owned(), title: parts[4].to_owned(), cwd: parts[5].to_owned() }) }).collect()
}

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

fn as_agent(record: &Record, pane: Option<&Pane>, session: &str, live: bool) -> AgentView {
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
    views_from(config, state, tmux_sessions(), panes())
}

fn views_from(
    config: &Config,
    state: &State,
    tmux_sessions: Vec<TmuxSession>,
    panes: Vec<Pane>,
) -> Vec<SessionView> {
    let cutoff = now() - config.stale_seconds;
    let mut records: Vec<Record> = state
        .records
        .values()
        .filter(|record| {
            record.updated_at >= cutoff
                && !record.tmux_session.is_empty()
                && !subagent_record(record, config.hide_subagents)
        })
        .cloned()
        .collect();
    for record in &mut records {
        if record.last_event.eq_ignore_ascii_case("UserPromptSubmit") {
            record.status = "running".to_owned();
            record.attention = false;
        }
    }
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
            pane: target.map(|agent| agent.pane.clone()).unwrap_or_default(),
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
