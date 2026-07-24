use crate::config::Config;
use crate::model::SessionView;
use crate::sessions;
use crate::state;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

pub fn rows(config: &Config) -> Result<String, String> {
    let views = crate::daemon::cached_views(config)
        .or_else(|_| state::load(config).map(|state| sessions::views(config, &state)))?;
    Ok(render_rows(&views, config.use_color))
}

pub fn run(config: Config) -> Result<(), String> {
    let (updates, initial) = updates(config.clone());
    let mut app = App::new(initial);
    let mut terminal = ratatui::init();
    let outcome = loop {
        apply_updates(&mut app, &updates);
        terminal
            .draw(|frame| draw(frame, &mut app))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(40)).map_err(|error| error.to_string())?
            && let Event::Key(key) = event::read().map_err(|error| error.to_string())?
            && key.kind == KeyEventKind::Press
            && let Some(outcome) = app.key(key.code, key.modifiers)
        {
            break outcome;
        }
    };
    ratatui::restore();
    if let Some(session) = outcome {
        if !session.pane.is_empty() {
            let _ = Command::new("tmux")
                .args(["select-pane", "-t", &session.pane])
                .status();
        }
        if !session.session.is_empty() {
            let _ = Command::new("tmux")
                .args(["switch-client", "-t", &session.session])
                .status();
        }
    }
    Ok(())
}

fn apply_updates(app: &mut App, updates: &Receiver<Vec<SessionView>>) {
    while let Ok(next) = updates.try_recv() {
        app.replace(next);
    }
}

fn updates(config: Config) -> (Receiver<Vec<SessionView>>, Vec<SessionView>) {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        // The initial fallback and all daemon subscriptions run off the UI task,
        // so slow tmux recovery cannot delay input or rendering.
        if let Ok(state) = state::load(&config)
            && sender.send(sessions::views(&config, &state)).is_err()
        {
            return;
        }
        for _ in 0..20 {
            if let Ok(subscriber) = crate::daemon::subscribe(&config) {
                for views in subscriber {
                    if sender.send(views).is_err() {
                        return;
                    }
                }
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
    (receiver, Vec::new())
}

struct App {
    sessions: Vec<SessionView>,
    query: String,
    selected: Option<String>,
    list: ListState,
}

impl App {
    fn new(sessions: Vec<SessionView>) -> Self {
        let selected = sessions.first().map(|session| session.session.clone());
        let mut list = ListState::default();
        if selected.is_some() {
            list.select(Some(0));
        }
        Self {
            sessions,
            query: String::new(),
            selected,
            list,
        }
    }
    fn filtered(&self) -> Vec<&SessionView> {
        self.sessions
            .iter()
            .filter(|session| fuzzy(&session.session, &self.query))
            .collect()
    }
    fn replace(&mut self, sessions: Vec<SessionView>) {
        self.sessions = sessions;
        let filtered = self.filtered();
        let index = self
            .selected
            .as_ref()
            .and_then(|id| filtered.iter().position(|session| session.session == *id))
            .unwrap_or(0);
        self.selected = filtered.get(index).map(|session| session.session.clone());
        self.list.select(self.selected.as_ref().map(|_| index));
    }
    fn reset_for_query(&mut self) {
        let filtered = self.filtered();
        self.selected = filtered.first().map(|session| session.session.clone());
        self.list.select(self.selected.as_ref().map(|_| 0));
    }
    fn move_selection(&mut self, delta: isize) {
        let filtered = self.filtered();
        if filtered.is_empty() {
            self.selected = None;
            self.list.select(None);
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|id| filtered.iter().position(|session| session.session == *id))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, filtered.len() as isize - 1) as usize;
        self.selected = Some(filtered[next].session.clone());
        self.list.select(Some(next));
    }
    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Option<SessionView>> {
        match code {
            KeyCode::Esc => Some(None),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(None),
            KeyCode::Enter => Some(
                self.selected
                    .as_ref()
                    .and_then(|id| self.sessions.iter().find(|session| &session.session == id))
                    .cloned(),
            ),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Char('r') if modifiers.contains(KeyModifiers::CONTROL) => {
                let _ = Command::new("tmux").args(["refresh-client", "-S"]).status();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.reset_for_query();
                None
            }
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(character);
                self.reset_for_query();
                None
            }
            _ => None,
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let [list_area, detail_area, query_area, help_area] = Layout::vertical([
        Constraint::Min(4),
        Constraint::Length(5),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let items: Vec<_> = app
        .filtered()
        .iter()
        .map(|session| ListItem::new(row_line(session)))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("amux sessions"),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, list_area, &mut app.list);
    let detail = app
        .selected
        .as_ref()
        .and_then(|id| app.sessions.iter().find(|session| &session.session == id))
        .map(detail_text)
        .unwrap_or_else(|| "No matching sessions".to_owned());
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title("agents"))
            .wrap(Wrap { trim: true }),
        detail_area,
    );
    frame.render_widget(
        Paragraph::new(format!("{}█", app.query)).block(
            Block::default()
                .borders(Borders::ALL)
                .title("search session"),
        ),
        query_area,
    );
    frame.render_widget(
        Paragraph::new("↑/↓ navigate · Enter switch · Esc quit · type to filter · Ctrl-R refresh"),
        help_area,
    );
}

fn row_line(session: &&SessionView) -> Line<'static> {
    let (icon, color) = match session.status.as_str() {
        "attention" => ("▲", Color::Red),
        "running" => ("◐", Color::Yellow),
        "done" => ("●", Color::Green),
        "offline" => ("○", Color::DarkGray),
        _ => ("·", Color::Gray),
    };
    Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::raw(format!(
            "{:<22} {:>2} {:>5} {:<8} {}",
            session.session,
            session.agent_count,
            age(session.last_attached),
            session.pane,
            clean_reason(session),
        )),
    ])
}

