use crate::model::{Record, SessionView};

pub fn list(records: &[Record]) -> String {
    records
        .iter()
        .map(|record| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                sanitize(&record.agent),
                sanitize(&record.status),
                dash(&sanitize(&record.tmux_session)),
                dash(&sanitize(&record.tmux_pane)),
                dash(&sanitize(&record.reason)),
                dash(&sanitize(&record.cwd))
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
                sanitize(&session.status),
                sanitize(&session.session),
                dash(&sanitize(&session.pane)),
                dash(&sanitize(&session.reason)),
                dash(&sanitize(&session.cwd))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// Make terminal control characters visible rather than letting input alter the terminal.
pub fn sanitize(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            '\x00'..='\x1f' => vec!['^', char::from_u32(c as u32 + 64).unwrap()],
            '\x7f' => vec!['^', '?'],
            '\u{80}'..='\u{9f}' => format!("^[[{}]", c as u32).chars().collect(),
            _ => vec![c],
        })
        .collect()
}
