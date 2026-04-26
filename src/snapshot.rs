use crate::cache::Cache;
use crate::config::{Config, HostConfig, IndexedWatch, Watch};
use crate::git::{self, RepoSnapshot};
use crate::host;
use crate::tmux::{self, Pane, PaneTarget};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct HostSnapshot {
    pub host: String,
    pub status: SnapshotStatus,
    pub collected_at: DateTime<Utc>,
    pub sessions: Vec<SessionSnapshot>,
    pub errors: Vec<SnapshotError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStatus {
    Ok,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchStatus {
    Matched,
    Orphan,
    Missing,
    Ambiguous,
    Shadowed,
    Unreachable,
    #[allow(dead_code)]
    Unknown,
}

impl MatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchStatus::Matched => "matched",
            MatchStatus::Orphan => "orphan",
            MatchStatus::Missing => "missing",
            MatchStatus::Ambiguous => "ambiguous",
            MatchStatus::Shadowed => "shadowed",
            MatchStatus::Unreachable => "unreachable",
            MatchStatus::Unknown => "unknown",
        }
    }
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

#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub target: String,
    pub display_id: String,
    pub raw_target: Option<String>,
    pub host: String,
    pub match_status: MatchStatus,
    pub watch_id: Option<String>,
    pub watch_index: Option<usize>,
    pub candidate_targets: Vec<String>,
    pub shadowed_by: Option<String>,
    pub state: SessionState,
    pub agent_hint: Option<String>,
    pub tmux: TmuxSnapshot,
    pub process: Option<ProcessSnapshot>,
    pub repo: Option<RepoSnapshot>,
    pub output: Option<OutputSnapshot>,
    pub errors: Vec<SnapshotError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmuxSnapshot {
    pub session: String,
    pub window: Option<String>,
    pub pane: Option<String>,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessSnapshot {
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputSnapshot {
    pub preview: String,
    pub recent: String,
    pub hash: String,
    pub last_output_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaneDetail {
    #[serde(flatten)]
    pub session: SessionSnapshot,
    pub recent_output: String,
}

#[derive(Debug, Clone)]
struct WatchResolution {
    watch: IndexedWatch,
    status: MatchStatus,
    pane_index: Option<usize>,
    candidate_targets: Vec<String>,
    shadowed_by: Option<String>,
}

struct PaneRowContext<'a> {
    watch: Option<&'a IndexedWatch>,
    match_status: MatchStatus,
    candidate_targets: Vec<String>,
    shadowed_by: Option<String>,
}

pub fn snapshot_host(config: &Config, host_id: &str) -> Result<HostSnapshot> {
    let cache_load = Cache::load_with_warning();
    let mut cache = cache_load.cache;
    let mut snapshot = snapshot_host_with_cache(config, host_id, &mut cache)?;
    append_cache_warning(&mut snapshot, cache_load.warning);
    append_cache_save_error(&mut snapshot, cache.save().err());
    Ok(snapshot)
}

pub fn snapshot_all(config: &Config) -> Result<Vec<HostSnapshot>> {
    let cache_load = Cache::load_with_warning();
    let mut cache = cache_load.cache;
    let mut snapshots = Vec::new();
    for host in &config.hosts {
        snapshots.push(snapshot_host_with_cache(config, &host.id, &mut cache)?);
    }
    if let Some(first) = snapshots.first_mut() {
        append_cache_warning(first, cache_load.warning);
        append_cache_save_error(first, cache.save().err());
    }
    Ok(snapshots)
}

pub fn inspect(config: &Config, id_or_target: &str) -> Result<PaneDetail> {
    if config.find_watch(id_or_target).is_some() {
        return inspect_watch(config, id_or_target);
    }
    if let Ok(target) = PaneTarget::parse(id_or_target) {
        return inspect_target(config, &target);
    }

    bail!("unknown watch or pane target `{id_or_target}`")
}

pub fn capture(config: &Config, id_or_target: &str, lines: usize) -> Result<String> {
    if lines == 0 {
        bail!("capture lines must be greater than zero");
    }
    let target = target_for_action(config, id_or_target, "capture")?;
    capture_pane(config, &target, lines)
}

pub fn capture_pane(config: &Config, target: &PaneTarget, lines: usize) -> Result<String> {
    if lines == 0 {
        bail!("capture lines must be greater than zero");
    }
    let host_config = config.host(&target.host)?;
    let command = tmux::capture_command(target, lines);
    host::run(config, host_config, &command)
}

pub fn target_for_action(config: &Config, id_or_target: &str, action: &str) -> Result<PaneTarget> {
    if config.find_watch(id_or_target).is_some() {
        return target_for_watch(config, id_or_target, action);
    }
    if let Ok(target) = PaneTarget::parse(id_or_target) {
        return Ok(target);
    }

    bail!("unknown watch or pane target `{id_or_target}`")
}

fn inspect_watch(config: &Config, watch_id: &str) -> Result<PaneDetail> {
    let watch = config.watch(watch_id)?;
    let cache_load = Cache::load_with_warning();
    let mut cache = cache_load.cache;
    let mut host_snapshot = snapshot_host_with_cache(config, &watch.watch.host, &mut cache)?;
    append_cache_warning(&mut host_snapshot, cache_load.warning);
    append_cache_save_error(&mut host_snapshot, cache.save().err());
    let mut snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| snapshot.watch_id.as_deref() == Some(watch_id))
        .ok_or_else(|| anyhow!("watch `{watch_id}` was not found in snapshot"))?;

    if snapshot.match_status != MatchStatus::Matched {
        return Ok(PaneDetail {
            session: snapshot,
            recent_output: String::new(),
        });
    }

    let target = target_for_snapshot(&snapshot)?;
    let recent_output = capture_for_inspect(config, &target, &mut snapshot)?;
    Ok(PaneDetail {
        session: snapshot,
        recent_output,
    })
}

fn inspect_target(config: &Config, target: &PaneTarget) -> Result<PaneDetail> {
    let cache_load = Cache::load_with_warning();
    let mut cache = cache_load.cache;
    let mut host_snapshot = snapshot_host_with_cache(config, &target.host, &mut cache)?;
    append_cache_warning(&mut host_snapshot, cache_load.warning);
    append_cache_save_error(&mut host_snapshot, cache.save().err());
    let target_string = target.to_string();
    let mut snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| snapshot.raw_target.as_deref() == Some(target_string.as_str()))
        .ok_or_else(|| anyhow!("pane `{target}` was not found on host `{}`", target.host))?;
    let recent_output = capture_for_inspect(config, target, &mut snapshot)?;
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
    let watches = config.watches_for_host(&host_config.id);
    let (watch_resolutions, claimed_targets) = resolve_watch_matches(&watches, panes);
    let mut snapshots = Vec::new();

    for resolution in watch_resolutions {
        match resolution.status {
            MatchStatus::Matched | MatchStatus::Shadowed => {
                let pane = resolution
                    .pane_index
                    .and_then(|index| panes.get(index))
                    .expect("watch resolution pane index should be valid");
                snapshots.push(snapshot_for_pane(
                    config,
                    host_config,
                    pane,
                    cache,
                    now,
                    PaneRowContext {
                        watch: Some(&resolution.watch),
                        match_status: resolution.status,
                        candidate_targets: resolution.candidate_targets,
                        shadowed_by: resolution.shadowed_by,
                    },
                ));
            }
            MatchStatus::Missing | MatchStatus::Ambiguous => {
                snapshots.push(watch_without_pane_snapshot(
                    host_config,
                    &resolution.watch,
                    resolution.status,
                    resolution.candidate_targets,
                    resolution.shadowed_by,
                    None,
                ));
            }
            MatchStatus::Orphan | MatchStatus::Unreachable | MatchStatus::Unknown => {}
        }
    }

    for pane in panes {
        if !claimed_targets.contains(&pane.target) {
            snapshots.push(snapshot_for_pane(
                config,
                host_config,
                pane,
                cache,
                now,
                PaneRowContext {
                    watch: None,
                    match_status: MatchStatus::Orphan,
                    candidate_targets: Vec::new(),
                    shadowed_by: None,
                },
            ));
        }
    }

    snapshots
}

