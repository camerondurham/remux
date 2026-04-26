use crate::config::{Config, HostKind};
use crate::snapshot;
use crate::ssh;
use crate::tmux::{self, PaneTarget};
use anyhow::{Context, Result};
use std::process::Command;

pub fn attach(config: &Config, id_or_target: &str, readonly: bool) -> Result<()> {
    let target = snapshot::target_for_action(config, id_or_target, "attach")?;
    attach_target(config, &target, readonly)
}

pub fn attach_target(config: &Config, target: &PaneTarget, readonly: bool) -> Result<()> {
    let host = config.host(&target.host)?;
    match host.kind {
        HostKind::Local => attach_local(target, readonly),
        HostKind::Ssh => {
            let command = attach_command(target, readonly);
            ssh::run_interactive(host, &command, config.poll.ssh_timeout)
        }
    }
}

fn attach_local(target: &PaneTarget, readonly: bool) -> Result<()> {
    let mut command = Command::new("tmux");
    command.arg("attach-session");
    if readonly {
        command.arg("-r");
    }
    command.arg("-t").arg(&target.session);
    command
        .arg(";")
        .arg("select-window")
        .arg("-t")
        .arg(&target.window);
    command
        .arg(";")
        .arg("select-pane")
        .arg("-t")
        .arg(&target.pane);

    let status = command
        .status()
        .with_context(|| format!("failed to start tmux attach for `{target}`"))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux attach for `{target}` exited with status {status}")
    }
}

fn attach_command(target: &PaneTarget, readonly: bool) -> String {
    let mut parts = vec!["tmux attach-session".to_string()];
    if readonly {
        parts.push("-r".to_string());
    }
    parts.push("-t".to_string());
    parts.push(tmux::shell_quote(&target.session));
    parts.push("\\; select-window -t".to_string());
    parts.push(tmux::shell_quote(&target.window));
    parts.push("\\; select-pane -t".to_string());
    parts.push(tmux::shell_quote(&target.pane));
    parts.join(" ")
}
