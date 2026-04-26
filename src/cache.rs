use crate::config::PollConfig;
use crate::snapshot::SessionState;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Cache {
    #[serde(default)]
    pub entries: BTreeMap<String, CacheEntry>,
    #[serde(default)]
    pub hosts: BTreeMap<String, HostCacheEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheEntry {
    pub pane_id: Option<String>,
    pub output_hash: String,
    pub last_output_at: Option<DateTime<Utc>>,
    pub last_successful_poll_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostCacheEntry {
    pub status: String,
    pub last_poll_at: DateTime<Utc>,
}

impl Cache {
    pub fn load() -> Self {
        let path = cache_path();
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = cache_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cache directory {}", parent.display())
            })?;
        }
        fs::write(&path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("failed to write cache {}", path.display()))
    }

    pub fn update_host(&mut self, host: &str, status: impl Into<String>, now: DateTime<Utc>) {
        self.hosts.insert(
            host.to_string(),
            HostCacheEntry {
                status: status.into(),
                last_poll_at: now,
            },
        );
    }

    pub fn update_output(
        &mut self,
        key: &str,
        pane_id: Option<String>,
        output_hash: &str,
        now: DateTime<Utc>,
        poll: &PollConfig,
    ) -> (SessionState, Option<DateTime<Utc>>) {
        let prior = self.entries.get(key);
        let (state, last_output_at) = match prior {
            Some(entry) if entry.output_hash == output_hash => {
                let last_output_at = entry.last_output_at.or(Some(entry.last_successful_poll_at));
                (
                    state_from_last_output(last_output_at, now, poll),
                    last_output_at,
                )
            }
            Some(_) => {
                let last_output_at = Some(now);
                (
                    state_from_last_output(last_output_at, now, poll),
                    last_output_at,
                )
            }
            None => (SessionState::Unknown, Some(now)),
        };

        self.entries.insert(
            key.to_string(),
            CacheEntry {
                pane_id,
                output_hash: output_hash.to_string(),
                last_output_at,
                last_successful_poll_at: now,
            },
        );

        (state, last_output_at)
    }
}

fn state_from_last_output(
    last_output_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    poll: &PollConfig,
) -> SessionState {
    let Some(last_output_at) = last_output_at else {
        return SessionState::Unknown;
    };
    let elapsed = now
        .signed_duration_since(last_output_at)
        .to_std()
        .unwrap_or_default();
    if elapsed <= poll.active_after {
        SessionState::Active
    } else if elapsed >= poll.idle_after {
        SessionState::Idle
    } else {
        SessionState::Quiet
    }
}

pub fn cache_path() -> PathBuf {
    if let Some(path) = std::env::var_os("REMUX_CACHE_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".local/share/remux/cache.json")
}
