use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub poll: PollConfig,
    #[serde(default)]
    pub hosts: Vec<HostConfig>,
    #[serde(default)]
    pub sessions: Vec<SessionConfig>,
}

#[derive(Debug, Deserialize)]
pub struct PollConfig {
    #[serde(
        default = "default_active_after",
        deserialize_with = "deserialize_duration"
    )]
    pub active_after: Duration,
    #[serde(
        default = "default_idle_after",
        deserialize_with = "deserialize_duration"
    )]
    pub idle_after: Duration,
    #[serde(default = "default_capture_lines")]
    pub capture_lines: usize,
    #[serde(
        default = "default_ssh_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub ssh_timeout: Duration,
    #[serde(
        default = "default_command_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub command_timeout: Duration,
}

#[derive(Debug, Deserialize)]
pub struct HostConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: HostKind,
    pub ssh: Option<SshConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HostKind {
    Local,
    Ssh,
}

#[derive(Debug, Deserialize)]
pub struct SshConfig {
    pub target: Option<String>,
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub config_file: Option<PathBuf>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    pub id: String,
    pub host: String,
    pub tmux: TmuxTarget,
    pub repo: Option<String>,
    pub agent_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmuxTarget {
    pub session: String,
    pub window: Option<u32>,
    pub pane: Option<u32>,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            active_after: default_active_after(),
            idle_after: default_idle_after(),
            capture_lines: default_capture_lines(),
            ssh_timeout: default_ssh_timeout(),
            command_timeout: default_command_timeout(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(path) => expand_home_path(path),
            None => default_config_path()?,
        };
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Config = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn host(&self, id: &str) -> Result<&HostConfig> {
        self.hosts
            .iter()
            .find(|host| host.id == id)
            .ok_or_else(|| anyhow!("unknown host `{id}`"))
    }

    pub fn session(&self, id: &str) -> Result<&SessionConfig> {
        self.sessions
            .iter()
            .find(|session| session.id == id)
            .ok_or_else(|| anyhow!("unknown session `{id}`"))
    }

    pub fn sessions_for_host(&self, host_id: &str) -> Vec<&SessionConfig> {
        self.sessions
            .iter()
            .filter(|session| session.host == host_id)
            .collect()
    }

    fn validate(&self) -> Result<()> {
        if self.poll.active_after.is_zero() {
            bail!("poll.active_after must be greater than zero");
        }
        if self.poll.idle_after < self.poll.active_after {
            bail!("poll.idle_after must be greater than or equal to poll.active_after");
        }
        if self.poll.capture_lines == 0 {
            bail!("poll.capture_lines must be greater than zero");
        }
        if self.poll.ssh_timeout.is_zero() {
            bail!("poll.ssh_timeout must be greater than zero");
        }
        if self.poll.command_timeout.is_zero() {
            bail!("poll.command_timeout must be greater than zero");
        }

        let mut host_ids = HashSet::new();
        for host in &self.hosts {
            if host.id.trim().is_empty() {
                bail!("host id must not be empty");
            }
            if !host_ids.insert(host.id.as_str()) {
                bail!("duplicate host id `{}`", host.id);
            }

            match host.kind {
                HostKind::Local => {
                    if host.ssh.is_some() {
                        bail!("local host `{}` must not include ssh config", host.id);
                    }
                }
                HostKind::Ssh => {
                    let ssh = host.ssh.as_ref().ok_or_else(|| {
                        anyhow!("host `{}` is type ssh but is missing ssh config", host.id)
                    })?;
                    if ssh.target().is_none() {
                        bail!(
                            "host `{}` is type ssh but is missing ssh.target or ssh.host",
                            host.id
                        );
                    }
                }
            }
        }

        let mut session_ids = HashSet::new();
        for session in &self.sessions {
            if session.id.trim().is_empty() {
                bail!("session id must not be empty");
            }
            if !session_ids.insert(session.id.as_str()) {
                bail!("duplicate session id `{}`", session.id);
            }
            if !host_ids.contains(session.host.as_str()) {
                bail!(
                    "session `{}` references missing host `{}`",
                    session.id,
                    session.host
                );
            }
            if session.tmux.session.trim().is_empty() {
                bail!("session `{}` tmux.session must not be empty", session.id);
            }
        }

        Ok(())
    }
}

impl HostConfig {
    pub fn is_local(&self) -> bool {
        self.kind == HostKind::Local
    }

