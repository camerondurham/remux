use crate::attach;
use crate::config::Config;
use crate::lifecycle;
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
use std::cmp::Ordering;
use std::collections::BTreeMap;
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
    selected_identity: Option<String>,
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
    kill_prompt: Option<KillPrompt>,
    input_prompt: Option<InputPrompt>,
    action_feedback: Option<ActionFeedback>,
    pending_confirmations: Vec<PendingConfirmation>,
    last_refresh: Option<Instant>,
    sort_mode: SortMode,
}

#[derive(Clone, Copy)]
enum SortMode {
    Attention,
    LastOutput,
    State,
    Id,
}

struct KillPrompt {
    target: String,
}

struct InputPrompt {
    kind: InputPromptKind,
    value: String,
}

#[derive(Clone)]
struct ActionFeedback {
    level: FeedbackLevel,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeedbackLevel {
    Info,
    Success,
    Error,
}

struct PendingConfirmation {
    kind: PendingConfirmationKind,
    host: String,
    session: String,
}

enum PendingConfirmationKind {
    CreatedSession,
}

#[derive(Clone)]
enum InputPromptKind {
    RenameSession { host: String, current: String },
    NewSession,
    NewPane,
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
    active: usize,
    quiet: usize,
    idle: usize,
    missing: usize,
    ambiguous: usize,
    shadowed: usize,
    errors: usize,
    attention: usize,
}

impl App {
    fn new(filter: String) -> Self {
        Self {
            snapshots: Vec::new(),
            selected: 0,
            selected_identity: None,
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
            kill_prompt: None,
            input_prompt: None,
            action_feedback: None,
            pending_confirmations: Vec::new(),
            last_refresh: None,
            sort_mode: SortMode::Attention,
        }
    }

    fn rows(&self) -> Vec<&SessionSnapshot> {
        let filter = self.filter.trim().to_lowercase();
        let mut rows: Vec<&SessionSnapshot> = self
            .snapshots
            .iter()
            .flat_map(|snapshot| snapshot.sessions.iter())
            .filter(|session| {
                filter.is_empty()
                    || row_search_text(session)
                        .to_lowercase()
                        .contains(filter.as_str())
            })
            .collect();
        rows.sort_by(|left, right| compare_rows(left, right, self.sort_mode));
        rows
    }

    fn selected_row(&self) -> Option<&SessionSnapshot> {
        let rows = self.rows();
        rows.get(self.selected).copied()
    }

    fn selected_row_identity(&self) -> Option<String> {
        self.selected_row().map(row_identity)
    }

    fn restore_selection(&mut self, preferred_identity: Option<String>) {
        let preferred_identity = preferred_identity.or_else(|| self.selected_identity.clone());
        let (selected, selected_identity) = {
            let rows = self.rows();
            if rows.is_empty() {
                self.selected = 0;
                self.selected_identity = None;
                return;
            }
            let selected = if let Some(identity) = preferred_identity {
                rows.iter()
                    .position(|row| row_identity(row) == identity)
                    .unwrap_or_else(|| self.selected.min(rows.len() - 1))
            } else {
                self.selected.min(rows.len() - 1)
            };
            (selected, rows.get(selected).map(|row| row_identity(row)))
        };
        if selected_identity.is_none() {
            self.selected = 0;
            self.selected_identity = None;
            return;
        }
        self.selected = selected;
        self.selected_identity = selected_identity;
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
                let selected_identity = self.selected_row_identity();
                self.update_host_finished(&snapshot);
                self.confirm_pending_for_snapshot(&snapshot);
                self.upsert_snapshot(snapshot);
                self.restore_selection(selected_identity);
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
        let ambiguous = sessions
            .iter()
            .filter(|session| session.match_status == MatchStatus::Ambiguous)
            .count();
        let shadowed = sessions
            .iter()
            .filter(|session| session.match_status == MatchStatus::Shadowed)
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
        let missing = count_state(SessionState::Missing);
        let attention = host_unreachable + missing + ambiguous + errors;

        DashboardSummary {
            host_ok,
            host_total,
            host_unreachable,
            sessions: sessions.len(),
            active: count_state(SessionState::Active),
            quiet: count_state(SessionState::Quiet),
            idle: count_state(SessionState::Idle),
            missing,
            ambiguous,
            shadowed,
            errors,
            attention,
        }
    }

