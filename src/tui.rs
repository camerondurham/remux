use crate::attach;
use crate::config::Config;
use crate::snapshot::{
    self, HostSnapshot, MatchStatus, PaneDetail, SessionSnapshot, SessionState, SnapshotError,
    SnapshotStatus,
};
use crate::tmux::PaneTarget;
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
    inspected_for: Option<String>,
    inspected_detail: Option<PaneDetail>,
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

struct DashboardSummary {
    host_ok: usize,
    host_total: usize,
    host_unreachable: usize,
    sessions: usize,
    matched: usize,
    active: usize,
    quiet: usize,
    idle: usize,
    missing: usize,
    errors: usize,
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
            inspected_for: None,
            inspected_detail: None,
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

    fn summary(&self) -> DashboardSummary {
        let (host_ok, host_total, host_unreachable) = if self.host_progress.is_empty() {
            (
                self.snapshots
                    .iter()
                    .filter(|snapshot| snapshot.status == SnapshotStatus::Ok)
                    .count(),
                self.snapshots.len(),
                self.snapshots
                    .iter()
                    .filter(|snapshot| snapshot.status == SnapshotStatus::Unreachable)
                    .count(),
            )
        } else {
            (
                self.host_progress
                    .iter()
                    .filter(|host| host.state == HostProgressState::Ok)
                    .count(),
                self.host_progress.len(),
                self.host_progress
                    .iter()
                    .filter(|host| host.state == HostProgressState::Unreachable)
                    .count(),
            )
        };

        let sessions: Vec<&SessionSnapshot> = self
            .snapshots
            .iter()
            .flat_map(|snapshot| snapshot.sessions.iter())
            .collect();
        let count_state = |state| {
            sessions
                .iter()
                .filter(|session| session.state == state)
                .count()
        };
        let matched = sessions
            .iter()
            .filter(|session| session.match_status == MatchStatus::Matched)
            .count();
        let errors = self
            .snapshots
            .iter()
            .map(|snapshot| {
                snapshot.errors.len()
                    + snapshot
                        .sessions
                        .iter()
                        .map(|session| session.errors.len())
                        .sum::<usize>()
            })
            .sum();

        DashboardSummary {
            host_ok,
            host_total,
            host_unreachable,
            sessions: sessions.len(),
            matched,
            active: count_state(SessionState::Active),
            quiet: count_state(SessionState::Quiet),
            idle: count_state(SessionState::Idle),
            missing: count_state(SessionState::Missing),
            errors,
        }
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
        KeyCode::Char('i') => {
            inspect_selected(config, app)?;
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
    if app.selected_row().is_none() {
        app.status = "no selected row".to_string();
        return Ok(());
    }
    let action = if readonly { "read-only attach" } else { "jump" };
    let result = suspend_terminal(terminal, || -> Result<PaneTarget> {
        let target = selected_attach_target(config, app, action)?;
        attach::attach_target(config, &target, readonly)
            .with_context(|| format!("failed to {action} `{target}`"))?;
        Ok(target)
    });
    match result {
        Ok(target) => app.status = format!("{action}ed {target}"),
        Err(err) => app.status = format!("{err:#}"),
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

fn inspect_selected(config: &Config, app: &mut App) -> Result<()> {
    let Some(row) = app.selected_row() else {
        app.status = "no selected row".to_string();
        return Ok(());
    };
    let display_id = row.display_id.clone();
    let Some(id) = row.watch_id.clone().or_else(|| row.raw_target.clone()) else {
        app.status = "selected row has no inspect target".to_string();
        return Ok(());
    };

    match snapshot::inspect(config, &id) {
        Ok(detail) => {
            app.inspected_for = Some(display_id);
            app.inspected_detail = Some(detail);
            app.status = format!("inspected {id}");
        }
        Err(err) => app.status = format!("{err:#}"),
    }
    Ok(())
}

fn selected_attach_target(config: &Config, app: &App, action: &str) -> Result<PaneTarget> {
    let row = app
        .selected_row()
        .ok_or_else(|| anyhow!("no selected row"))?;
    if let Some(reason) = attach_refusal_reason(row) {
        bail!("selected row cannot attach: {reason}");
    }
    if let Some(watch_id) = &row.watch_id {
        return snapshot::target_for_action(config, watch_id, action);
    }

    let raw_target = row
        .raw_target
        .as_ref()
        .ok_or_else(|| anyhow!("selected row has no live pane"))?;
    PaneTarget::parse(raw_target)
        .with_context(|| format!("selected row has invalid pane target `{raw_target}`"))
}

fn attach_refusal_reason(row: &SessionSnapshot) -> Option<String> {
    match row.match_status {
        MatchStatus::Matched | MatchStatus::Orphan => None,
        MatchStatus::Missing => Some("missing live pane".to_string()),
        MatchStatus::Ambiguous => Some("ambiguous candidates".to_string()),
        MatchStatus::Shadowed => Some(format!(
            "shadowed by {}",
            row.shadowed_by.as_deref().unwrap_or("another watch")
        )),
        MatchStatus::Unreachable => Some("host unreachable".to_string()),
        MatchStatus::Unknown => Some("unknown attach state".to_string()),
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(if app.help { 12 } else { 14 }),
            Constraint::Length(1),
        ])
        .split(area);

    draw_summary(frame, chunks[0], app);
    draw_sessions(frame, chunks[1], app);
    draw_detail(frame, chunks[2], app);
    draw_status(frame, chunks[3], app);
}

fn draw_summary(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let summary = app.summary();
    let mut spans = vec![
        Span::styled("hosts ", label_style()),
        Span::styled(
            format!("{}/{} ok", summary.host_ok, summary.host_total),
            if summary.host_unreachable > 0 {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD)
            },
        ),
        Span::raw(" | "),
        Span::styled("panes ", label_style()),
        Span::styled(
            summary.sessions.to_string(),
            Style::default().fg(Color::White),
        ),
        Span::raw(" | "),
        Span::styled("matched ", label_style()),
        Span::styled(
            summary.matched.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled("active ", label_style()),
        Span::styled(
            summary.active.to_string(),
            state_style(SessionState::Active),
        ),
        Span::raw(" | "),
        Span::styled("quiet ", label_style()),
        Span::styled(summary.quiet.to_string(), state_style(SessionState::Quiet)),
        Span::raw(" | "),
        Span::styled("idle ", label_style()),
        Span::styled(summary.idle.to_string(), state_style(SessionState::Idle)),
        Span::raw(" | "),
        Span::styled("filter: ", label_style()),
        Span::styled(filter_label(app), Style::default().fg(Color::White)),
    ];
    if summary.missing > 0 {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled("missing ", label_style()));
        spans.push(Span::styled(
            summary.missing.to_string(),
            state_style(SessionState::Missing),
        ));
    }
    if summary.host_unreachable > 0 {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled("unreachable ", label_style()));
        spans.push(Span::styled(
            summary.host_unreachable.to_string(),
            state_style(SessionState::Unreachable),
        ));
    }
    if summary.errors > 0 {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled("errors ", label_style()));
        spans.push(Span::styled(
            summary.errors.to_string(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title("remux alpha"));
    frame.render_widget(paragraph, area);
}

fn draw_sessions(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let rows = app.rows();
    if rows.is_empty() {
        let paragraph = Paragraph::new(Text::from(empty_state_lines(app)))
            .block(Block::default().borders(Borders::ALL).title("sessions"))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
        return;
    }

    let table_rows = rows.iter().map(|session| {
        let repo = session
            .repo
            .as_ref()
            .map(|repo| basename(&repo.path).to_string())
            .unwrap_or_else(|| "-".to_string());
        let dirty = session
            .repo
            .as_ref()
            .and_then(|repo| repo.dirty_count)
            .map(|dirty| dirty.to_string())
            .unwrap_or_else(|| "-".to_string());
        let preview = session
            .output
            .as_ref()
            .map(|output| short_message(&output.preview))
            .unwrap_or_else(|| first_row_error(session).unwrap_or_else(|| "-".to_string()));
        Row::new(vec![
            Cell::from(session.host.clone()),
            Cell::from(session.display_id.clone()),
            Cell::from(session.match_status.as_str()).style(match_style(session.match_status)),
            Cell::from(session.state.as_str()).style(state_style(session.state)),
            Cell::from(
                session
                    .process
                    .as_ref()
                    .map(|process| process.command.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(repo),
            Cell::from(dirty.clone()).style(dirty_style(dirty.as_str())),
            Cell::from(preview),
        ])
        .style(row_style(session))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(10),
            Constraint::Min(16),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(7),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new([
            "HOST", "ID", "MATCH", "STATE", "CMD", "REPO", "DIRTY", "PREVIEW",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("sessions"))
    .row_highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    if !rows.is_empty() {
        state.select(Some(app.selected.min(rows.len() - 1)));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_detail(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(if app.help {
        "keys"
    } else {
        "pane preview"
    });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.help {
        let text = Text::from(vec![
            Line::from("j/down select next"),
            Line::from("k/up select previous"),
            Line::from("r refresh now"),
            Line::from("/ filter"),
            Line::from("enter readonly attach"),
            Line::from("a read-write jump"),
            Line::from("c capture into detail"),
            Line::from("i inspect and refresh detail"),
            Line::from("q quit"),
        ]);
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
        return;
    }

    let Some(selected) = app.selected_row() else {
        frame.render_widget(
            Paragraph::new(Text::from(empty_state_lines(app))).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };

    let row = inspected_row(app, selected);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(43), Constraint::Percentage(57)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Text::from(detail_meta_lines(row))).wrap(Wrap { trim: false }),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Text::from(detail_preview_lines(app, selected, row)))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn draw_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let mode = if app.editing_filter {
        "filter"
    } else if app.polling {
        "polling"
    } else {
        "ready"
    };
    let mut spans = vec![
        key_span("enter"),
        Span::raw(" readonly | "),
        key_span("a"),
        Span::raw(" jump | "),
        key_span("r"),
        Span::raw(" refresh | "),
        key_span("/"),
        Span::raw(" filter | "),
        key_span("c"),
        Span::raw(" capture | "),
        key_span("i"),
        Span::raw(" inspect | "),
        key_span("q"),
        Span::raw(" quit"),
        Span::raw("   "),
        Span::styled(mode, Style::default().fg(Color::Cyan)),
        Span::raw(format!(" | {} | filter: {}", app.status, app.filter)),
    ];
    if app.editing_filter {
        spans.push(Span::styled(" _", Style::default().fg(Color::LightCyan)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn inspected_row<'a>(app: &'a App, selected: &'a SessionSnapshot) -> &'a SessionSnapshot {
    if app.inspected_for.as_deref() == Some(selected.display_id.as_str())
        && let Some(detail) = &app.inspected_detail
    {
        return &detail.session;
    }
    selected
}

fn detail_meta_lines(row: &SessionSnapshot) -> Vec<Line<'static>> {
    let pid = row
        .process
        .as_ref()
        .and_then(|process| process.pid)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    let command = row
        .process
        .as_ref()
        .map(|process| process.command.as_str())
        .unwrap_or("-");
    let cwd = row
        .process
        .as_ref()
        .map(|process| process.cwd.as_str())
        .unwrap_or("-");
    let repo_path = row
        .repo
        .as_ref()
        .map(|repo| repo.path.as_str())
        .unwrap_or("-");
    let branch = row
        .repo
        .as_ref()
        .and_then(|repo| repo.branch.as_deref())
        .unwrap_or("-");
    let dirty = row
        .repo
        .as_ref()
        .and_then(|repo| repo.dirty_count)
        .map(|dirty| dirty.to_string())
        .unwrap_or_else(|| "-".to_string());
    let watch = row.watch_id.as_deref().unwrap_or("-");
    let raw_target = row.raw_target.as_deref().unwrap_or("-");

    let mut lines = vec![
        Line::from(Span::styled(
            row.display_id.clone(),
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("activity: ", label_style()),
            Span::styled(row.state.as_str(), state_style(row.state)),
            Span::raw(" | "),
            Span::styled("match: ", label_style()),
            Span::styled(row.match_status.as_str(), match_style(row.match_status)),
            Span::raw(" | "),
            Span::styled("watch: ", label_style()),
            Span::raw(watch.to_string()),
        ]),
        Line::from(vec![
            Span::styled("target: ", label_style()),
            Span::raw(raw_target.to_string()),
        ]),
        Line::from(vec![
            Span::styled("tmux: ", label_style()),
            Span::raw(tmux_target(row)),
            Span::raw(" | "),
            Span::styled("pane id: ", label_style()),
            Span::raw(row.tmux.pane_id.as_deref().unwrap_or("-").to_string()),
            Span::raw(" | "),
            Span::styled("pid: ", label_style()),
            Span::raw(pid.clone()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("command: ", label_style()),
            Span::raw(command.to_string()),
        ]),
        Line::from(vec![
            Span::styled("cwd: ", label_style()),
            Span::raw(cwd.to_string()),
        ]),
        Line::from(vec![
            Span::styled("repo: ", label_style()),
            Span::raw(repo_path.to_string()),
        ]),
        Line::from(vec![
            Span::styled("branch: ", label_style()),
            Span::raw(branch.to_string()),
            Span::raw(" | "),
            Span::styled("dirty: ", label_style()),
            Span::styled(dirty.clone(), dirty_style(dirty.as_str())),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("attach: ", label_style()),
            Span::raw(attach_hint(row)),
        ]),
        Line::from(vec![
            Span::styled("capture: ", label_style()),
            Span::raw(capture_hint(row)),
        ]),
    ];
    if let Some(agent) = &row.agent_hint {
        lines.push(Line::from(vec![
            Span::styled("hint: ", label_style()),
            Span::raw(agent.clone()),
        ]));
    }
    lines
}

fn detail_preview_lines(
    app: &App,
    selected: &SessionSnapshot,
    row: &SessionSnapshot,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Recent output preview",
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    if let Some(output) = selected_output(app, selected, row).filter(|output| !output.is_empty()) {
        for line in tail_lines(output, 8) {
            lines.push(Line::from(line));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No recent output in cache. Capture or inspect the selected pane to refresh this view.",
            muted_style(),
        )));
    }

    if let Some(repo) = &row.repo {
        if !repo.changed_files.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Changed files ({})", repo.changed_files.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for file in repo.changed_files.iter().take(6) {
                lines.push(Line::from(file.clone()));
            }
            if repo.changed_files.len() > 6 {
                lines.push(Line::from(Span::styled("...", muted_style())));
            }
        }
        if let Some(error) = &repo.error {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Repo error: ", Style::default().fg(Color::Red)),
                Span::raw(short_message(error)),
            ]));
        }
    }

    if !row.errors.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Errors",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        for error in &row.errors {
            lines.push(Line::from(format!(
                "{}: {}",
                error.kind,
                short_message(&error.message)
            )));
        }
    }

    if !row.candidate_targets.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Candidates",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for target in &row.candidate_targets {
            lines.push(Line::from(target.clone()));
        }
    }

    if let Some(shadowed_by) = &row.shadowed_by {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Shadowed by: ", label_style()),
            Span::raw(shadowed_by.clone()),
        ]));
    }

    lines
}