fn resolve_watch_matches(
    watches: &[IndexedWatch],
    panes: &[Pane],
) -> (Vec<WatchResolution>, HashSet<String>) {
    let mut claimed_by: HashMap<String, String> = HashMap::new();
    let mut resolutions = Vec::new();

    for watch in watches {
        let candidates: Vec<usize> = panes
            .iter()
            .enumerate()
            .filter_map(|(index, pane)| watch_matches_pane(&watch.watch, pane).then_some(index))
            .collect();
        let candidate_targets = candidates
            .iter()
            .map(|index| panes[*index].target.clone())
            .collect();

        match candidates.as_slice() {
            [] => resolutions.push(WatchResolution {
                watch: watch.clone(),
                status: MatchStatus::Missing,
                pane_index: None,
                candidate_targets,
                shadowed_by: None,
            }),
            [pane_index] => {
                let pane = &panes[*pane_index];
                if let Some(winner) = claimed_by.get(&pane.target) {
                    resolutions.push(WatchResolution {
                        watch: watch.clone(),
                        status: MatchStatus::Shadowed,
                        pane_index: Some(*pane_index),
                        candidate_targets,
                        shadowed_by: Some(winner.clone()),
                    });
                } else {
                    claimed_by.insert(pane.target.clone(), watch.watch.id.clone());
                    resolutions.push(WatchResolution {
                        watch: watch.clone(),
                        status: MatchStatus::Matched,
                        pane_index: Some(*pane_index),
                        candidate_targets: Vec::new(),
                        shadowed_by: None,
                    });
                }
            }
            _ => resolutions.push(WatchResolution {
                watch: watch.clone(),
                status: MatchStatus::Ambiguous,
                pane_index: None,
                candidate_targets,
                shadowed_by: None,
            }),
        }
    }

    (resolutions, claimed_by.into_keys().collect())
}