    fn cycle_sort_mode(&mut self) {
        let selected_identity = self.selected_row_identity();
        self.sort_mode = match self.sort_mode {
            SortMode::Attention => SortMode::LastOutput,
            SortMode::LastOutput => SortMode::State,
            SortMode::State => SortMode::Id,
            SortMode::Id => SortMode::Attention,
        };
        self.restore_selection(selected_identity);
    }

    fn set_feedback(&mut self, level: FeedbackLevel, message: impl Into<String>) {
        self.action_feedback = Some(ActionFeedback {
            level,
            message: compact_status_message(&message.into()),
        });
    }

    fn add_session_confirmation(&mut self, host: impl Into<String>, session: impl Into<String>) {
        self.pending_confirmations.push(PendingConfirmation {
            kind: PendingConfirmationKind::CreatedSession,
            host: host.into(),
            session: session.into(),
        });
    }

    fn confirm_pending_for_snapshot(&mut self, snapshot: &HostSnapshot) {
        if self.pending_confirmations.is_empty() {
            return;
        }

        let mut remaining = Vec::new();
        let pending = std::mem::take(&mut self.pending_confirmations);
        for confirmation in pending {
            if confirmation.host != snapshot.host {
                remaining.push(confirmation);
                continue;
            }

            match confirmation.kind {
                PendingConfirmationKind::CreatedSession => {
                    let target = format!("{}/{}", confirmation.host, confirmation.session);
                    if snapshot.status == SnapshotStatus::Unreachable {
                        let reason = snapshot
                            .errors
                            .first()
                            .map(|error| error.message.as_str())
                            .unwrap_or("host unreachable during confirmation scan");
                        self.set_feedback(
                            FeedbackLevel::Error,
                            format!(
                                "created session {target}, but refresh could not confirm: {reason}"
                            ),
                        );
                        continue;
                    }

                    let visible = snapshot.sessions.iter().any(|row| {
                        row.raw_target.is_some()
                            && row.tmux.session == confirmation.session
                            && !matches!(
                                row.match_status,
                                MatchStatus::Missing | MatchStatus::Unreachable
                            )
                    });
                    if visible {
                        self.set_feedback(
                            FeedbackLevel::Success,
                            format!("created session {target}; confirmed in latest scan"),
                        );
                    } else {
                        self.set_feedback(
                            FeedbackLevel::Error,
                            format!(
                                "created session {target}, but it was not visible in latest scan"
                            ),
                        );
                    }
                }
            }
        }
        self.pending_confirmations = remaining;
    }
}

impl HostProgressState {
    fn is_finished(self) -> bool {
        matches!(self, HostProgressState::Ok | HostProgressState::Unreachable)
    }
}

impl FeedbackLevel {
    fn label(self) -> &'static str {
        match self {
            FeedbackLevel::Info => "info",
            FeedbackLevel::Success => "ok",
            FeedbackLevel::Error => "error",
        }
    }
}

impl SortMode {
    fn label(self) -> &'static str {
        match self {
            SortMode::Attention => "attention",
            SortMode::LastOutput => "last-output",
            SortMode::State => "state",
            SortMode::Id => "id",
        }
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