fn selected_output<'a>(
    app: &'a App,
    selected: &'a SessionSnapshot,
    row: &'a SessionSnapshot,
) -> Option<&'a str> {
    if app.captured_for.as_deref() == Some(selected.display_id.as_str()) {
        return app.captured_output.as_deref();
    }
    if app.inspected_for.as_deref() == Some(selected.display_id.as_str())
        && let Some(detail) = &app.inspected_detail
        && !detail.recent_output.is_empty()
    {
        return Some(detail.recent_output.as_str());
    }
    row.output
        .as_ref()
        .map(|output| {
            if output.recent.is_empty() {
                output.preview.as_str()
            } else {
                output.recent.as_str()
            }
        })
        .filter(|output| !output.is_empty())
}

fn empty_state_lines(app: &App) -> Vec<Line<'static>> {
    if app.snapshots.is_empty() && app.polling {
        return vec![
            Line::from(Span::styled(
                "Polling hosts",
                Style::default().fg(Color::Cyan),
            )),
            Line::from(app.progress_summary()),
        ];
    }

    if !app.filter.trim().is_empty() {
        return vec![
            Line::from(format!("No rows match filter `{}`", app.filter.trim())),
            Line::from(host_problem_summary(app)),
        ];
    }

    let mut lines = vec![Line::from("No live tmux panes discovered")];
    let problems = unreachable_host_messages(app);
    if !problems.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Unreachable hosts",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        for problem in problems {
            lines.push(Line::from(problem));
        }
    } else if !app.snapshots.is_empty() {
        lines.push(Line::from("All reachable hosts returned zero panes"));
    }
    lines
}