    pub fn ssh(&self) -> Result<&SshConfig> {
        self.ssh
            .as_ref()
            .ok_or_else(|| anyhow!("host `{}` is missing ssh config", self.id))
    }
}

impl SshConfig {
    pub fn target(&self) -> Option<String> {
        if let Some(target) = &self.target {
            return Some(target.clone());
        }

        let host = self.host.as_ref()?;
        Some(match &self.user {
            Some(user) => format!("{user}@{host}"),
            None => host.clone(),
        })
    }

    pub fn ssh_options(&self, default_timeout: Duration) -> BTreeMap<String, String> {
        let timeout = default_timeout.as_secs().max(1).to_string();
        let mut options = BTreeMap::from([
            ("BatchMode".to_string(), "yes".to_string()),
            ("ConnectTimeout".to_string(), timeout),
        ]);
        options.extend(self.options.clone());
        options
    }
}

pub fn expand_home_path(path: &Path) -> PathBuf {
    let display = path.to_string_lossy();
    if display == "~" {
        return std::env::var_os("HOME").map_or_else(|| path.to_path_buf(), PathBuf::from);
    }
    if let Some(rest) = display.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

fn default_config_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config/remux/config.yaml"))
}

fn default_active_after() -> Duration {
    Duration::from_secs(5 * 60)
}

fn default_idle_after() -> Duration {
    Duration::from_secs(60 * 60)
}

fn default_capture_lines() -> usize {
    120
}

fn default_ssh_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_command_timeout() -> Duration {
    Duration::from_secs(15)
}

fn deserialize_duration<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_duration(&value).map_err(serde::de::Error::custom)
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("duration must not be empty".to_string());
    }

    let split_at = value
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split_at);
    let amount: u64 = number
        .parse()
        .map_err(|_| format!("invalid duration amount `{value}`"))?;
    let seconds = match unit {
        "" | "s" => amount,
        "m" => amount * 60,
        "h" => amount * 60 * 60,
        _ => return Err(format!("invalid duration unit `{unit}` in `{value}`")),
    };
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_with_hosts_sessions_and_poll() {
        let config: Config = serde_yaml::from_str(
            r#"
poll:
  active_after: 5m
  idle_after: 60m
  capture_lines: 80
  ssh_timeout: 7s
  command_timeout: 11s
hosts:
  - id: local
    type: local
  - id: pi
    type: ssh
    ssh:
      target: cam@192.168.0.197
sessions:
  - id: agent
    host: pi
    tmux:
      session: work
      window: 0
      pane: 1
    repo: ~/work
    agent_hint: codex
"#,
        )
        .unwrap();

        config.validate().unwrap();
        assert_eq!(config.poll.capture_lines, 80);
        assert_eq!(config.poll.command_timeout, Duration::from_secs(11));
        assert_eq!(
            config.host("pi").unwrap().ssh().unwrap().target().unwrap(),
            "cam@192.168.0.197"
        );
        assert_eq!(
            config.session("agent").unwrap().agent_hint.as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn rejects_duplicate_hosts() {
        let config: Config = serde_yaml::from_str(
            r#"
hosts:
  - id: pi
    type: ssh
    ssh: { target: cam@192.168.0.197 }
  - id: pi
    type: ssh
    ssh: { target: cam@192.168.0.198 }
"#,
        )
        .unwrap();

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate host id")
        );
    }

    #[test]
    fn rejects_duplicate_sessions() {
        let config: Config = serde_yaml::from_str(
            r#"
hosts:
  - id: local
    type: local
sessions:
  - id: agent
    host: local
    tmux: { session: one }
  - id: agent
    host: local
    tmux: { session: two }
"#,
        )
        .unwrap();

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate session id")
        );
    }

    #[test]
    fn rejects_missing_host_reference() {
        let config: Config = serde_yaml::from_str(
            r#"
hosts:
  - id: local
    type: local
sessions:
  - id: agent
    host: missing
    tmux: { session: one }
"#,
        )
        .unwrap();

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("missing host")
        );
    }

    #[test]
    fn parses_duration_units() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }
}
