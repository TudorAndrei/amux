use crate::config::Config;
use crate::model::{Record, SessionView};

pub fn list(records: &[Record]) -> String {
    records
        .iter()
        .map(|record| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                record.agent,
                record.status,
                dash(&record.tmux_session),
                dash(&record.tmux_pane),
                dash(&record.reason),
                dash(&record.cwd)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn sessions(sessions: &[SessionView]) -> String {
    sessions
        .iter()
        .map(|session| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                session.status,
                session.session,
                dash(&session.pane),
                dash(&session.reason),
                dash(&session.cwd)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

pub fn status(config: &Config, sessions: &[SessionView]) -> String {
    let agents: Vec<_> = sessions
        .iter()
        .flat_map(|session| &session.agents)
        .collect();
    let (kind, count, icon, style) = if agents
        .iter()
        .filter(|agent| agent.status == "attention")
        .count()
        > 0
    {
        let count = agents
            .iter()
            .filter(|agent| agent.status == "attention")
            .count();
        ("attention", count, "▲", "fg=red,bold")
    } else if agents
        .iter()
        .filter(|agent| agent.status == "running")
        .count()
        > 0
    {
        let count = agents
            .iter()
            .filter(|agent| agent.status == "running")
            .count();
        ("running", count, "◐", "fg=yellow")
    } else if agents.iter().filter(|agent| agent.status == "done").count() > 0 {
        let count = agents.iter().filter(|agent| agent.status == "done").count();
        ("done", count, "●", "fg=green")
    } else if agents
        .iter()
        .filter(|agent| agent.status == "offline")
        .count()
        > 0
    {
        let count = agents
            .iter()
            .filter(|agent| agent.status == "offline")
            .count();
        ("offline", count, "○", "fg=colour244")
    } else {
        return String::new();
    };
    let _ = kind;
    if config.use_color {
        format!("#[{style}]{icon}#[default] {count}")
    } else {
        format!("{icon} {count}")
    }
}
