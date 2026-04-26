use crate::attach;
use crate::config::Config;
use crate::snapshot::{self, HostSnapshot, MatchStatus, SessionSnapshot, SnapshotError};
use crate::tmux::PaneTarget;
use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use std::collections::VecDeque;
use std::io::{self, Stdout};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

pub fn run(config: &Config, host: Option<String>, filter: Option<String>) -> Result<()> {
    if let Some(host_id) = &host {
        config.host(host_id)?;
    }

    let mut terminal = enter_terminal()?;
    let result = run_app(
        config.clone(),
        host,
        filter.unwrap_or_default(),
        &mut terminal,
    );
    leave_terminal(&mut terminal)?;
    result
}

fn run_app(
    config: Config,
    host: Option<String>,
    filter: String,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(filter);
    spawn_refresh(&config, host.clone(), tx.clone(), &mut app)?;

    loop {
        while let Ok(message) = rx.try_recv() {
            app.apply_refresh(message);
        }

        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && handle_key(key, &config, host.clone(), tx.clone(), &mut app, terminal)?
        {
            break;
        }
    }

    Ok(())
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to initialize terminal")
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")
}

fn suspend_terminal<T>(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    action: impl FnOnce() -> Result<T>,
) -> Result<T> {
    leave_terminal(terminal)?;
    let result = action();
    *terminal = enter_terminal()?;
    result
}

struct App {
    snapshots: Vec<HostSnapshot>,
    selected: usize,
    filter: String,
    editing_filter: bool,
    help: bool,
    status: String,
    polling: bool,
    host_progress: Vec<HostProgress>,
    refresh_started_at: Option<Instant>,
    captured_for: Option<String>,
    captured_output: Option<String>,
    last_refresh: Option<Instant>,
}

#[derive(Clone)]
struct HostProgress {
    id: String,
    state: HostProgressState,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    message: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostProgressState {
    Queued,
    Polling,
    Ok,
    Unreachable,
}

impl App {
    fn new(filter: String) -> Self {
        Self {
            snapshots: Vec::new(),
            selected: 0,
            filter,
            editing_filter: false,
            help: false,
            status: "polling".to_string(),
            polling: false,
            host_progress: Vec::new(),
            refresh_started_at: None,
            captured_for: None,
            captured_output: None,
            last_refresh: None,
        }
    }

    fn rows(&self) -> Vec<&SessionSnapshot> {
        let filter = self.filter.trim().to_lowercase();
        self.snapshots
            .iter()
            .flat_map(|snapshot| snapshot.sessions.iter())
            .filter(|session| {
                filter.is_empty()
                    || row_search_text(session)
                        .to_lowercase()
                        .contains(filter.as_str())
            })
            .collect()
    }

    fn selected_row(&self) -> Option<&SessionSnapshot> {
        let rows = self.rows();
        rows.get(self.selected).copied()
    }