fn host_problem_summary(app: &App) -> String {
    let problems = unreachable_host_messages(app);
    if problems.is_empty() {
        "No hidden host errors".to_string()
    } else {
        format!("Host problems: {}", problems.join(" | "))
    }
}

fn unreachable_host_messages(app: &App) -> Vec<String> {
    let mut messages = Vec::new();
    for snapshot in &app.snapshots {
        if snapshot.status == SnapshotStatus::Unreachable {
            let message = snapshot
                .errors
                .first()
                .map(|error| short_message(&error.message))
                .unwrap_or_else(|| "unreachable".to_string());
            messages.push(format!("{}: {message}", snapshot.host));
        }
    }
    for progress in &app.host_progress {
        if progress.state == HostProgressState::Unreachable
            && !messages
                .iter()
                .any(|message| message.starts_with(&format!("{}:", progress.id)))
        {
            let message = progress
                .message
                .as_deref()
                .map(short_message)
                .unwrap_or_else(|| "unreachable".to_string());
            messages.push(format!("{}: {message}", progress.id));
        }
    }
    messages
}

fn tail_lines(output: &str, max_lines: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
}

fn tmux_target(row: &SessionSnapshot) -> String {
    row.tmux
        .window
        .as_ref()
        .zip(row.tmux.pane.as_ref())
        .map(|(window, pane)| format!("{}:{}.{}", row.tmux.session, window, pane))
        .unwrap_or_else(|| row.tmux.session.clone())
}

