use crate::config::{Config, HostConfig};
use crate::host;
use crate::tmux;
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct RepoSnapshot {
    pub path: String,
    pub branch: Option<String>,
    pub dirty_count: Option<usize>,
    pub changed_files: Vec<String>,
    pub error: Option<String>,
}

pub fn infer(config: &Config, host_config: &HostConfig, cwd: &str) -> Option<RepoSnapshot> {
    if cwd.trim().is_empty() {
        return None;
    }

    let cwd_arg = path_arg(cwd, host_config.is_local());
    let root = run_git(
        config,
        host_config,
        &format!("git -C {cwd_arg} rev-parse --show-toplevel"),
    )
    .ok()?;
    let root = root.trim();
    if root.is_empty() {
        return None;
    }

    Some(collect(config, host_config, root))
}

pub fn collect(config: &Config, host_config: &HostConfig, path: &str) -> RepoSnapshot {
    let expanded_path = expand_repo_path(path, host_config.is_local());
    let path_arg = path_arg(path, host_config.is_local());
    let branch = run_git(
        config,
        host_config,
        &format!("git -C {path_arg} rev-parse --abbrev-ref HEAD"),
    );
    let status = run_git(
        config,
        host_config,
        &format!("git -C {path_arg} status --porcelain=v1"),
    );

    match (branch, status) {
        (Ok(branch), Ok(status)) => {
            let changed_files: Vec<String> = status
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToString::to_string)
                .collect();
            RepoSnapshot {
                path: expanded_path,
                branch: Some(branch.trim().to_string()),
                dirty_count: Some(changed_files.len()),
                changed_files,
                error: None,
            }
        }
        (branch, status) => {
            let error = branch
                .err()
                .or_else(|| status.err())
                .map(|err| format!("{err:#}"));
            RepoSnapshot {
                path: expanded_path,
                branch: None,
                dirty_count: None,
                changed_files: Vec::new(),
                error,
            }
        }
    }
}

fn run_git(config: &Config, host_config: &HostConfig, command: &str) -> Result<String> {
    host::run(config, host_config, command)
}

fn expand_repo_path(path: &str, local: bool) -> String {
    if local {
        return expand_home_path(path).to_string_lossy().into_owned();
    }
    path.to_string()
}

fn path_arg(path: &str, local: bool) -> String {
    if local {
        tmux::shell_quote(&expand_repo_path(path, true))
    } else {
        tmux::shell_path(path)
    }
}

fn expand_home_path(path: &str) -> PathBuf {
    let path = Path::new(path);
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
