use crate::config::Config;
use crate::model::SessionView;
use crate::sessions;
use crate::state;
use nucleo::Nucleo;
use nucleo::pattern::{CaseMatching, Normalization};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

pub fn rows(config: &Config) -> Result<String, String> {
    let views = crate::daemon::cached_views(config)
        .or_else(|_| state::load(config).map(|state| sessions::views(config, &state)))?;
    Ok(render_rows(&views, config.use_color))
}

pub fn run(config: Config) -> Result<(), String> {
    run_native(config)
}

fn run_native(config: Config) -> Result<(), String> {
    let (updates, initial) = updates(config.clone());
    let mut app = App::new(initial);
    let mut terminal = ratatui::init();
    let outcome = loop {
        apply_updates(&mut app, &updates);
        app.tick();
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
        crate::tmux::switch_client(&session.session, &session.pane)?;
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
    matcher: Nucleo<String>,
    select_first_match: bool,
}

impl App {
    fn new(sessions: Vec<SessionView>) -> Self {
        let mut app = Self {
            sessions,
            query: String::new(),
            selected: None,
            list: ListState::default(),
            matcher: Nucleo::new(nucleo::Config::DEFAULT, Arc::new(|| ()), Some(1), 1),
            select_first_match: true,
        };
        app.rebuild_matcher();
        app
    }

    fn rebuild_matcher(&mut self) {
        self.matcher.restart(true);
        let injector = self.matcher.injector();
        for session in &self.sessions {
            injector.push(session.session.clone(), |name, columns| {
                columns[0] = name.as_str().into();
            });
        }
    }

    fn filtered(&self) -> Vec<&SessionView> {
        self.matcher
            .snapshot()
            .matched_items(..)
            .filter_map(|item| {
                self.sessions
                    .iter()
                    .find(|session| session.session == *item.data)
            })
            .collect()
    }

    fn tick(&mut self) {
        let status = self.matcher.tick(0);
        if status.changed {
            self.sync_selection();
        }
    }

    fn sync_selection(&mut self) {
        let matched = self
            .filtered()
            .into_iter()
            .map(|session| session.session.clone())
            .collect::<Vec<_>>();
        if matched.is_empty() {
            self.selected = None;
            self.list.select(None);
            return;
        }
        let index = if self.select_first_match {
            self.select_first_match = false;
            0
        } else {
            self.selected
                .as_ref()
                .and_then(|id| matched.iter().position(|session| session == id))
                .unwrap_or(0)
        };
        self.selected = Some(matched[index].clone());
        self.list.select(Some(index));
    }

    fn replace(&mut self, sessions: Vec<SessionView>) {
        self.sessions = sessions;
        if self.selected.as_ref().is_some_and(|selected| {
            !self
                .sessions
                .iter()
                .any(|session| session.session == *selected)
        }) {
            self.selected = None;
            self.list.select(None);
        }
        self.rebuild_matcher();
    }

    fn reset_for_query(&mut self) {
        self.matcher.pattern.reparse(
            0,
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );
        self.selected = None;
        self.list.select(None);
        self.select_first_match = true;
    }

    fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.reset_for_query();
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
                let mut query = self.query.clone();
                query.pop();
                self.set_query(query);
                None
            }
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                let mut query = self.query.clone();
                query.push(character);
                self.set_query(query);
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
                .title("search session name"),
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

    fn settle(app: &mut App) {
        for _ in 0..50 {
            app.tick();
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn names(app: &App) -> Vec<&str> {
        app.filtered()
            .into_iter()
            .map(|session| session.session.as_str())
            .collect()
    }

    #[test]
    fn nucleo_ranks_exact_session_name_first() {
        let mut app = App::new(vec![session("fabulous-oof"), session("foo")]);
        app.set_query("foo");
        settle(&mut app);

        assert_eq!(names(&app).first(), Some(&"foo"));
        assert_eq!(app.selected.as_deref(), Some("foo"));
    }

    #[test]
    fn matcher_only_indexes_session_names() {
        let mut non_matching = session("alpha");
        non_matching.status = "notification".to_owned();
        non_matching.reason = "notification".to_owned();
        non_matching.cwd = "/notification".to_owned();
        non_matching.pane = "%notification".to_owned();
        let mut app = App::new(vec![non_matching, session("notifier")]);
        app.set_query("noti");
        settle(&mut app);

        assert_eq!(names(&app), vec!["notifier"]);
    }

    #[test]
    fn query_change_selects_the_first_match() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        settle(&mut app);
        app.move_selection(1);
        assert_eq!(app.selected.as_deref(), Some("beta"));
        app.key(KeyCode::Char('a'), KeyModifiers::NONE);
        settle(&mut app);
        assert_eq!(app.selected.as_deref(), Some("alpha"));
    }

    #[test]
    fn passive_update_preserves_a_valid_selection() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        settle(&mut app);
        app.move_selection(1);
        app.replace(vec![session("alpha"), session("beta"), session("gamma")]);
        settle(&mut app);
        assert_eq!(app.selected.as_deref(), Some("beta"));
    }

    #[test]
    fn removing_a_selected_session_never_keeps_a_stale_target() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        settle(&mut app);
        app.move_selection(1);
        assert_eq!(app.selected.as_deref(), Some("beta"));

        app.replace(vec![session("alpha")]);
        settle(&mut app);

        assert_eq!(app.selected.as_deref(), Some("alpha"));
        assert_eq!(
            app.key(KeyCode::Enter, KeyModifiers::NONE)
                .flatten()
                .map(|session| session.session),
            Some("alpha".to_owned())
        );
    }

    #[test]
    fn empty_and_no_match_snapshots_clear_selection() {
        let mut app = App::new(Vec::new());
        settle(&mut app);
        assert_eq!(names(&app), Vec::<&str>::new());
        assert_eq!(app.selected, None);

        app.replace(vec![session("alpha")]);
        app.set_query("missing");
        settle(&mut app);
        assert_eq!(names(&app), Vec::<&str>::new());
        assert_eq!(app.selected, None);
    }

    #[test]
    fn matching_handles_case_unicode_spaces_and_hyphens() {
        let sessions = vec![
            session("CamelCase"),
            session("東京-agent"),
            session("client work"),
            session("api-gateway"),
        ];

        let mut app = App::new(sessions.clone());
        app.set_query("camel");
        settle(&mut app);
        assert_eq!(names(&app), vec!["CamelCase"]);

        let mut app = App::new(sessions.clone());
        app.set_query("東京");
        settle(&mut app);
        assert_eq!(names(&app), vec!["東京-agent"]);

        let mut app = App::new(sessions.clone());
        app.set_query("client work");
        settle(&mut app);
        assert_eq!(names(&app), vec!["client work"]);

        let mut app = App::new(sessions);
        app.set_query("api-gateway");
        settle(&mut app);
        assert_eq!(names(&app), vec!["api-gateway"]);
    }

    #[test]
    fn backspace_reparses_a_non_empty_query() {
        let mut app = App::new(vec![session("alpha"), session("beta")]);
        app.key(KeyCode::Char('b'), KeyModifiers::NONE);
        settle(&mut app);
        assert_eq!(names(&app), vec!["beta"]);

        app.key(KeyCode::Backspace, KeyModifiers::NONE);
        settle(&mut app);
        assert_eq!(names(&app), vec!["alpha", "beta"]);
        assert_eq!(app.selected.as_deref(), Some("alpha"));
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
        settle(&mut app);
        let started = Instant::now();
        apply_updates(&mut app, &receiver);
        app.key(KeyCode::Down, KeyModifiers::NONE);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(app.selected.as_deref(), Some("beta"));
        worker.join().unwrap();
        apply_updates(&mut app, &receiver);
        settle(&mut app);
        assert_eq!(app.selected.as_deref(), Some("beta"));
    }
}