    if app.kill_prompt.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let prompt = app.kill_prompt.take().expect("prompt exists");
                match lifecycle::kill(config, &prompt.target, true, false) {
                    Ok(()) => {
                        app.set_feedback(
                            FeedbackLevel::Success,
                            format!("killed {}; refreshing to confirm", prompt.target),
                        );
                        spawn_refresh(config, host, tx, app)?;
                    }
                    Err(err) => {
                        app.set_feedback(FeedbackLevel::Error, format!("kill failed: {err:#}"));
                    }
                }
            }
            _ => {
                app.kill_prompt = None;
                app.set_feedback(FeedbackLevel::Info, "kill cancelled");
            }
        }
        return Ok(false);
    }

    if app.input_prompt.is_some() {
        handle_input_prompt_key(key, config, host, tx, app)?;
        return Ok(false);
    }

    if app.editing_filter {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.editing_filter = false,
            KeyCode::Backspace => {
                let selected_identity = app.selected_row_identity();
                app.filter.pop();
                app.restore_selection(selected_identity);
            }
            KeyCode::Char(ch) => {
                let selected_identity = app.selected_row_identity();
                app.filter.push(ch);
                app.restore_selection(selected_identity);
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
        KeyCode::Char('s') => {
            app.cycle_sort_mode();
            app.status = format!("sort: {}", app.sort_mode.label());
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
            app.selected_identity = app.selected_row_identity();
            Ok(false)
        }
        KeyCode::Up => {
            app.selected = app.selected.saturating_sub(1);
            app.selected_identity = app.selected_row_identity();
            Ok(false)
        }
        KeyCode::Char('k') => {
            begin_kill_prompt(config, app)?;
            Ok(false)
        }
        KeyCode::Char('e') => {
            begin_rename_prompt(app);
            Ok(false)
        }
        KeyCode::Char('n') => {
            app.input_prompt = Some(InputPrompt {
                kind: InputPromptKind::NewSession,
                value: String::new(),
            });
            app.status = "new session: enter <host>/<session>".to_string();
            Ok(false)
        }
        KeyCode::Char('p') => {
            app.input_prompt = Some(InputPrompt {
                kind: InputPromptKind::NewPane,
                value: String::new(),
            });
            app.status = "new pane: enter <host>/<session>".to_string();
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

fn begin_kill_prompt(config: &Config, app: &mut App) -> Result<()> {
    let Some(row) = app.selected_row() else {
        app.status = "no selected row".to_string();
        return Ok(());
    };
    if let Some(reason) = attach_refusal_reason(row) {
        app.status = format!("selected row cannot be killed: {reason}");
        return Ok(());
    }

    let target = if let Some(watch_id) = &row.watch_id {
        match snapshot::target_for_action(config, watch_id, "kill") {
            Ok(target) => target.to_string(),
            Err(err) => {
                app.status = format!("{err:#}");
                return Ok(());
            }
        }
    } else if let Some(raw_target) = &row.raw_target {
        raw_target.clone()
    } else {
        app.status = "selected row has no live kill target".to_string();
        return Ok(());
    };

    app.kill_prompt = Some(KillPrompt { target });
    Ok(())
}

fn begin_rename_prompt(app: &mut App) {
    let Some(row) = app.selected_row() else {
        app.status = "no selected row".to_string();
        return;
    };
    app.input_prompt = Some(InputPrompt {
        kind: InputPromptKind::RenameSession {
            host: row.host.clone(),
            current: row.tmux.session.clone(),
        },
        value: row.tmux.session.clone(),
    });
    app.status = "rename session: edit name and press enter".to_string();
}

fn handle_input_prompt_key(
    key: KeyEvent,
    config: &Config,
    host: Option<String>,
    tx: mpsc::Sender<RefreshMessage>,
    app: &mut App,
) -> Result<()> {
    let Some(prompt) = app.input_prompt.as_mut() else {
        return Ok(());
    };

    match key.code {
        KeyCode::Esc => {
            app.input_prompt = None;
            app.set_feedback(FeedbackLevel::Info, "action cancelled");
        }
        KeyCode::Backspace => {
            prompt.value.pop();
        }
        KeyCode::Enter => {
            execute_input_prompt(config, host, tx, app)?;
        }
        KeyCode::Char(ch) => {
            prompt.value.push(ch);
        }
        _ => {}
    }
    Ok(())
}

fn execute_input_prompt(
    config: &Config,
    scoped_host: Option<String>,
    tx: mpsc::Sender<RefreshMessage>,
    app: &mut App,
) -> Result<()> {
    let Some(prompt) = app.input_prompt.take() else {
        return Ok(());
    };

    match prompt.kind {
        InputPromptKind::RenameSession {
            host: host_id,
            current,
        } => {
            let new_name = prompt.value.trim();
            if new_name.is_empty() {
                app.set_feedback(FeedbackLevel::Info, "rename cancelled: empty name");
                return Ok(());
            }
            if new_name == current {
                app.set_feedback(FeedbackLevel::Info, "rename skipped: unchanged");
                return Ok(());
            }
            match lifecycle::rename_session(config, &host_id, &current, new_name, false) {
                Ok(()) => {
                    app.set_feedback(
                        FeedbackLevel::Success,
                        format!("renamed {host_id}/{current} -> {new_name}; refreshing to confirm"),
                    );
                    spawn_refresh(config, Some(host_id), tx, app)?;
                }
                Err(err) => {
                    app.set_feedback(FeedbackLevel::Error, format!("rename failed: {err:#}"));
                }
            }
        }
        InputPromptKind::NewSession => {
            let Some((host_id, session_name)) = parse_host_session(prompt.value.trim()) else {
                app.set_feedback(FeedbackLevel::Error, "new session expects <host>/<session>");
                return Ok(());
            };
            if let Some(scope) = scoped_host.as_deref()
                && scope != host_id
            {
                app.set_feedback(
                    FeedbackLevel::Error,
                    format!(
                    "new session blocked in scoped view: expected host `{scope}`, got `{host_id}`"
                    ),
                );
                return Ok(());
            }
            match lifecycle::new_session(config, host_id, session_name, None, None, false) {
                Ok(()) => {
                    app.set_feedback(
                        FeedbackLevel::Success,
                        format!("created session {host_id}/{session_name}; refreshing to confirm"),
                    );
                    app.add_session_confirmation(host_id, session_name);
                    spawn_refresh(config, Some(host_id.to_string()), tx, app)?;
                }
                Err(err) => {
                    app.set_feedback(FeedbackLevel::Error, format!("new session failed: {err:#}"));
                }
            }
        }
        InputPromptKind::NewPane => {
            let Some((host_id, session_name)) = parse_host_session(prompt.value.trim()) else {
                app.set_feedback(FeedbackLevel::Error, "new pane expects <host>/<session>");
                return Ok(());
            };
            if let Some(scope) = scoped_host.as_deref()
                && scope != host_id
            {
                app.set_feedback(
                    FeedbackLevel::Error,
                    format!(
                        "new pane blocked in scoped view: expected host `{scope}`, got `{host_id}`"
                    ),
                );
                return Ok(());
            }
            match lifecycle::new_pane(config, host_id, session_name, false) {
                Ok(()) => {
                    app.set_feedback(
                        FeedbackLevel::Success,
                        format!("spawned pane in {host_id}/{session_name}; refreshing to confirm"),
                    );
                    spawn_refresh(config, Some(host_id.to_string()), tx, app)?;
                }
                Err(err) => {
                    app.set_feedback(FeedbackLevel::Error, format!("new pane failed: {err:#}"));
                }
            }
        }
    }
    Ok(())
}

fn parse_host_session(input: &str) -> Option<(&str, &str)> {
    let (host, session) = input.split_once('/')?;
    if host.is_empty() || session.is_empty() || session.contains(':') {
        return None;
    }
    Some((host, session))
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
    match snapshot::capture_pane(config, &target, config.poll.capture_lines, false) {
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
            Constraint::Length(4),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    draw_summary(frame, chunks[0], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(24),
            Constraint::Percentage(42),
            Constraint::Percentage(34),
        ])
        .split(chunks[1]);
    draw_topology_tree(frame, body[0], app);
    draw_live_table(frame, body[1], app);
    draw_inspector(frame, body[2], app);
    draw_status(frame, chunks[2], app);
}

fn draw_summary(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let summary = app.summary();
    let top = Line::from(vec![
        Span::styled("hosts ", label_style()),
        Span::styled(
            format!("{}/{} up", summary.host_ok, summary.host_total),
            if summary.host_unreachable > 0 {
                state_style(SessionState::Unreachable)
            } else {
                state_style(SessionState::Active)
            },
        ),
        Span::raw(" | "),
        Span::styled("panes ", label_style()),
        Span::raw(summary.sessions.to_string()),
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
        Span::styled("missing ", label_style()),
        Span::styled(
            summary.missing.to_string(),
            state_style(SessionState::Missing),
        ),
        Span::raw(" | "),
        Span::styled("ambiguous ", label_style()),
        Span::styled(
            summary.ambiguous.to_string(),
            match_style(MatchStatus::Ambiguous),
        ),
        Span::raw(" | "),
        Span::styled("shadowed ", label_style()),
        Span::styled(
            summary.shadowed.to_string(),
            match_style(MatchStatus::Shadowed),
        ),
    ]);

    let bottom = Line::from(vec![
        Span::styled("attention ", label_style()),
        Span::styled(
            summary.attention.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled("errors ", label_style()),
        Span::styled(
            summary.errors.to_string(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled("sort ", label_style()),
        Span::raw(app.sort_mode.label()),
        Span::raw(" | "),
        Span::styled("filter ", label_style()),
        Span::raw(filter_label(app)),
    ]);
    let paragraph = Paragraph::new(Text::from(vec![top, bottom]))
        .block(Block::default().borders(Borders::ALL).title("kpi"));
    frame.render_widget(paragraph, area);
}

fn draw_live_table(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let rows = app.rows();
    if rows.is_empty() {
        let paragraph = Paragraph::new(Text::from(empty_state_lines(app)))
            .block(Block::default().borders(Borders::ALL).title("live table"))
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
            Cell::from(session.display_id.clone()),
            Cell::from(table_target(session)),
            Cell::from(session.match_status.as_str()).style(match_style(session.match_status)),
            Cell::from(session.state.as_str()).style(state_style(session.state)),
            Cell::from(last_output_age(session)),
            Cell::from(
                session
                    .process
                    .as_ref()
                    .map(|process| process.command.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(repo),
            Cell::from(dirty.clone()).style(dirty_style(dirty.as_str())),
            Cell::from(preview).style(muted_style()),
        ])
        .style(row_style(session))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Min(16),
            Constraint::Min(18),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Min(16),
        ],
    )
    .header(
        Row::new([
            "ID", "TARGET", "MATCH", "STATE", "LAST", "CMD", "REPO", "DIRTY", "PREVIEW",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("live table"))
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

fn draw_topology_tree(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let rows = app.rows();
    let selected = rows.get(app.selected).copied();
    let mut hosts: BTreeMap<String, Vec<&SessionSnapshot>> = BTreeMap::new();
    for row in rows {
        hosts.entry(row.host.clone()).or_default().push(row);
    }

    let mut lines = Vec::new();
    if hosts.is_empty() {
        lines.push(Line::from("No topology rows"));
    } else {
        for (host, host_rows) in hosts {
            let status = app
                .snapshots
                .iter()
                .find(|snapshot| snapshot.host == host)
                .map(|snapshot| snapshot.status)
                .unwrap_or(SnapshotStatus::Ok);
            lines.push(Line::from(vec![
                Span::styled("▸ ", muted_style()),
                Span::styled(host.clone(), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(
                    match status {
                        SnapshotStatus::Ok => "(ok)",
                        SnapshotStatus::Unreachable => "(unreachable)",
                    },
                    if status == SnapshotStatus::Ok {
                        state_style(SessionState::Active)
                    } else {
                        state_style(SessionState::Unreachable)
                    },
                ),
            ]));

            let mut sessions: BTreeMap<String, Vec<&SessionSnapshot>> = BTreeMap::new();
            for row in host_rows {
                sessions
                    .entry(row.tmux.session.clone())
                    .or_default()
                    .push(row);
            }
            for (session, session_rows) in sessions {
                lines.push(Line::from(format!("  ▸ session {session}")));
                for row in session_rows {
                    let marker = if selected.map(|item| item.display_id.as_str())
                        == Some(row.display_id.as_str())
                    {
                        ">"
                    } else {
                        " "
                    };
                    let pane = row
                        .tmux
                        .window
                        .as_ref()
                        .zip(row.tmux.pane.as_ref())
                        .map(|(window, pane)| format!("{window}.{pane}"))
                        .unwrap_or_else(|| "-".to_string());
                    let command = row
                        .process
                        .as_ref()
                        .map(|process| process.command.as_str())
                        .unwrap_or("-");
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {marker} "), muted_style()),
                        Span::styled(pane, muted_style()),
                        Span::raw(" "),
                        Span::styled(command.to_string(), Style::default().fg(Color::White)),
                        Span::raw(" "),
                        Span::styled(row.state.as_str(), state_style(row.state)),
                    ]));
                }
            }
        }
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("topology"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_inspector(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let block =
        Block::default()
            .borders(Borders::ALL)
            .title(if app.help { "keys" } else { "inspector" });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.help {
        let text = Text::from(vec![
            Line::from("j/down select next"),
            Line::from("up select previous"),
            Line::from("r refresh now"),
            Line::from("s cycle table sort"),
            Line::from("/ filter"),
            Line::from("enter readonly attach"),
            Line::from("a read-write jump"),
            Line::from("c capture into detail"),
            Line::from("i inspect and refresh detail"),
            Line::from("k kill selected pane"),
            Line::from("e rename selected session"),
            Line::from("n create session (<host>/<session>)"),
            Line::from("p spawn pane (<host>/<session>)"),
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

    // Right column: status notes pinned on top (fixed size based on content),
    // preview fills the rest. Without this split a long preview would push
    // status notes off-screen.
    let right = chunks[1];
    let status_lines = detail_status_lines(row);
    let status_height = (status_lines.len() as u16).min(right.height.saturating_sub(3));
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(status_height), Constraint::Min(1)])
        .split(right);

    if status_height > 0 {
        frame.render_widget(
            Paragraph::new(Text::from(status_lines)).wrap(Wrap { trim: false }),
            right_chunks[0],
        );
    }

    let preview_area = right_chunks[1];
    // Header + trailing tail lines sized to actually fit the preview area.
    let preview_body_rows = preview_area.height.saturating_sub(1) as usize;
    frame.render_widget(
        Paragraph::new(Text::from(detail_preview_lines(
            app,
            selected,
            row,
            preview_body_rows,
        )))
        .wrap(Wrap { trim: false }),
        preview_area,
    );
}

fn draw_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if let Some(prompt) = &app.kill_prompt {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(vec![
                    Span::styled("confirm ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("kill {}? (y/N)", prompt.target)),
                ]),
                footer_key_line(),
            ])),
            area,
        );
        return;
    }
    if let Some(prompt) = &app.input_prompt {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                input_prompt_line(prompt),
                footer_key_line(),
            ])),
            area,
        );
        return;
    }

    let mode = if app.editing_filter {
        "filter"
    } else if app.polling {
        "polling"
    } else {
        "ready"
    };
    let mut status_spans = vec![
        Span::styled(mode, Style::default().fg(Color::Cyan)),
        Span::raw(" | "),
    ];
    if let Some(feedback) = &app.action_feedback {
        status_spans.push(Span::styled(
            feedback.level.label(),
            feedback_level_style(feedback.level),
        ));
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(
            feedback.message.clone(),
            feedback_level_style(feedback.level),
        ));
        status_spans.push(Span::raw(" | "));
    }
    status_spans.push(Span::raw(app.status.clone()));
    status_spans.push(Span::raw(format!(" | filter: {}", app.filter)));
    if app.editing_filter {
        status_spans.push(Span::styled(" _", Style::default().fg(Color::LightCyan)));
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(status_spans),
            footer_key_line(),
        ])),
        area,
    );
}

