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
    last_refresh: Option<Instant>,
    sort_mode: SortMode,
    show_context: bool,
    inspect_mode: bool,
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
    sessions: usize,
    quiet: usize,
    idle: usize,
    missing: usize,
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
            last_refresh: None,
            sort_mode: SortMode::Attention,
            show_context: true,
            inspect_mode: false,
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
        let missing = count_state(SessionState::Missing);
        DashboardSummary {
            sessions: sessions.len(),
            quiet: count_state(SessionState::Quiet),
            idle: count_state(SessionState::Idle),
            missing,
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
}

impl HostProgressState {
    fn is_finished(self) -> bool {
        matches!(self, HostProgressState::Ok | HostProgressState::Unreachable)
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
                        app.status = format!("killed {}", prompt.target);
                        spawn_refresh(config, host, tx, app)?;
                    }
                    Err(err) => app.status = format!("{err:#}"),
                }
            }
            _ => {
                app.kill_prompt = None;
                app.status = "kill cancelled".to_string();
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
        KeyCode::Esc => {
            if app.inspect_mode {
                app.inspect_mode = false;
                app.status = "browse".to_string();
                Ok(false)
            } else if app.help {
                app.help = false;
                Ok(false)
            } else {
                Ok(true)
            }
        }
        KeyCode::Char('q') => Ok(true),
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
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected = app.selected.saturating_sub(1);
            app.selected_identity = app.selected_row_identity();
            Ok(false)
        }
        KeyCode::Char('x') => {
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
            app.inspect_mode = true;
            inspect_selected(config, app)?;
            Ok(false)
        }
        KeyCode::Char('d') => {
            app.show_context = !app.show_context;
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
            app.status = "action cancelled".to_string();
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
                app.status = "rename cancelled: empty name".to_string();
                return Ok(());
            }
            if new_name == current {
                app.status = "rename skipped: unchanged".to_string();
                return Ok(());
            }
            match lifecycle::rename_session(config, &host_id, &current, new_name, false) {
                Ok(()) => {
                    app.status = format!("renamed {host_id}/{current} -> {new_name}");
                    spawn_refresh(config, Some(host_id), tx, app)?;
                }
                Err(err) => app.status = format!("{err:#}"),
            }
        }
        InputPromptKind::NewSession => {
            let Some((host_id, session_name)) = parse_host_session(prompt.value.trim()) else {
                app.status = "new session expects <host>/<session>".to_string();
                return Ok(());
            };
            if let Some(scope) = scoped_host.as_deref()
                && scope != host_id
            {
                app.status = format!(
                    "new session blocked in scoped view: expected host `{scope}`, got `{host_id}`"
                );
                return Ok(());
            }
            match lifecycle::new_session(config, host_id, session_name, None, None, false) {
                Ok(()) => {
                    app.status = format!("created session {host_id}/{session_name}");
                    spawn_refresh(config, Some(host_id.to_string()), tx, app)?;
                }
                Err(err) => app.status = format!("{err:#}"),
            }
        }
        InputPromptKind::NewPane => {
            let Some((host_id, session_name)) = parse_host_session(prompt.value.trim()) else {
                app.status = "new pane expects <host>/<session>".to_string();
                return Ok(());
            };
            if let Some(scope) = scoped_host.as_deref()
                && scope != host_id
            {
                app.status = format!(
                    "new pane blocked in scoped view: expected host `{scope}`, got `{host_id}`"
                );
                return Ok(());
            }
            match lifecycle::new_pane(config, host_id, session_name, false) {
                Ok(()) => {
                    app.status = format!("spawned pane in {host_id}/{session_name}");
                    spawn_refresh(config, Some(host_id.to_string()), tx, app)?;
                }
                Err(err) => app.status = format!("{err:#}"),
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
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);

    draw_summary(frame, chunks[0], app);
    if app.inspect_mode || app.help {
        draw_inspector(frame, chunks[1], app);
    } else if app.show_context && area.width > 110 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(chunks[1]);
        draw_live_table(frame, body[0], app);
        draw_context_rail(frame, body[1], app);
    } else {
        draw_live_table(frame, chunks[1], app);
    }
    draw_status(frame, chunks[2], app);
}

