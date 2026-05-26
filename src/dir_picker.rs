use crate::config::{Config, HostConfig};
use crate::{host, tmux};
use anyhow::{Context, Result, anyhow, bail};
use std::io::Write;
use std::process::{Command, Stdio};

const PRUNE_DIRS: &[&str] = &[".git", "node_modules", "target", ".cache"];

pub fn pick_directory(config: &Config, host_id: &str) -> Result<Option<String>> {
    let host_config = config.host(host_id)?;
    if fzf_missing()? {
        bail!("fzf is not available");
    }
    if host_config.session_roots.is_empty() {
        bail!("host `{host_id}` has no session_roots configured");
    }

    let rows = directory_rows(config, host_config)?;
    if rows.is_empty() {
        bail!("host `{host_id}` session_roots produced no directories");
    }
    run_fzf(&rows)
}

fn directory_rows(config: &Config, host_config: &HostConfig) -> Result<Vec<String>> {
    let command = directory_scan_command(&host_config.session_roots);
    let output = host::run(config, host_config, &command)
        .with_context(|| format!("failed to list directories on host `{}`", host_config.id))?;
    let mut rows: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    rows.sort();
    rows.dedup();
    Ok(rows)
}

fn directory_scan_command(roots: &[String]) -> String {
    let prune_expr = PRUNE_DIRS
        .iter()
        .map(|name| format!("-name {}", tmux::shell_quote(name)))
        .collect::<Vec<_>>()
        .join(" -o ");
    let roots = roots
        .iter()
        .map(|root| tmux::shell_path(root.trim()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "for _remux_root in {roots}; do [ -d \"$_remux_root\" ] || continue; find \"$_remux_root\" \\( {prune_expr} \\) -prune -o -type d -print; done 2>/dev/null"
    )
}

fn fzf_missing() -> Result<bool> {
    match Command::new("fzf").arg("--version").output() {
        Ok(output) => Ok(!output.status.success()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).context("failed to check fzf availability"),
    }
}

fn run_fzf(rows: &[String]) -> Result<Option<String>> {
    let mut child = Command::new("fzf")
        .arg("--prompt")
        .arg("cwd> ")
        .arg("--height")
        .arg("40%")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start fzf")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
        for row in rows {
            writeln!(stdin, "{row}")?;
        }
    }
    let output = child.wait_with_output().context("failed to wait for fzf")?;
    if !output.status.success() {
        return Ok(None);
    }
    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HostConfig, HostKind, PollConfig, SessionTemplatesConfig};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn directory_scan_command_quotes_roots_and_prunes_noisy_dirs() {
        let command =
            directory_scan_command(&["~/code".to_string(), "/tmp/client work".to_string()]);

        assert!(command.contains("$HOME/'code'"));
        assert!(command.contains("'/tmp/client work'"));
        assert!(command.contains("-name '.git'"));
        assert!(command.contains("-name 'node_modules'"));
        assert!(command.contains("-type d -print"));
    }

    #[test]
    fn directory_rows_scans_configured_roots_and_prunes_noise() {
        let root = unique_temp_dir();
        let project = root.join("project");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(project.join(".git").join("objects")).unwrap();
        fs::create_dir_all(project.join("target").join("debug")).unwrap();

        let config = Config {
            poll: PollConfig::default(),
            session_templates: SessionTemplatesConfig::default(),
            hosts: vec![HostConfig {
                id: "local".to_string(),
                kind: HostKind::Local,
                tmux_socket: None,
                session_roots: vec![root.to_string_lossy().to_string()],
                ssh: None,
            }],
            watches: Vec::new(),
            sessions: Vec::new(),
        };

        let rows = directory_rows(&config, config.host("local").unwrap()).unwrap();
        let project_path = project.to_string_lossy().to_string();

        assert!(rows.contains(&project_path));
        assert!(rows.iter().any(|row| row.ends_with("/project/src")));
        assert!(!rows.iter().any(|row| row.contains("/.git/")));
        assert!(!rows.iter().any(|row| row.contains("/target/")));

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_temp_dir() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("remux-dir-picker-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
