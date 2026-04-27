use crate::config::TmuxTarget;
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use std::fmt;

pub const INVENTORY_COMMAND: &str = "tmux list-panes -a -F '#S\t#I\t#P\t#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}\t#{session_attached}'";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneTarget {
    pub host: String,
    pub session: String,
    pub window: String,
    pub pane: String,
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pane {
    pub target: String,
    pub host: String,
    pub session: String,
    pub window: String,
    pub pane: String,
    pub pane_id: String,
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: String,
    pub session_attached: bool,
}

impl PaneTarget {
    pub fn parse(input: &str) -> Result<Self> {
        let (host, rest) = input.split_once('/').ok_or_else(|| {
            anyhow!("pane target must look like <host>/<session>:<window>.<pane>")
        })?;
        let (session, rest) = rest
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("pane target must include tmux session and window"))?;
        let (window, pane) = rest.split_once('.').ok_or_else(|| {
            anyhow!("pane target must include window and pane as <window>.<pane>")
        })?;

        if host.is_empty() || session.is_empty() || window.is_empty() || pane.is_empty() {
            bail!("pane target must look like <host>/<session>:<window>.<pane>");
        }

        Ok(Self {
            host: host.to_string(),
            session: session.to_string(),
            window: window.to_string(),
            pane: pane.to_string(),
            pane_id: None,
        })
    }

    pub fn tmux_target(&self) -> String {
        format!("{}:{}.{}", self.session, self.window, self.pane)
    }
}

impl fmt::Display for PaneTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}:{}.{}",
            self.host, self.session, self.window, self.pane
        )
    }
}

pub fn parse_inventory(host: &str, output: &str) -> Result<Vec<Pane>> {
    let mut panes = Vec::new();
    for (index, line) in output.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 {
            bail!(
                "failed to parse tmux inventory line {} for host `{}`: expected 8 tab-separated fields, got {}",
                index + 1,
                host,
                fields.len()
            );
        }
        let session = fields[0].to_string();
        let window = fields[1].to_string();
        let pane = fields[2].to_string();
        let target = format!("{host}/{session}:{window}.{pane}");
        panes.push(Pane {
            target,
            host: host.to_string(),
            session,
            window,
            pane,
            pane_id: fields[3].to_string(),
            pid: fields[4].parse().ok(),
            command: fields[5].to_string(),
            cwd: fields[6].to_string(),
            session_attached: parse_tmux_bool(fields[7]),
        });
    }
    Ok(panes)
}

pub fn capture_command(target: &PaneTarget, lines: usize, color: bool) -> String {
    let color_flag = if color { "-e " } else { "" };
    format!(
        "tmux capture-pane {}-pt {} -S -{}",
        color_flag,
        shell_quote(&target.tmux_target()),
        lines
    )
}

pub fn new_session_command(session: &str, cwd: Option<&str>, window_name: Option<&str>) -> String {
    let mut parts = vec!["tmux new-session -d -s".to_string(), shell_quote(session)];
    if let Some(cwd) = cwd {
        parts.push("-c".to_string());
        parts.push(shell_path(cwd));
    }
    if let Some(window_name) = window_name {
        parts.push("-n".to_string());
        parts.push(shell_quote(window_name));
    }
    parts.join(" ")
}

pub fn kill_session_command(session: &str) -> String {
    format!("tmux kill-session -t {}", shell_quote(session))
}

pub fn kill_pane_command(target: &PaneTarget) -> String {
    format!("tmux kill-pane -t {}", shell_quote(&target.tmux_target()))
}

fn parse_tmux_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn shell_path(value: &str) -> String {
    if value == "~" {
        return "$HOME".to_string();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return format!("$HOME/{}", shell_quote(rest));
    }
    shell_quote(value)
}

pub fn matches_target(pane: &Pane, target: &TmuxTarget) -> bool {
    if pane.session != target.session {
        return false;
    }
    if let Some(window) = target.window
        && pane.window != window.to_string()
    {
        return false;
    }
    if let Some(pane_index) = target.pane
        && pane.pane != pane_index.to_string()
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pane_target() {
        let target = PaneTarget::parse("pi/codex:0.1").unwrap();
        assert_eq!(target.host, "pi");
        assert_eq!(target.session, "codex");
        assert_eq!(target.window, "0");
        assert_eq!(target.pane, "1");
        assert_eq!(target.to_string(), "pi/codex:0.1");
    }

    #[test]
    fn parses_tmux_inventory() {
        let output = "codex\t0\t1\t%3\t1234\tzsh\t/home/cam/work\t1\n";
        let panes = parse_inventory("pi", output).unwrap();
        assert_eq!(panes[0].target, "pi/codex:0.1");
        assert_eq!(panes[0].pid, Some(1234));
        assert_eq!(panes[0].command, "zsh");
        assert!(panes[0].session_attached);
    }

    #[test]
    fn capture_command_quotes_target() {
        let target = PaneTarget::parse("pi/codex:0.1").unwrap();
        assert_eq!(
            capture_command(&target, 120, false),
            "tmux capture-pane -pt 'codex:0.1' -S -120"
        );
    }

    #[test]
    fn capture_command_preserves_color_when_requested() {
        let target = PaneTarget::parse("pi/codex:0.1").unwrap();
        assert_eq!(
            capture_command(&target, 20, true),
            "tmux capture-pane -e -pt 'codex:0.1' -S -20"
        );
    }

    #[test]
    fn lifecycle_commands_quote_targets() {
        assert_eq!(
            new_session_command("work", Some("~/repo"), Some("main")),
            "tmux new-session -d -s 'work' -c $HOME/'repo' -n 'main'"
        );
        assert_eq!(kill_session_command("work"), "tmux kill-session -t 'work'");
    }

    #[test]
    fn shell_path_expands_remote_home() {
        assert_eq!(shell_path("~/work/repo"), "$HOME/'work/repo'");
        assert_eq!(shell_path("/tmp/repo"), "'/tmp/repo'");
    }
}
