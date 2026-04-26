use crate::cache::Cache;
use crate::config::{Config, HostConfig, SessionConfig};
use crate::git::{self, RepoSnapshot};
use crate::host;
use crate::tmux::{self, Pane, PaneTarget};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Serialize)]
pub struct HostSnapshot {
    pub host: String,
    pub status: SnapshotStatus,
    pub collected_at: DateTime<Utc>,
    pub sessions: Vec<SessionSnapshot>,
    pub errors: Vec<SnapshotError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStatus {
    Ok,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Active,
    Quiet,
    Idle,
    Missing,
    Unreachable,
    Unknown,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Active => "active",
            SessionState::Quiet => "quiet",
            SessionState::Idle => "idle",
            SessionState::Missing => "missing",
            SessionState::Unreachable => "unreachable",
            SessionState::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub target: String,
    pub host: String,
    pub state: SessionState,
    pub agent_hint: Option<String>,
    pub tmux: TmuxSnapshot,
    pub process: Option<ProcessSnapshot>,
    pub repo: Option<RepoSnapshot>,
    pub output: Option<OutputSnapshot>,
    pub errors: Vec<SnapshotError>,
}

#[derive(Debug, Serialize)]
pub struct TmuxSnapshot {
    pub session: String,
    pub window: Option<String>,
    pub pane: Option<String>,
    pub pane_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProcessSnapshot {
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: String,
}

#[derive(Debug, Serialize)]
pub struct OutputSnapshot {
    pub preview: String,
    pub hash: String,
    pub last_output_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct PaneDetail {
    #[serde(flatten)]
    pub session: SessionSnapshot,
    pub recent_output: String,
}

pub fn snapshot_host(config: &Config, host_id: &str) -> Result<HostSnapshot> {
    let mut cache = Cache::load();
    let snapshot = snapshot_host_with_cache(config, host_id, &mut cache)?;
    let _ = cache.save();
    Ok(snapshot)
}

pub fn snapshot_all(config: &Config) -> Result<Vec<HostSnapshot>> {
    let mut cache = Cache::load();
    let mut snapshots = Vec::new();
    for host in &config.hosts {
        snapshots.push(snapshot_host_with_cache(config, &host.id, &mut cache)?);
    }
    let _ = cache.save();
    Ok(snapshots)
}

pub fn inspect(config: &Config, id_or_target: &str) -> Result<PaneDetail> {
    if let Ok(target) = PaneTarget::parse(id_or_target) {
        return inspect_target(config, &target);
    }

    let session = config.session(id_or_target)?;
    let mut cache = Cache::load();
    let host_snapshot = snapshot_host_with_cache(config, &session.host, &mut cache)?;
    let _ = cache.save();
    let snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| snapshot.session_id == session.id)
        .ok_or_else(|| anyhow!("session `{}` was not found in snapshot", session.id))?;

    if snapshot.state == SessionState::Missing || snapshot.state == SessionState::Unreachable {
        return Ok(PaneDetail {
            session: snapshot,
            recent_output: String::new(),
        });
    }