fn draw_summary(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let summary = app.summary();
    let mode = if app.inspect_mode {
        "inspect"
    } else if app.editing_filter {
        "filter"
    } else {
        "browse"
    };
    let line = Line::from(vec![
        Span::styled("remux", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" | "),
        Span::raw(format!("/ {}", filter_label(app))),
        Span::raw(" | "),
        Span::raw(format!(
            "{} targets  {} stale  {} missing",
            summary.sessions,
            summary.quiet + summary.idle,
            summary.missing
        )),
        Span::raw(" | "),
        Span::styled(mode, Style::default().fg(Color::Cyan)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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
        let hint = row_action_hint(session);
        Row::new(vec![
            Cell::from(session.display_id.clone()),
            Cell::from(session.host.clone()),
            Cell::from(canonical_state(session)).style(canonical_state_style(session)),
            Cell::from(table_target(session)),
            Cell::from(last_output_age(session)),
            Cell::from(
                session
                    .process
                    .as_ref()
                    .map(|process| process.command.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(hint).style(muted_style()),
        ])
        .style(Style::default())
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Min(18),
            Constraint::Length(12),
            Constraint::Length(11),
            Constraint::Min(14),
            Constraint::Length(8),
            Constraint::Min(16),
            Constraint::Min(22),
        ],
    )
    .header(
        Row::new(["NAME", "HOST", "STATE", "TARGET", "AGE", "CMD", "ACTION"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default())
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
            Paragraph::new(Line::from(vec![
                Span::styled("confirm ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("kill {}? (y/N)", prompt.target)),
            ])),
            area,
        );
        return;
    }
    if let Some(prompt) = &app.input_prompt {
        frame.render_widget(Paragraph::new(input_prompt_line(prompt)), area);
        return;
    }

    let mode = if app.editing_filter {
        "filter"
    } else if app.polling {
        "polling"
    } else {
        "ready"
    };
    let mut spans = vec![
        Span::raw(
            "↑↓ move  Enter attach(ro)  a jump(rw)  i inspect  / filter  d details  ? help  x kill  q quit  ",
        ),
        Span::styled(mode, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(short_message(&app.status), muted_style()),
    ];
    if app.editing_filter {
        spans.push(Span::styled(" _", Style::default().fg(Color::LightCyan)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_context_rail(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(selected) = app.selected_row() else {
        return;
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("target ", label_style()),
            Span::raw(table_target(selected)),
        ]),
        Line::from(vec![
            Span::styled("host ", label_style()),
            Span::raw(selected.host.clone()),
        ]),
        Line::from(vec![
            Span::styled("state ", label_style()),
            Span::styled(canonical_state(selected), canonical_state_style(selected)),
        ]),
        Line::from(vec![
            Span::styled("cmd ", label_style()),
            Span::raw(
                selected
                    .process
                    .as_ref()
                    .map(|p| short_message(&p.command))
                    .unwrap_or_else(|| "-".into()),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled("press i for full inspect", muted_style())),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title("context").borders(Borders::LEFT)),
        area,
    );
}

fn canonical_state(row: &SessionSnapshot) -> &'static str {
    if matches!(row.match_status, MatchStatus::Ambiguous) {
        "ambiguous"
    } else if matches!(
        row.match_status,
        MatchStatus::Shadowed | MatchStatus::Orphan | MatchStatus::Unknown
    ) || matches!(row.state, SessionState::Unknown)
    {
        "drift"
    } else if matches!(
        row.match_status,
        MatchStatus::Missing | MatchStatus::Unreachable
    ) || matches!(row.state, SessionState::Missing | SessionState::Unreachable)
    {
        "missing"
    } else if row.state == SessionState::Idle {
        "stale"
    } else if row.state == SessionState::Quiet {
        "busy"
    } else {
        "ready"
    }
}
fn canonical_state_style(row: &SessionSnapshot) -> Style {
    match canonical_state(row) {
        "ready" => Style::default().fg(Color::Green),
        "stale" => Style::default().fg(Color::Yellow),
        "drift" => Style::default().fg(Color::LightBlue),
        "busy" => Style::default().fg(Color::LightYellow),
        "missing" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "ambiguous" => Style::default().fg(Color::Magenta),
        _ => Style::default(),
    }
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

fn row_action_hint(session: &SessionSnapshot) -> String {
    if let Some(reason) = attach_refusal_reason(session) {
        return format!("blocked: {reason}");
    }
    if session.watch_id.is_some() {
        return "enter attach watch".to_string();
    }
    if let Some(target) = &session.raw_target {
        return format!("enter attach {target}");
    }
    if let Some(output) = &session.output {
        return short_message(&output.preview);
    }
    "enter attach".to_string()
}