fn detail_text(session: &SessionView) -> String {
    session
        .agents
        .iter()
        .map(|agent| {
            format!(
                "{}  {:<9} {:<10} {}",
                agent.pane, agent.agent, agent.status, agent.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fuzzy(value: &str, query: &str) -> bool {
    let value = value.to_ascii_lowercase();
    let mut chars = value.chars();
    query
        .to_ascii_lowercase()
        .chars()
        .all(|needle| chars.by_ref().any(|candidate| candidate == needle))
}

fn render_rows(sessions: &[SessionView], color: bool) -> String {
    sessions
        .iter()
        .map(|session| {
            let icon = match session.status.as_str() {
                "attention" => "▲",
                "running" => "◐",
                "done" => "●",
                "offline" => "○",
                _ => "·",
            };
            let styled = if color {
                match session.status.as_str() {
                    "attention" => format!("\x1b[31;1m{icon}\x1b[0m"),
                    "running" => format!("\x1b[33m{icon}\x1b[0m"),
                    "done" => format!("\x1b[32m{icon}\x1b[0m"),
                    "offline" => format!("\x1b[38;5;244m{icon}\x1b[0m"),
                    _ => icon.to_owned(),
                }
            } else {
                icon.to_owned()
            };
            format!(
                "{}\t{}\t{}\t{} {:<34} {:>5}  {}",
                session.session,
                session.pane,
                session.cwd,
                styled,
                session.session.chars().take(34).collect::<String>(),
                age(session.last_attached),
                clean_reason(session)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn age(timestamp: i64) -> String {
    let delta = (crate::event::now() - timestamp).max(0);
    if delta < 60 {
        format!("{delta}s")
    } else if delta < 3_600 {
        format!("{}m", delta / 60)
    } else if delta < 86_400 {
        format!("{}h", delta / 3_600)
    } else {
        format!("{}d", delta / 86_400)
    }
}
fn clean_reason(session: &SessionView) -> String {
    if session.reason == session.session {
        String::new()
    } else {
        session.reason.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> SessionView {
        SessionView {
            session: name.to_owned(),
            last_attached: 0,
            attached: false,
            status: "running".to_owned(),
            attention: false,
            agent_count: 1,
            live_agent_count: 1,
            agents: Vec::new(),
            pane: String::new(),
            reason: String::new(),
            cwd: String::new(),
            updated_at: 0,
        }
    }

    #[test]
    fn fuzzy_is_session_name_only() {
        assert!(fuzzy("alpha-session", "aps"));
        assert!(!fuzzy("alpha-session", "az"));
    }

    #[test]
    fn query_change_selects_the_first_match() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        app.move_selection(1);
        assert_eq!(app.selected.as_deref(), Some("beta"));
        app.key(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(app.selected.as_deref(), Some("alpha"));
    }

    #[test]
    fn passive_update_preserves_a_valid_selection() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        app.move_selection(1);
        app.replace(vec![session("alpha"), session("beta"), session("gamma")]);
        assert_eq!(app.selected.as_deref(), Some("beta"));
    }

    #[test]
    fn delayed_refresh_does_not_block_key_handling_or_reset_selection() {
        use std::time::Instant;

        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            sender
                .send(vec![session("alpha"), session("beta"), session("gamma")])
                .unwrap();
        });
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        let started = Instant::now();
        apply_updates(&mut app, &receiver);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(app.selected.as_deref(), Some("beta"));
        worker.join().unwrap();
        apply_updates(&mut app, &receiver);
        assert_eq!(app.selected.as_deref(), Some("beta"));
    }
}
