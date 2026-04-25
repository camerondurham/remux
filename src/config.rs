use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub hosts: Vec<HostConfig>,
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

    fn validate(&self) -> Result<()> {
        let mut host_ids = HashSet::new();
        for host in &self.hosts {
            if host.id.trim().is_empty() {
                bail!("host id must not be empty");
            }
            if !host_ids.insert(host.id.as_str()) {
                bail!("duplicate host id `{}`", host.id);
            }

            match host.kind {
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
        Ok(())
    }
}

impl HostConfig {
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

    pub fn ssh_options(&self) -> BTreeMap<String, String> {
        let mut options = BTreeMap::from([
            ("BatchMode".to_string(), "yes".to_string()),
            ("ConnectTimeout".to_string(), "5".to_string()),
        ]);
        options.extend(self.options.clone());
        options
    }
}

fn default_config_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config/remux/config.yaml"))
}

fn expand_home_path(path: &Path) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pi_config() {
        let config: Config = serde_yaml::from_str(
            r#"
hosts:
  - id: pi
    type: ssh
    ssh:
      target: cam@192.168.0.197
"#,
        )
        .unwrap();

        config.validate().unwrap();
        assert_eq!(
            config.host("pi").unwrap().ssh().unwrap().target().unwrap(),
            "cam@192.168.0.197"
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
}