fn footer_key_line() -> Line<'static> {
    Line::from(vec![
        key_span("enter"),
        Span::raw(" readonly | "),
        key_span("a"),
        Span::raw(" jump | "),
        key_span("r"),
        Span::raw(" refresh | "),
        key_span("s"),
        Span::raw(" sort | "),
        key_span("/"),
        Span::raw(" filter | "),
        key_span("c"),
        Span::raw(" capture | "),
        key_span("i"),
        Span::raw(" inspect | "),
        key_span("k"),
        Span::raw(" kill | "),
        key_span("e"),
        Span::raw(" rename | "),
        key_span("n"),
        Span::raw(" new-session | "),
        key_span("p"),
        Span::raw(" new-pane | "),
        key_span("q"),
        Span::raw(" quit"),
    ])
}

fn inspected_row<'a>(app: &'a App, selected: &'a SessionSnapshot) -> &'a SessionSnapshot {
    if app.inspected_for.as_deref() == Some(selected.display_id.as_str())
        && let Some(detail) = &app.inspected_detail
    {
        return &detail.session;
    }
    selected
}

fn input_prompt_line(prompt: &InputPrompt) -> Line<'static> {
    let (label, hint) = match &prompt.kind {
        InputPromptKind::RenameSession { host, current } => (
            format!("rename {host}/{current} -> "),
            "enter new session name (Esc to cancel)",
        ),
        InputPromptKind::NewSession => (
            "new session ".to_string(),
            "enter <host>/<session> (Esc to cancel)",
        ),
        InputPromptKind::NewPane => (
            "new pane ".to_string(),
            "enter <host>/<session> (Esc to cancel)",
        ),
    };
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::Yellow)),
        Span::raw(prompt.value.clone()),
        Span::styled(" _", Style::default().fg(Color::LightCyan)),
        Span::raw(" | "),
        Span::styled(hint, muted_style()),
    ])
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
    max_body_lines: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Recent output preview",
        Style::default().add_modifier(Modifier::BOLD),
    ))];

    if let Some(output) = selected_output(app, selected, row).filter(|output| !output.is_empty()) {
        // Emit exactly as many tail lines as will fit in the preview pane so
        // the most recent output is visible rather than clipped off-screen.
        let tail_count = max_body_lines.max(1);
        for line in tail_lines(output, tail_count) {
            lines.push(Line::from(line));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No recent output in cache. Capture or inspect the selected pane to refresh this view.",
            muted_style(),
        )));
    }

    lines
}

