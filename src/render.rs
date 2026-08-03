use crate::model::{HistoryEvent, Record, SessionView};

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

pub fn events(events: &[HistoryEvent]) -> String {
    events
        .iter()
        .map(|event| {
            let record = &event.record;
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                dash(&sanitize(&record.updated_at_iso)),
                sanitize(&record.agent),
                sanitize(&record.status),
                dash(&sanitize(&record.tmux_session)),
                dash(&sanitize(&record.tmux_pane)),
                dash(&sanitize(&record.last_event)),
                dash(&sanitize(&record.reason)),
                dash(&sanitize(&record.cwd))
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

#[cfg(test)]
mod tests {
    use super::{events, sanitize};
    use crate::model::{HistoryEvent, Record};

    #[test]
    fn sanitize_makes_controls_visible_and_preserves_unicode() {
        assert_eq!(sanitize("wait\x1b[2J\n"), "wait^[[2J^J");
        assert_eq!(sanitize("東京 e\u{301} 🦀"), "東京 e\u{301} 🦀");
    }

    #[test]
    fn event_rows_sanitize_every_string_column() {
        let control = "\u{1b}X";
        let event = HistoryEvent {
            key: "ignored".to_owned(),
            record: Record {
                updated_at_iso: format!("time{control}"),
                agent: format!("agent{control}"),
                status: format!("status{control}"),
                tmux_session: format!("session{control}"),
                tmux_pane: format!("pane{control}"),
                last_event: format!("event{control}"),
                reason: format!("reason{control}"),
                cwd: format!("cwd{control}"),
                ..Record::default()
            },
        };
        assert_eq!(
            events(&[event]),
            "time^[X\tagent^[X\tstatus^[X\tsession^[X\tpane^[X\tevent^[X\treason^[X\tcwd^[X"
        );
    }
}