fn attach_hint(row: &SessionSnapshot) -> String {
    if let Some(reason) = attach_refusal_reason(row) {
        return format!("unavailable, {reason}");
    }

    if row.watch_id.is_some() {
        format!("enter readonly | a jump -> remux attach {}", row.display_id)
    } else if let Some(target) = &row.raw_target {
        format!("enter readonly | a jump -> remux attach '{target}'")
    } else {
        "unavailable".to_string()
    }
}

fn capture_hint(row: &SessionSnapshot) -> String {
    match row.match_status {
        MatchStatus::Matched => format!("c -> remux capture {} --lines <n>", row.display_id),
        MatchStatus::Orphan => row
            .raw_target
            .as_ref()
            .map(|target| format!("c -> remux capture '{target}' --lines <n>"))
            .unwrap_or_else(|| "unavailable".to_string()),
        MatchStatus::Missing => "unavailable, missing live pane".to_string(),
        MatchStatus::Ambiguous => "unavailable, ambiguous watch".to_string(),
        MatchStatus::Shadowed => format!(
            "unavailable, shadowed by {}",
            row.shadowed_by.as_deref().unwrap_or("another watch")
        ),
        MatchStatus::Unreachable => "unavailable, host unreachable".to_string(),
        MatchStatus::Unknown => "unavailable".to_string(),
    }
}

