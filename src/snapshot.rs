use crate::config::Config;
use crate::ssh;
use crate::tmux::{self, Pane, PaneTarget};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub struct HostSnapshot {
    pub host: String,
    pub status: SnapshotStatus,
    pub collected_at: DateTime<Utc>,
    pub panes: Vec<PaneSnapshot>,
    pub errors: Vec<SnapshotError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStatus {
    Ok,
    Unreachable,
}

#[derive(Debug, Serialize)]
pub struct PaneSnapshot {
    #[serde(flatten)]
    pub pane: Pane,
    pub output: OutputPreview,
}

#[derive(Debug, Serialize)]
pub struct OutputPreview {
    pub preview: String,
    pub hash: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct PaneDetail {
    #[serde(flatten)]
    pub pane: Pane,
    pub output: String,
    pub output_hash: String,
}

pub fn snapshot_host(config: &Config, host_id: &str) -> Result<HostSnapshot> {
    let host = config.host(host_id)?;
    match ssh::run(host, tmux::INVENTORY_COMMAND) {
        Ok(output) => {
            let panes = tmux::parse_inventory(host_id, &output)?
                .into_iter()
                .map(|pane| {
                    let target = PaneTarget {
                        host: host_id.to_string(),
                        session: pane.session.clone(),
                        window: pane.window.clone(),
                        pane: pane.pane.clone(),
                    };
                    let output = capture_pane(config, &target, 20)
                        .unwrap_or_else(|err| format!("failed to capture pane output: {err:#}\n"));
                    PaneSnapshot {
                        pane,
                        output: OutputPreview {
                            preview: preview(&output),
                            hash: hash(&output),
                        },
                    }
                })
                .collect();
            Ok(HostSnapshot {
                host: host_id.to_string(),
                status: SnapshotStatus::Ok,
                collected_at: Utc::now(),
                panes,
                errors: Vec::new(),
            })
        }
        Err(err) => Ok(HostSnapshot {
            host: host_id.to_string(),
            status: SnapshotStatus::Unreachable,
            collected_at: Utc::now(),
            panes: Vec::new(),
            errors: vec![SnapshotError {
                kind: "ssh".to_string(),
                message: format!("{err:#}"),
            }],
        }),
    }
}

pub fn inspect_pane(config: &Config, target: &PaneTarget) -> Result<PaneDetail> {
    let host = config.host(&target.host)?;
    let inventory = ssh::run(host, tmux::INVENTORY_COMMAND)?;
    let panes = tmux::parse_inventory(&target.host, &inventory)?;
    let pane = panes
        .into_iter()
        .find(|pane| {
            pane.session == target.session
                && pane.window == target.window
                && pane.pane == target.pane
        })
        .ok_or_else(|| anyhow!("pane `{target}` was not found on host `{}`", target.host))?;
    let output = capture_pane(config, target, 120)?;
    Ok(PaneDetail {
        pane,
        output_hash: hash(&output),
        output,
    })
}

pub fn capture_pane(config: &Config, target: &PaneTarget, lines: usize) -> Result<String> {
    let host = config.host(&target.host)?;
    let command = tmux::capture_command(target, lines);
    ssh::run(host, &command)
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