    let target = target_for_snapshot(&snapshot)?;
    let recent_output = capture_pane(config, &target, config.poll.capture_lines)?;
    Ok(PaneDetail {
        session: snapshot,
        recent_output,
    })
}

pub fn capture(config: &Config, id_or_target: &str, lines: usize) -> Result<String> {
    if let Ok(target) = PaneTarget::parse(id_or_target) {
        return capture_pane(config, &target, lines);
    }

    let session = config.session(id_or_target)?;
    let target = target_for_session(config, session)?;
    capture_pane(config, &target, lines)
}

pub fn capture_pane(config: &Config, target: &PaneTarget, lines: usize) -> Result<String> {
    let host_config = config.host(&target.host)?;
    let command = tmux::capture_command(target, lines);
    host::run(config, host_config, &command)
}

fn inspect_target(config: &Config, target: &PaneTarget) -> Result<PaneDetail> {
    let mut cache = Cache::load();
    let host_snapshot = snapshot_host_with_cache(config, &target.host, &mut cache)?;
    let _ = cache.save();
    let snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| {
            snapshot.tmux.session == target.session
                && snapshot.tmux.window.as_deref() == Some(target.window.as_str())
                && snapshot.tmux.pane.as_deref() == Some(target.pane.as_str())
        })
        .ok_or_else(|| anyhow!("pane `{target}` was not found on host `{}`", target.host))?;
    let recent_output = capture_pane(config, target, config.poll.capture_lines)?;
    Ok(PaneDetail {
        session: snapshot,
        recent_output,
    })
}

fn snapshot_host_with_cache(
    config: &Config,
    host_id: &str,
    cache: &mut Cache,
) -> Result<HostSnapshot> {
    let host_config = config.host(host_id)?;
    let now = Utc::now();
    match host::run(config, host_config, tmux::INVENTORY_COMMAND) {
        Ok(output) => {
            let panes = tmux::parse_inventory(host_id, &output)?;
            let sessions = sessions_from_panes(config, host_config, &panes, cache, now);
            cache.update_host(host_id, "ok", now);
            Ok(HostSnapshot {
                host: host_id.to_string(),
                status: SnapshotStatus::Ok,
                collected_at: now,
                sessions,
                errors: Vec::new(),
            })
        }
        Err(err) => {
            cache.update_host(host_id, "unreachable", now);
            let message = format!("{err:#}");
            Ok(HostSnapshot {
                host: host_id.to_string(),
                status: SnapshotStatus::Unreachable,
                collected_at: now,
                sessions: unreachable_sessions(config, host_id, &message),
                errors: vec![SnapshotError {
                    kind: "poll".to_string(),
                    message,
                }],
            })
        }
    }
}

fn sessions_from_panes(
    config: &Config,
    host_config: &HostConfig,
    panes: &[Pane],
    cache: &mut Cache,
    now: DateTime<Utc>,
) -> Vec<SessionSnapshot> {
    let configured = config.sessions_for_host(&host_config.id);
    if configured.is_empty() {
        return panes
            .iter()
            .map(|pane| snapshot_for_pane(config, host_config, None, pane, cache, now))
            .collect();
    }

    let mut seen_targets = HashSet::new();
    let mut snapshots = Vec::new();
    for session in configured {
        match panes
            .iter()
            .find(|pane| tmux::matches_target(pane, &session.tmux))
        {
            Some(pane) => {
                seen_targets.insert(pane.target.clone());
                snapshots.push(snapshot_for_pane(
                    config,
                    host_config,
                    Some(session),
                    pane,
                    cache,
                    now,
                ));
            }
            None => snapshots.push(missing_session_snapshot(host_config, session)),
        }
    }

    for pane in panes {
        if !seen_targets.contains(&pane.target) {
            snapshots.push(snapshot_for_pane(
                config,
                host_config,
                None,
                pane,
                cache,
                now,
            ));
        }
    }

    snapshots
}

fn snapshot_for_pane(
    config: &Config,
    host_config: &HostConfig,
    session: Option<&SessionConfig>,
    pane: &Pane,
    cache: &mut Cache,
    now: DateTime<Utc>,
) -> SessionSnapshot {
    let target = PaneTarget {
        host: host_config.id.clone(),
        session: pane.session.clone(),
        window: pane.window.clone(),
        pane: pane.pane.clone(),
    };
    let output = capture_pane(config, &target, config.poll.capture_lines)
        .unwrap_or_else(|err| format!("failed to capture pane output: {err:#}\n"));
    let output_hash = hash(&output);
    let session_id = session
        .map(|session| session.id.clone())
        .unwrap_or_else(|| pane.target.clone());
    let cache_key = format!("{}/{}", host_config.id, session_id);
    let (state, last_output_at) = cache.update_output(
        &cache_key,
        Some(pane.pane_id.clone()),
        &output_hash,
        now,
        &config.poll,
    );

    SessionSnapshot {
        session_id,
        target: pane.target.clone(),
        host: host_config.id.clone(),
        state,
        agent_hint: session.and_then(|session| session.agent_hint.clone()),
        tmux: TmuxSnapshot {
            session: pane.session.clone(),
            window: Some(pane.window.clone()),
            pane: Some(pane.pane.clone()),
            pane_id: Some(pane.pane_id.clone()),
        },
        process: Some(ProcessSnapshot {
            pid: pane.pid,
            command: pane.command.clone(),
            cwd: pane.cwd.clone(),
        }),
        repo: session
            .and_then(|session| session.repo.as_deref())
            .map(|repo| git::collect(config, host_config, repo)),
        output: Some(OutputSnapshot {
            preview: preview(&output),
            hash: output_hash,
            last_output_at,
        }),
        errors: Vec::new(),
    }
}

fn missing_session_snapshot(host_config: &HostConfig, session: &SessionConfig) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session.id.clone(),
        target: session_target_string(host_config, session),
        host: host_config.id.clone(),
        state: SessionState::Missing,
        agent_hint: session.agent_hint.clone(),
        tmux: TmuxSnapshot {
            session: session.tmux.session.clone(),
            window: session.tmux.window.map(|window| window.to_string()),
            pane: session.tmux.pane.map(|pane| pane.to_string()),
            pane_id: None,
        },
        process: None,
        repo: None,
        output: None,
        errors: vec![SnapshotError {
            kind: "missing".to_string(),
            message: format!("configured session `{}` was not found", session.id),
        }],
    }
}