fn filter_label(app: &App) -> String {
    let trimmed = app.filter.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn label_style() -> Style {
    Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::BOLD)
}

fn muted_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn key_span(key: &'static str) -> Span<'static> {
    Span::styled(
        key,
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn row_search_text(row: &SessionSnapshot) -> String {
    let repo_text = row
        .repo
        .as_ref()
        .map(|repo| {
            format!(
                "{} {}",
                repo.path,
                repo.changed_files
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .unwrap_or_default();
    let output_text = row
        .output
        .as_ref()
        .map(|output| format!("{} {}", output.preview, output.recent))
        .unwrap_or_default();
    format!(
        "{} {} {} {} {} {} {} {}",
        row.host,
        row.display_id,
        row.match_status.as_str(),
        row.state.as_str(),
        row.process
            .as_ref()
            .map(|process| process.command.as_str())
            .unwrap_or(""),
        row.process
            .as_ref()
            .map(|process| process.cwd.as_str())
            .unwrap_or(""),
        repo_text,
        output_text
    )
}

fn row_style(session: &SessionSnapshot) -> Style {
    if matches!(
        session.match_status,
        MatchStatus::Unreachable | MatchStatus::Missing
    ) {
        return match_style(session.match_status);
    }
    match session.state {
        SessionState::Idle => muted_style(),
        SessionState::Quiet => Style::default().fg(Color::Gray),
        SessionState::Missing | SessionState::Unreachable => state_style(session.state),
        SessionState::Active | SessionState::Unknown => Style::default(),
    }
}

fn match_style(status: MatchStatus) -> Style {
    match status {
        MatchStatus::Matched => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        MatchStatus::Orphan => Style::default().fg(Color::White),
        MatchStatus::Missing => Style::default().fg(Color::Red),
        MatchStatus::Ambiguous => Style::default().fg(Color::Magenta),
        MatchStatus::Shadowed => Style::default().fg(Color::Blue),
        MatchStatus::Unreachable => Style::default().fg(Color::Red),
        MatchStatus::Unknown => Style::default().fg(Color::Gray),
    }
}

fn state_style(state: SessionState) -> Style {
    match state {
        SessionState::Active => Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
        SessionState::Quiet => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::DIM),
        SessionState::Idle => Style::default().fg(Color::DarkGray),
        SessionState::Missing => Style::default().fg(Color::Yellow),
        SessionState::Unreachable => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        SessionState::Unknown => Style::default().fg(Color::Gray),
    }
}

fn dirty_style(value: &str) -> Style {
    match value {
        "-" | "0" => Style::default().fg(Color::DarkGray),
        _ => Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
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

fn first_row_error(session: &SessionSnapshot) -> Option<String> {
    session
        .errors
        .first()
        .map(|error| short_message(&error.message))
}
