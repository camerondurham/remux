use crate::snapshot::{HostSnapshot, MatchStatus, SessionSnapshot, SessionState};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct SessionRollup {
    pub host: String,
    pub session: String,
    pub target: String,
    pub windows: usize,
    pub panes: usize,
    pub attached: bool,
    pub state: SessionState,
    pub match_status: MatchStatus,
    pub active_cmd: Option<String>,
    pub repo: Option<String>,
}

#[derive(Default)]
struct RollupBuilder {
    host: String,
    session: String,
    target: Option<String>,
    windows: BTreeSet<String>,
    pane_targets: BTreeSet<String>,
    panes: usize,
    attached: bool,
    state: Option<SessionState>,
    matched: bool,
    watched_panes: usize,
    active_cmd: Option<String>,
    watched_repos: Vec<String>,
}

pub fn rollups_from_snapshots(snapshots: &[HostSnapshot]) -> Vec<SessionRollup> {
    let mut builders: BTreeMap<(String, String), RollupBuilder> = BTreeMap::new();

    for snapshot in snapshots {
        for row in &snapshot.sessions {
            if row.raw_target.is_none() {
                continue;
            }
            let key = (row.host.clone(), row.tmux.session.clone());
            builders
                .entry(key)
                .or_insert_with(|| RollupBuilder {
                    host: row.host.clone(),
                    session: row.tmux.session.clone(),
                    ..RollupBuilder::default()
                })
                .push(row);
        }
    }

    builders.into_values().map(RollupBuilder::finish).collect()
}

impl RollupBuilder {
    fn push(&mut self, row: &SessionSnapshot) {
        let Some(raw_target) = &row.raw_target else {
            return;
        };
        if !self.pane_targets.insert(raw_target.clone()) {
            return;
        }

        if self.target.is_none() || is_more_active(row.state, self.state.unwrap_or(row.state)) {
            self.target = row.raw_target.clone();
            self.active_cmd = row.process.as_ref().map(|process| process.command.clone());
        }
        if let Some(window) = &row.tmux.window {
            self.windows.insert(window.clone());
        }
        self.panes += 1;
        self.attached |= row.tmux.session_attached.unwrap_or(false);
        self.state = Some(match self.state {
            Some(existing) if !is_more_active(row.state, existing) => existing,
            _ => row.state,
        });
        self.matched |= row.match_status == MatchStatus::Matched;
        if row.watch_id.is_some() {
            self.watched_panes += 1;
            if let Some(repo) = &row.repo {
                self.watched_repos.push(repo.path.clone());
            }
        }
    }

    fn finish(self) -> SessionRollup {
        SessionRollup {
            host: self.host,
            session: self.session,
            target: self.target.unwrap_or_else(|| "-".to_string()),
            windows: self.windows.len(),
            panes: self.panes,
            attached: self.attached,
            state: self.state.unwrap_or(SessionState::Unknown),
            match_status: if self.matched {
                MatchStatus::Matched
            } else {
                MatchStatus::Orphan
            },
            active_cmd: self.active_cmd,
            repo: if self.watched_panes == self.watched_repos.len() {
                common_repo(self.watched_repos)
            } else {
                None
            },
        }
    }
}

fn is_more_active(candidate: SessionState, current: SessionState) -> bool {
    state_rank(candidate) > state_rank(current)
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

fn common_repo(repos: Vec<String>) -> Option<String> {
    let mut iter = repos.into_iter();
    let first = iter.next()?;
    iter.all(|repo| repo == first).then_some(first)
}