fn unreachable_sessions(config: &Config, host_id: &str, message: &str) -> Vec<SessionSnapshot> {
    config
        .sessions_for_host(host_id)
        .into_iter()
        .map(|session| SessionSnapshot {
            session_id: session.id.clone(),
            target: session_target_string(
                config.host(host_id).expect("host was already resolved"),
                session,
            ),
            host: host_id.to_string(),
            state: SessionState::Unreachable,
            agent_hint: session.agent_hint.clone(),
            tmux: TmuxSnapshot {
                session: session.tmux.session.clone(),
                window: session.tmux.window.map(|window| window.to_string()),
                pane: session.tmux.pane.map(|pane| pane.to_string()),
                pane_id: None,
            },
            process: None,
            repo: None,
            output: None,
            errors: vec![SnapshotError {
                kind: "unreachable".to_string(),
                message: message.to_string(),
            }],
        })
        .collect()
}

fn target_for_session(config: &Config, session: &SessionConfig) -> Result<PaneTarget> {
    let mut cache = Cache::load();
    let host_snapshot = snapshot_host_with_cache(config, &session.host, &mut cache)?;
    let _ = cache.save();
    let snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| snapshot.session_id == session.id)
        .ok_or_else(|| anyhow!("session `{}` was not found in snapshot", session.id))?;
    target_for_snapshot(&snapshot)
}

fn target_for_snapshot(snapshot: &SessionSnapshot) -> Result<PaneTarget> {
    Ok(PaneTarget {
        host: snapshot.host.clone(),
        session: snapshot.tmux.session.clone(),
        window: snapshot.tmux.window.clone().ok_or_else(|| {
            anyhow!(
                "session `{}` has no resolved tmux window",
                snapshot.session_id
            )
        })?,
        pane: snapshot.tmux.pane.clone().ok_or_else(|| {
            anyhow!(
                "session `{}` has no resolved tmux pane",
                snapshot.session_id
            )
        })?,
    })
}

fn session_target_string(host_config: &HostConfig, session: &SessionConfig) -> String {
    match (session.tmux.window, session.tmux.pane) {
        (Some(window), Some(pane)) => {
            format!(
                "{}/{}:{}.{}",
                host_config.id, session.tmux.session, window, pane
            )
        }
        (Some(window), None) => format!("{}/{}:{}.*", host_config.id, session.tmux.session, window),
        _ => format!("{}/{}", host_config.id, session.tmux.session),
    }
}

fn preview(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

fn hash(output: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(output.as_bytes());
    format!("{:x}", hasher.finalize())
}