fn detail_status_lines(row: &SessionSnapshot) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(repo) = &row.repo {
        if !repo.changed_files.is_empty() {
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
            lines.push(Line::from(vec![
                Span::styled("Repo error: ", Style::default().fg(Color::Red)),
                Span::raw(short_message(error)),
            ]));
        }
    }

    if !row.errors.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
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
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "Candidates",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for target in &row.candidate_targets {
            lines.push(Line::from(target.clone()));
        }
    }

    if let Some(shadowed_by) = &row.shadowed_by {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
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

fn feedback_level_style(level: FeedbackLevel) -> Style {
    match level {
        FeedbackLevel::Info => Style::default().fg(Color::Yellow),
        FeedbackLevel::Success => Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
        FeedbackLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn compact_status_message(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
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

fn table_target(row: &SessionSnapshot) -> String {
    row.raw_target
        .clone()
        .or_else(|| {
            if row.target.trim().is_empty() {
                None
            } else {
                Some(row.target.clone())
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

fn row_identity(row: &SessionSnapshot) -> String {
    row.watch_id
        .as_ref()
        .map(|watch_id| format!("watch:{watch_id}"))
        .or_else(|| {
            row.raw_target
                .as_ref()
                .map(|target| format!("target:{target}"))
        })
        .unwrap_or_else(|| format!("display:{}|{}", row.display_id, row.target))
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

fn compare_rows(left: &SessionSnapshot, right: &SessionSnapshot, mode: SortMode) -> Ordering {
    match mode {
        SortMode::Attention => attention_score(right)
            .cmp(&attention_score(left))
            .then_with(|| state_rank(right.state).cmp(&state_rank(left.state)))
            .then_with(|| left.display_id.cmp(&right.display_id)),
        SortMode::LastOutput => last_output_timestamp(right)
            .cmp(&last_output_timestamp(left))
            .then_with(|| left.display_id.cmp(&right.display_id)),
        SortMode::State => state_rank(right.state)
            .cmp(&state_rank(left.state))
            .then_with(|| left.display_id.cmp(&right.display_id)),
        SortMode::Id => left.display_id.cmp(&right.display_id),
    }
}

fn attention_score(row: &SessionSnapshot) -> usize {
    let mut score = 0;
    score += match row.match_status {
        MatchStatus::Unreachable => 6,
        MatchStatus::Missing => 5,
        MatchStatus::Ambiguous => 4,
        MatchStatus::Shadowed => 3,
        MatchStatus::Unknown => 2,
        MatchStatus::Orphan => 1,
        MatchStatus::Matched => 0,
    };
    score += row.errors.len();
    score += match row.state {
        SessionState::Unreachable => 4,
        SessionState::Missing => 3,
        SessionState::Unknown => 1,
        SessionState::Idle | SessionState::Quiet | SessionState::Active => 0,
    };
    score
}

fn last_output_timestamp(row: &SessionSnapshot) -> i64 {
    row.output
        .as_ref()
        .and_then(|output| output.last_output_at.map(|timestamp| timestamp.timestamp()))
        .unwrap_or(i64::MIN)
}

fn last_output_age(row: &SessionSnapshot) -> String {
    let Some(last_output_at) = row.output.as_ref().and_then(|output| output.last_output_at) else {
        return "-".to_string();
    };
    let age = Utc::now().signed_duration_since(last_output_at);
    if age.num_seconds() < 60 {
        format!("{}s", age.num_seconds().max(0))
    } else if age.num_minutes() < 60 {
        format!("{}m", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h", age.num_hours())
    } else {
        format!("{}d", age.num_days())
    }
}

fn state_rank(state: SessionState) -> u8 {
    match state {
        SessionState::Active => 6,
        SessionState::Quiet => 5,
        SessionState::Idle => 4,
        SessionState::Unknown => 3,
        SessionState::Missing => 2,
        SessionState::Unreachable => 1,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_progress_preserves_management_feedback() {
        let mut app = App::new(String::new());
        app.set_feedback(
            FeedbackLevel::Success,
            "created session pi/new-work; refreshing to confirm",
        );

        app.begin_refresh(vec!["pi".to_string()]);

        let feedback = app.action_feedback.as_ref().expect("feedback");
        assert_eq!(feedback.level, FeedbackLevel::Success);
        assert_eq!(
            feedback.message,
            "created session pi/new-work; refreshing to confirm"
        );
        assert!(app.status.contains("0/1 hosts"));
    }

    #[test]
    fn created_session_confirmation_reports_visible_session() {
        let mut app = App::new(String::new());
        app.add_session_confirmation("pi", "new-work");

        app.apply_refresh(RefreshMessage::HostFinished {
            snapshot: host_snapshot(
                "pi",
                SnapshotStatus::Ok,
                vec![session_snapshot("pi", "new-work")],
                Vec::new(),
            ),
        });

        let feedback = app.action_feedback.as_ref().expect("feedback");
        assert_eq!(feedback.level, FeedbackLevel::Success);
        assert_eq!(
            feedback.message,
            "created session pi/new-work; confirmed in latest scan"
        );
        assert!(app.pending_confirmations.is_empty());
    }

    #[test]
    fn created_session_confirmation_reports_missing_session() {
        let mut app = App::new(String::new());
        app.add_session_confirmation("pi", "new-work");

        app.apply_refresh(RefreshMessage::HostFinished {
            snapshot: host_snapshot("pi", SnapshotStatus::Ok, Vec::new(), Vec::new()),
        });

        let feedback = app.action_feedback.as_ref().expect("feedback");
        assert_eq!(feedback.level, FeedbackLevel::Error);
        assert_eq!(
            feedback.message,
            "created session pi/new-work, but it was not visible in latest scan"
        );
    }

    #[test]
    fn multiline_feedback_is_compacted_for_the_status_bar() {
        assert_eq!(
            compact_status_message("failed to create\n\nhint: verify ssh works"),
            "failed to create | hint: verify ssh works"
        );
    }

    fn host_snapshot(
        host: &str,
        status: SnapshotStatus,
        sessions: Vec<SessionSnapshot>,
        errors: Vec<SnapshotError>,
    ) -> HostSnapshot {
        HostSnapshot {
            host: host.to_string(),
            status,
            collected_at: Utc::now(),
            sessions,
            errors,
        }
    }

    fn session_snapshot(host: &str, session: &str) -> SessionSnapshot {
        SessionSnapshot {
            session_id: format!("{host}/{session}:0.0"),
            target: format!("{host}/{session}:0.0"),
            display_id: format!("{host}/{session}:0.0"),
            raw_target: Some(format!("{host}/{session}:0.0")),
            host: host.to_string(),
            match_status: MatchStatus::Orphan,
            watch_id: None,
            watch_index: None,
            candidate_targets: Vec::new(),
            shadowed_by: None,
            state: SessionState::Active,
            agent_hint: None,
            tmux: snapshot::TmuxSnapshot {
                session: session.to_string(),
                window: Some("0".to_string()),
                pane: Some("0".to_string()),
                pane_id: Some("%1".to_string()),
                session_attached: Some(false),
            },
            process: None,
            repo: None,
            output: None,
            errors: Vec::new(),
        }
    }
}