    fn clamp_selection(&mut self) {
        let len = self.rows().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    fn begin_refresh(&mut self, host_ids: Vec<String>) {
        self.polling = true;
        self.refresh_started_at = Some(Instant::now());
        self.host_progress = host_ids
            .into_iter()
            .map(|id| HostProgress {
                id,
                state: HostProgressState::Queued,
                started_at: None,
                finished_at: None,
                message: None,
            })
            .collect();
        self.status = self.progress_summary();
    }

    fn apply_refresh(&mut self, message: RefreshMessage) {
        match message {
            RefreshMessage::HostStarted { host } => {
                if let Some(progress) = self.host_progress.iter_mut().find(|item| item.id == host) {
                    progress.state = HostProgressState::Polling;
                    progress.started_at = Some(Instant::now());
                    progress.message = None;
                }
                self.status = self.progress_summary();
            }
            RefreshMessage::HostFinished { snapshot } => {
                self.update_host_finished(&snapshot);
                self.upsert_snapshot(snapshot);
                self.clamp_selection();
                self.status = self.progress_summary();
            }
            RefreshMessage::Complete => {
                self.polling = false;
                self.last_refresh = Some(Instant::now());
                self.status = format!("scan complete: {}", self.progress_summary());
            }
        }
    }

    fn update_host_finished(&mut self, snapshot: &HostSnapshot) {
        if let Some(progress) = self
            .host_progress
            .iter_mut()
            .find(|item| item.id == snapshot.host)
        {
            progress.state = match snapshot.status {
                snapshot::SnapshotStatus::Ok => HostProgressState::Ok,
                snapshot::SnapshotStatus::Unreachable => HostProgressState::Unreachable,
            };
            progress.finished_at = Some(Instant::now());
            progress.message = snapshot.errors.first().map(|error| error.message.clone());
        }
    }

    fn upsert_snapshot(&mut self, snapshot: HostSnapshot) {
        if let Some(existing) = self
            .snapshots
            .iter_mut()
            .find(|existing| existing.host == snapshot.host)
        {
            *existing = snapshot;
        } else {
            self.snapshots.push(snapshot);
        }

        let order: Vec<String> = self
            .host_progress
            .iter()
            .map(|progress| progress.id.clone())
            .collect();
        self.snapshots.sort_by_key(|snapshot| {
            order
                .iter()
                .position(|host| host == &snapshot.host)
                .unwrap_or(usize::MAX)
        });
    }

    fn progress_summary(&self) -> String {
        if self.host_progress.is_empty() {
            return "no hosts queued".to_string();
        }

        let total = self.host_progress.len();
        let finished = self
            .host_progress
            .iter()
            .filter(|host| host.state.is_finished())
            .count();
        let queued = self
            .host_progress
            .iter()
            .filter(|host| host.state == HostProgressState::Queued)
            .count();
        let polling: Vec<String> = self
            .host_progress
            .iter()
            .filter(|host| host.state == HostProgressState::Polling)
            .map(|host| {
                let elapsed = host
                    .started_at
                    .map(|started| format_elapsed(started.elapsed()))
                    .unwrap_or_else(|| "0s".to_string());
                format!("{} {elapsed}", host.id)
            })
            .collect();

        let mut parts = vec![format!("{finished}/{total} hosts")];
        if !polling.is_empty() {
            parts.push(format!("polling {}", polling.join(", ")));
        }
        if queued > 0 {
            parts.push(format!("{queued} queued"));
        }
        if let Some(started) = self.refresh_started_at {
            parts.push(format!("elapsed {}", format_elapsed(started.elapsed())));
        }
        parts.join(" | ")
    }
}

impl HostProgressState {
    fn is_finished(self) -> bool {
        matches!(self, HostProgressState::Ok | HostProgressState::Unreachable)
    }
}

enum RefreshMessage {
    HostStarted { host: String },
    HostFinished { snapshot: HostSnapshot },
    Complete,
}

fn spawn_refresh(
    config: &Config,
    host: Option<String>,
    tx: mpsc::Sender<RefreshMessage>,
    app: &mut App,
) -> Result<()> {
    if app.polling {
        return Ok(());
    }
    let host_ids = host_ids_for_refresh(config, host.as_deref())?;
    app.begin_refresh(host_ids.clone());
    let config = config.clone();
    thread::spawn(move || {
        poll_hosts(&config, host_ids, tx);
    });
    Ok(())
}

fn host_ids_for_refresh(config: &Config, host: Option<&str>) -> Result<Vec<String>> {
    Ok(match host {
        Some(host) => vec![config.host(host)?.id.clone()],
        None => config.hosts.iter().map(|host| host.id.clone()).collect(),
    })
}

fn poll_hosts(config: &Config, host_ids: Vec<String>, tx: mpsc::Sender<RefreshMessage>) {
    if host_ids.is_empty() {
        let _ = tx.send(RefreshMessage::Complete);
        return;
    }

    let worker_count = config.poll.max_concurrency.min(host_ids.len()).max(1);
    let queue = Arc::new(Mutex::new(VecDeque::from(host_ids)));
    let mut handles = Vec::new();

    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let config = config.clone();
        handles.push(thread::spawn(move || {
            loop {
                let host_id = {
                    let mut queue = queue.lock().expect("poll queue poisoned");
                    queue.pop_front()
                };
                let Some(host_id) = host_id else {
                    break;
                };
                if tx
                    .send(RefreshMessage::HostStarted {
                        host: host_id.clone(),
                    })
                    .is_err()
                {
                    break;
                }
                let snapshot = snapshot::snapshot_host(&config, &host_id)
                    .unwrap_or_else(|err| synthetic_error_snapshot(host_id, err));
                if tx.send(RefreshMessage::HostFinished { snapshot }).is_err() {
                    break;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
    let _ = tx.send(RefreshMessage::Complete);
}

fn synthetic_error_snapshot(host: String, err: anyhow::Error) -> HostSnapshot {
    HostSnapshot {
        host,
        status: snapshot::SnapshotStatus::Unreachable,
        collected_at: Utc::now(),
        sessions: Vec::new(),
        errors: vec![SnapshotError {
            kind: "poll".to_string(),
            message: format!("{err:#}"),
        }],
    }
}

fn handle_key(
    key: KeyEvent,
    config: &Config,
    host: Option<String>,
    tx: mpsc::Sender<RefreshMessage>,
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    if app.editing_filter {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.editing_filter = false,
            KeyCode::Backspace => {
                app.filter.pop();
                app.clamp_selection();
            }
            KeyCode::Char(ch) => {
                app.filter.push(ch);
                app.clamp_selection();
            }
            _ => {}
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Ok(true),
        KeyCode::Char('/') => {
            app.editing_filter = true;
            Ok(false)
        }
        KeyCode::Char('?') => {
            app.help = !app.help;
            Ok(false)
        }
        KeyCode::Char('r') => {
            spawn_refresh(config, host, tx, app)?;
            Ok(false)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.rows().len();
            if app.selected + 1 < len {
                app.selected += 1;
            }
            Ok(false)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected = app.selected.saturating_sub(1);
            Ok(false)
        }
        KeyCode::Enter => {
            attach_selected(config, app, terminal, true)?;
            Ok(false)
        }
        KeyCode::Char('a') => {
            attach_selected(config, app, terminal, false)?;
            Ok(false)
        }
        KeyCode::Char('c') => {
            capture_selected(config, app)?;
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn attach_selected(
    config: &Config,
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    readonly: bool,
) -> Result<()> {
    let Some(target) = selected_attach_target(app) else {
        app.status = "selected row cannot attach".to_string();
        return Ok(());
    };
    let result = suspend_terminal(terminal, || {
        attach::attach_target(config, &target, readonly)
    });
    if let Err(err) = result {
        app.status = format!("{err:#}");
    } else {
        app.status = format!("attached to {target}");
    }
    Ok(())
}

fn capture_selected(config: &Config, app: &mut App) -> Result<()> {
    let Some(row) = app.selected_row() else {
        app.status = "no selected row".to_string();
        return Ok(());
    };
    let Some(raw_target) = &row.raw_target else {
        app.status = "selected row has no live pane".to_string();
        return Ok(());
    };
    let display_id = row.display_id.clone();
    let target = PaneTarget::parse(raw_target)?;
    match snapshot::capture_pane(config, &target, config.poll.capture_lines) {
        Ok(output) => {
            app.captured_for = Some(display_id);
            app.captured_output = Some(output);
            app.status = format!("captured {target}");
        }
        Err(err) => app.status = format!("{err:#}"),
    }
    Ok(())
}

fn selected_attach_target(app: &App) -> Option<PaneTarget> {
    let row = app.selected_row()?;
    if matches!(
        row.match_status,
        MatchStatus::Ambiguous
            | MatchStatus::Missing
            | MatchStatus::Shadowed
            | MatchStatus::Unreachable
            | MatchStatus::Unknown
    ) {
        return None;
    }
    PaneTarget::parse(row.raw_target.as_ref()?).ok()
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(7),
            Constraint::Length(if app.help { 13 } else { 10 }),
            Constraint::Length(1),
        ])
        .split(area);

    draw_hosts(frame, chunks[0], app);
    draw_sessions(frame, chunks[1], app);
    draw_detail(frame, chunks[2], app);
    draw_status(frame, chunks[3], app);
}

fn draw_hosts(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let mut spans = Vec::new();
    if !app.host_progress.is_empty() {
        for progress in &app.host_progress {
            spans.push(Span::styled(
                format!("{} {}", progress.id, progress_label(progress)),
                progress_style(progress.state),
            ));
            spans.push(Span::raw("   "));
        }
    } else {
        for snapshot in &app.snapshots {
            let status = match snapshot.status {
                snapshot::SnapshotStatus::Ok => "ok",
                snapshot::SnapshotStatus::Unreachable => "unreachable",
            };
            let style = if matches!(snapshot.status, snapshot::SnapshotStatus::Ok) {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            spans.push(Span::styled(format!("{} {}", snapshot.host, status), style));
            spans.push(Span::raw("   "));
        }
    }
    if spans.is_empty() {
        spans.push(Span::raw("no hosts"));
    }
    let paragraph = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .title("remux - hosts"),
    );
    frame.render_widget(paragraph, area);
}

fn draw_sessions(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let rows = app.rows();
    let table_rows = rows.iter().map(|session| {
        let repo = session
            .repo
            .as_ref()
            .map(|repo| basename(&repo.path).to_string())
            .unwrap_or_else(|| "-".to_string());
        let state = session.state.as_str();
        Row::new(vec![
            Cell::from(session.host.clone()),
            Cell::from(session.display_id.clone()),
            Cell::from(session.match_status.as_str()),
            Cell::from(
                session
                    .process
                    .as_ref()
                    .map(|process| process.command.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(repo),
            Cell::from(state),
        ])
        .style(status_style(session.match_status))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(12),
            Constraint::Min(18),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(["HOST", "ID", "MATCH", "CMD", "REPO", "STATE"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("sessions"))
    .row_highlight_style(Style::default().bg(Color::DarkGray));
    let mut state = TableState::default();
    if !rows.is_empty() {
        state.select(Some(app.selected.min(rows.len() - 1)));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_detail(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let text = if app.help {
        Text::from(vec![
            Line::from("j/down select next"),
            Line::from("k/up select previous"),
            Line::from("r refresh now"),
            Line::from("/ filter"),
            Line::from("enter attach read-only"),
            Line::from("a attach read-write"),
            Line::from("c capture into detail"),
            Line::from("q quit"),
        ])
    } else if let Some(row) = app.selected_row() {
        let output = if app.captured_for.as_deref() == Some(row.display_id.as_str()) {
            app.captured_output.as_deref()
        } else {
            row.output.as_ref().map(|output| output.preview.as_str())
        }
        .unwrap_or("");
        let mut lines = vec![
            Line::from(format!(
                "Target: {}  Pane: {}  PID: {}",
                row.raw_target.as_deref().unwrap_or("-"),
                row.tmux.pane_id.as_deref().unwrap_or("-"),
                row.process
                    .as_ref()
                    .and_then(|process| process.pid)
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_string())
            )),
            Line::from(format!(
                "Command: {}  CWD: {}",
                row.process
                    .as_ref()
                    .map(|process| process.command.as_str())
                    .unwrap_or("-"),
                row.process
                    .as_ref()
                    .map(|process| process.cwd.as_str())
                    .unwrap_or("-")
            )),
            Line::from(format!(
                "Repo: {}  Branch: {}  Dirty: {}",
                row.repo
                    .as_ref()
                    .map(|repo| repo.path.as_str())
                    .unwrap_or("-"),
                row.repo
                    .as_ref()
                    .and_then(|repo| repo.branch.as_deref())
                    .unwrap_or("-"),
                row.repo
                    .as_ref()
                    .and_then(|repo| repo.dirty_count)
                    .map(|dirty| dirty.to_string())
                    .unwrap_or_else(|| "-".to_string())
            )),
        ];
        if !row.candidate_targets.is_empty() {
            lines.push(Line::from(format!(
                "Candidates: {}",
                row.candidate_targets.join(", ")
            )));
        }
        if let Some(shadowed_by) = &row.shadowed_by {
            lines.push(Line::from(format!("Shadowed by: {shadowed_by}")));
        }
        lines.push(Line::from(""));
        lines.extend(output.lines().map(|line| Line::from(line.to_string())));
        Text::from(lines)
    } else {
        Text::from("no sessions")
    };
    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("detail"))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_status(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, app: &App) {
    let mode = if app.editing_filter {
        "filter"
    } else if app.polling {
        "polling"
    } else {
        "ready"
    };
    let line = format!("{mode} | {} | filter: {}", app.status, app.filter);
    frame.render_widget(Paragraph::new(line), area);
}

fn row_search_text(row: &SessionSnapshot) -> String {
    format!(
        "{} {} {} {} {} {}",
        row.host,
        row.display_id,
        row.match_status.as_str(),
        row.process
            .as_ref()
            .map(|process| process.command.as_str())
            .unwrap_or(""),
        row.process
            .as_ref()
            .map(|process| process.cwd.as_str())
            .unwrap_or(""),
        row.output
            .as_ref()
            .map(|output| output.preview.as_str())
            .unwrap_or("")
    )
}

fn status_style(status: MatchStatus) -> Style {
    match status {
        MatchStatus::Matched => Style::default().fg(Color::Green),
        MatchStatus::Orphan => Style::default().fg(Color::White),
        MatchStatus::Missing => Style::default().fg(Color::Yellow),
        MatchStatus::Ambiguous => Style::default().fg(Color::Magenta),
        MatchStatus::Shadowed => Style::default().fg(Color::Blue),
        MatchStatus::Unreachable => Style::default().fg(Color::Red),
        MatchStatus::Unknown => Style::default().fg(Color::Gray),
    }
}

fn progress_label(progress: &HostProgress) -> String {
    match progress.state {
        HostProgressState::Queued => "queued".to_string(),
        HostProgressState::Polling => {
            let elapsed = progress
                .started_at
                .map(|started| format_elapsed(started.elapsed()))
                .unwrap_or_else(|| "0s".to_string());
            format!("polling {elapsed}")
        }
        HostProgressState::Ok => {
            let elapsed = progress
                .started_at
                .zip(progress.finished_at)
                .map(|(started, finished)| format_elapsed(finished.duration_since(started)))
                .unwrap_or_else(|| "done".to_string());
            format!("ok {elapsed}")
        }
        HostProgressState::Unreachable => {
            let elapsed = progress
                .started_at
                .zip(progress.finished_at)
                .map(|(started, finished)| format_elapsed(finished.duration_since(started)))
                .unwrap_or_else(|| "done".to_string());
            if let Some(message) = &progress.message {
                format!("unreachable {elapsed}: {}", short_message(message))
            } else {
                format!("unreachable {elapsed}")
            }
        }
    }
}

fn progress_style(state: HostProgressState) -> Style {
    match state {
        HostProgressState::Queued => Style::default().fg(Color::DarkGray),
        HostProgressState::Polling => Style::default().fg(Color::Cyan),
        HostProgressState::Ok => Style::default().fg(Color::Green),
        HostProgressState::Unreachable => Style::default().fg(Color::Red),
    }
}

fn format_elapsed(duration: Duration) -> String {
    if duration.as_millis() < 1_000 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}s", duration.as_secs())
    }
}

fn short_message(message: &str) -> String {
    let first_line = message.lines().next().unwrap_or(message).trim();
    const LIMIT: usize = 48;
    if first_line.chars().count() <= LIMIT {
        first_line.to_string()
    } else {
        format!("{}...", first_line.chars().take(LIMIT).collect::<String>())
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
}
