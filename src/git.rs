use crate::config::{Config, HostConfig};
use crate::host;
use crate::tmux;
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

#[allow(dead_code)]
pub fn infer(config: &Config, host_config: &HostConfig, cwd: &str) -> Option<RepoSnapshot> {
    if cwd.trim().is_empty() {
        return None;
    }

    let cwd_arg = path_arg(cwd, host_config.is_local());
    let root = host::run(
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
    let branch = host::run(
        config,
        host_config,
        &format!("git -C {path_arg} rev-parse --abbrev-ref HEAD"),
    );
    let status = host::run(
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

/// Parse the git block emitted by the combined SSH command.
/// Lines: toplevel, branch, dirty_count (from `wc -l`).
/// Returns `None` if toplevel is empty (cwd not in a repo).
/// `changed_files` is left empty — the batched path only collects dirty count.
pub fn parse_git_block(lines: &[&str]) -> Option<RepoSnapshot> {
    let toplevel = lines.first().map(|s| s.trim()).unwrap_or("").to_string();
    if toplevel.is_empty() {
        return None;
    }
    let branch = lines
        .get(1)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dirty_count = lines.get(2).and_then(|s| s.trim().parse::<usize>().ok());
    Some(RepoSnapshot {
        path: toplevel,
        branch,
        dirty_count,
        changed_files: vec![],
        error: None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_block_happy_path() {
        let lines = ["/home/cam/work", "main", "3"];
        let snap = parse_git_block(&lines).unwrap();
        assert_eq!(snap.path, "/home/cam/work");
        assert_eq!(snap.branch, Some("main".to_string()));
        assert_eq!(snap.dirty_count, Some(3));
        assert!(snap.changed_files.is_empty());
        assert!(snap.error.is_none());
    }

    #[test]
    fn parse_git_block_empty_toplevel_returns_none() {
        let lines = ["", "main", "0"];
        assert!(parse_git_block(&lines).is_none());
    }

    #[test]
    fn parse_git_block_no_lines_returns_none() {
        assert!(parse_git_block(&[]).is_none());
    }

    #[test]
    fn parse_git_block_malformed_dirty_count() {
        let lines = ["/repo", "main", "notanumber"];
        let snap = parse_git_block(&lines).unwrap();
        assert_eq!(snap.dirty_count, None);
    }

    #[test]
    fn parse_git_block_missing_branch() {
        let lines = ["/repo", "", "0"];
        let snap = parse_git_block(&lines).unwrap();
        assert_eq!(snap.branch, None);
        assert_eq!(snap.dirty_count, Some(0));
    }
}