fn watch_matches_pane(watch: &Watch, pane: &Pane) -> bool {
    let matcher = &watch.matcher;
    if let Some(command) = &matcher.command
        && pane.command != *command
    {
        return false;
    }
    if let Some(cwd) = &matcher.cwd
        && pane.cwd != *cwd
    {
        return false;
    }
    if let Some(prefix) = &matcher.cwd_prefix
        && !path_matches_prefix(&pane.cwd, prefix)
    {
        return false;
    }
    if let Some(tmux) = &matcher.tmux
        && !tmux::matches_target(pane, tmux)
    {
        return false;
    }
    true
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return path.starts_with('/');
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn snapshot_for_pane(
    config: &Config,
    host_config: &HostConfig,
    pane: &Pane,
    cache: &mut Cache,
    now: DateTime<Utc>,
    row: PaneRowContext<'_>,
) -> SessionSnapshot {
    let target = PaneTarget {
        host: host_config.id.clone(),
        session: pane.session.clone(),
        window: pane.window.clone(),
        pane: pane.pane.clone(),
    };
    let display_id = row
        .watch
        .map(|watch| watch.watch.id.clone())
        .unwrap_or_else(|| pane.target.clone());
    let cache_key = format!("{}/{}", host_config.id, display_id);
    let (state, output, errors) = match capture_pane(config, &target, config.poll.capture_lines) {
        Ok(output) => {
            let output_hash = hash(&output);
            let (state, last_output_at) = cache.update_output(
                &cache_key,
                Some(pane.pane_id.clone()),
                &output_hash,
                now,
                &config.poll,
            );
            (
                state,
                Some(OutputSnapshot {
                    preview: preview(&output),
                    recent: recent_preview(&output, 12),
                    hash: output_hash,
                    last_output_at,
                }),
                Vec::new(),
            )
        }
        Err(err) => (
            SessionState::Unknown,
            None,
            vec![SnapshotError {
                kind: "capture".to_string(),
                message: format!("{err:#}"),
            }],
        ),
    };

    let configured_repo = row.watch.and_then(|watch| watch.watch.repo.as_deref());
    let repo = configured_repo
        .map(|repo| git::collect(config, host_config, repo))
        .or_else(|| git::infer(config, host_config, &pane.cwd));

    SessionSnapshot {
        session_id: display_id.clone(),
        target: pane.target.clone(),
        display_id,
        raw_target: Some(pane.target.clone()),
        host: host_config.id.clone(),
        match_status: row.match_status,
        watch_id: row.watch.map(|watch| watch.watch.id.clone()),
        watch_index: row.watch.map(|watch| watch.index),
        candidate_targets: row.candidate_targets,
        shadowed_by: row.shadowed_by,
        state,
        agent_hint: row.watch.and_then(|watch| watch.watch.agent_hint.clone()),
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
        repo,
        output,
        errors,
    }
}

fn watch_without_pane_snapshot(
    host_config: &HostConfig,
    watch: &IndexedWatch,
    match_status: MatchStatus,
    candidate_targets: Vec<String>,
    shadowed_by: Option<String>,
    message: Option<String>,
) -> SessionSnapshot {
    let target = watch_target_string(host_config, &watch.watch);
    let state = match match_status {
        MatchStatus::Missing => SessionState::Missing,
        MatchStatus::Unreachable => SessionState::Unreachable,
        _ => SessionState::Unknown,
    };
    let message = message.unwrap_or_else(|| match match_status {
        MatchStatus::Missing => format!("watch `{}` did not match a live pane", watch.watch.id),
        MatchStatus::Ambiguous => format!(
            "watch `{}` matched {} live panes",
            watch.watch.id,
            candidate_targets.len()
        ),
        MatchStatus::Shadowed => format!(
            "watch `{}` matched a pane claimed by `{}`",
            watch.watch.id,
            shadowed_by.as_deref().unwrap_or("an earlier watch")
        ),
        MatchStatus::Unreachable => format!("host `{}` was unreachable", host_config.id),
        _ => format!(
            "watch `{}` has status {}",
            watch.watch.id,
            match_status.as_str()
        ),
    });

    SessionSnapshot {
        session_id: watch.watch.id.clone(),
        target,
        display_id: watch.watch.id.clone(),
        raw_target: None,
        host: host_config.id.clone(),
        match_status,
        watch_id: Some(watch.watch.id.clone()),
        watch_index: Some(watch.index),
        candidate_targets,
        shadowed_by,
        state,
        agent_hint: watch.watch.agent_hint.clone(),
        tmux: TmuxSnapshot {
            session: watch
                .watch
                .matcher
                .tmux
                .as_ref()
                .map(|tmux| tmux.session.clone())
                .unwrap_or_else(|| "-".to_string()),
            window: watch
                .watch
                .matcher
                .tmux
                .as_ref()
                .and_then(|tmux| tmux.window.map(|window| window.to_string())),
            pane: watch
                .watch
                .matcher
                .tmux
                .as_ref()
                .and_then(|tmux| tmux.pane.map(|pane| pane.to_string())),
            pane_id: None,
        },
        process: None,
        repo: None,
        output: None,
        errors: vec![SnapshotError {
            kind: match_status.as_str().to_string(),
            message,
        }],
    }
}

fn unreachable_sessions(config: &Config, host_id: &str, message: &str) -> Vec<SessionSnapshot> {
    let host = config.host(host_id).expect("host was already resolved");
    config
        .watches_for_host(host_id)
        .into_iter()
        .map(|watch| {
            watch_without_pane_snapshot(
                host,
                &watch,
                MatchStatus::Unreachable,
                Vec::new(),
                None,
                Some(message.to_string()),
            )
        })
        .collect()
}

fn target_for_watch(config: &Config, watch_id: &str, action: &str) -> Result<PaneTarget> {
    let watch = config.watch(watch_id)?;
    let cache_load = Cache::load_with_warning();
    let mut cache = cache_load.cache;
    let mut host_snapshot = snapshot_host_with_cache(config, &watch.watch.host, &mut cache)?;
    append_cache_warning(&mut host_snapshot, cache_load.warning);
    append_cache_save_error(&mut host_snapshot, cache.save().err());
    let snapshot = host_snapshot
        .sessions
        .into_iter()
        .find(|snapshot| snapshot.watch_id.as_deref() == Some(watch_id))
        .ok_or_else(|| anyhow!("watch `{watch_id}` was not found in snapshot"))?;

    match snapshot.match_status {
        MatchStatus::Matched => target_for_snapshot(&snapshot),
        MatchStatus::Ambiguous => bail!(
            "cannot {action} watch `{watch_id}` because it is ambiguous; candidates: {}",
            snapshot.candidate_targets.join(", ")
        ),
        MatchStatus::Missing => {
            bail!("cannot {action} watch `{watch_id}` because no live pane matched")
        }
        MatchStatus::Shadowed => bail!(
            "cannot {action} watch `{watch_id}` because it is shadowed by `{}`",
            snapshot
                .shadowed_by
                .as_deref()
                .unwrap_or("an earlier watch")
        ),
        MatchStatus::Unreachable => bail!(
            "cannot {action} watch `{watch_id}` because host `{}` is unreachable",
            snapshot.host
        ),
        MatchStatus::Orphan | MatchStatus::Unknown => bail!(
            "cannot {action} watch `{watch_id}` with status {}",
            snapshot.match_status.as_str()
        ),
    }
}

fn target_for_snapshot(snapshot: &SessionSnapshot) -> Result<PaneTarget> {
    Ok(PaneTarget {
        host: snapshot.host.clone(),
        session: snapshot.tmux.session.clone(),
        window: snapshot.tmux.window.clone().ok_or_else(|| {
            anyhow!(
                "session `{}` has no resolved tmux window",
                snapshot.display_id
            )
        })?,
        pane: snapshot.tmux.pane.clone().ok_or_else(|| {
            anyhow!(
                "session `{}` has no resolved tmux pane",
                snapshot.display_id
            )
        })?,
    })
}

fn watch_target_string(host_config: &HostConfig, watch: &Watch) -> String {
    let Some(tmux) = &watch.matcher.tmux else {
        return "-".to_string();
    };
    match (tmux.window, tmux.pane) {
        (Some(window), Some(pane)) => {
            format!("{}/{}:{}.{}", host_config.id, tmux.session, window, pane)
        }
        (Some(window), None) => format!("{}/{}:{}.*", host_config.id, tmux.session, window),
        _ => format!("{}/{}", host_config.id, tmux.session),
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

fn recent_preview(output: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn hash(output: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(output.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn capture_for_inspect(
    config: &Config,
    target: &PaneTarget,
    snapshot: &mut SessionSnapshot,
) -> Result<String> {
    match capture_pane(config, target, config.poll.capture_lines) {
        Ok(output) => Ok(output),
        Err(err) => {
            let message = format!("{err:#}");
            if !snapshot
                .errors
                .iter()
                .any(|error| error.kind == "capture" && error.message == message)
            {
                snapshot.errors.push(SnapshotError {
                    kind: "capture".to_string(),
                    message,
                });
            }
            Ok(String::new())
        }
    }
}

fn append_cache_warning(snapshot: &mut HostSnapshot, warning: Option<String>) {
    if let Some(message) = warning {
        snapshot.errors.push(SnapshotError {
            kind: "cache".to_string(),
            message,
        });
    }
}

fn append_cache_save_error(snapshot: &mut HostSnapshot, error: Option<anyhow::Error>) {
    if let Some(error) = error {
        snapshot.errors.push(SnapshotError {
            kind: "cache".to_string(),
            message: format!("{error:#}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{IndexedWatch, TmuxTarget, Watch, WatchMatchConfig};

    #[test]
    fn command_and_cwd_watch_survives_coordinate_drift() {
        let watch = watch(
            "rpi2-kiro",
            WatchMatchConfig {
                command: Some("kiro-cli".to_string()),
                cwd: Some("/home/cam".to_string()),
                cwd_prefix: None,
                tmux: None,
            },
        );
        let panes = vec![pane("rpi2/0:3.2", "0", "3", "2", "kiro-cli", "/home/cam")];

        let (resolutions, claimed) = resolve_watch_matches(&[watch], &panes);

        assert_eq!(resolutions[0].status, MatchStatus::Matched);
        assert_eq!(resolutions[0].pane_index, Some(0));
        assert!(claimed.contains("rpi2/0:3.2"));
    }

    #[test]
    fn ambiguous_watch_does_not_claim_candidates() {
        let watch = watch(
            "node-agent",
            WatchMatchConfig {
                command: Some("node".to_string()),
                cwd: None,
                cwd_prefix: Some("/home/cam/openclaw".to_string()),
                tmux: None,
            },
        );
        let panes = vec![
            pane(
                "rpi2/work:0.0",
                "work",
                "0",
                "0",
                "node",
                "/home/cam/openclaw",
            ),
            pane(
                "rpi2/work:0.1",
                "work",
                "0",
                "1",
                "node",
                "/home/cam/openclaw/src",
            ),
        ];

        let (resolutions, claimed) = resolve_watch_matches(&[watch], &panes);

        assert_eq!(resolutions[0].status, MatchStatus::Ambiguous);
        assert_eq!(resolutions[0].candidate_targets.len(), 2);
        assert!(claimed.is_empty());
    }

    #[test]
    fn later_watch_is_shadowed_by_earlier_claim() {
        let first = watch(
            "first",
            WatchMatchConfig {
                command: Some("bash".to_string()),
                cwd: Some("/tmp".to_string()),
                cwd_prefix: None,
                tmux: None,
            },
        );
        let second = watch(
            "second",
            WatchMatchConfig {
                command: None,
                cwd: None,
                cwd_prefix: None,
                tmux: Some(TmuxTarget {
                    session: "scratch".to_string(),
                    window: Some(1),
                    pane: Some(0),
                }),
            },
        );
        let panes = vec![pane(
            "local/scratch:1.0",
            "scratch",
            "1",
            "0",
            "bash",
            "/tmp",
        )];

        let (resolutions, claimed) = resolve_watch_matches(&[first, second], &panes);

        assert_eq!(resolutions[0].status, MatchStatus::Matched);
        assert_eq!(resolutions[1].status, MatchStatus::Shadowed);
        assert_eq!(resolutions[1].shadowed_by.as_deref(), Some("first"));
        assert!(claimed.contains("local/scratch:1.0"));
    }

    fn watch(id: &str, matcher: WatchMatchConfig) -> IndexedWatch {
        IndexedWatch {
            index: 0,
            watch: Watch {
                id: id.to_string(),
                host: "rpi2".to_string(),
                matcher,
                repo: None,
                agent_hint: None,
            },
        }
    }

    fn pane(
        target: &str,
        session: &str,
        window: &str,
        pane_index: &str,
        command: &str,
        cwd: &str,
    ) -> Pane {
        Pane {
            target: target.to_string(),
            host: target.split('/').next().unwrap().to_string(),
            session: session.to_string(),
            window: window.to_string(),
            pane: pane_index.to_string(),
            pane_id: "%1".to_string(),
            pid: Some(1),
            command: command.to_string(),
            cwd: cwd.to_string(),
        }
    }
}
