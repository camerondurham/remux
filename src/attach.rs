use crate::config::{Config, HostKind, SessionConfig};
use crate::ssh;
use crate::tmux;
use anyhow::{Context, Result};
use std::process::Command;

pub fn attach(config: &Config, session_id: &str, readonly: bool) -> Result<()> {
    let session = config.session(session_id)?;
    let host = config.host(&session.host)?;
    match host.kind {
        HostKind::Local => attach_local(session, readonly),
        HostKind::Ssh => {
            let command = attach_command(session, readonly);
            ssh::run_interactive(host, &command, config.poll.ssh_timeout)
        }
    }
}

fn attach_local(session: &SessionConfig, readonly: bool) -> Result<()> {
    let mut command = Command::new("tmux");
    command.arg("attach-session");
    if readonly {
        command.arg("-r");
    }
    command.arg("-t").arg(&session.tmux.session);
    if let Some(window) = session.tmux.window {
        command
            .arg(";")
            .arg("select-window")
            .arg("-t")
            .arg(window.to_string());
    }
    if let Some(pane) = session.tmux.pane {
        command
            .arg(";")
            .arg("select-pane")
            .arg("-t")
            .arg(pane.to_string());
    }

    let status = command
        .status()
        .with_context(|| format!("failed to start tmux attach for `{}`", session.id))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "tmux attach for `{}` exited with status {status}",
            session.id
        )
    }
}

fn attach_command(session: &SessionConfig, readonly: bool) -> String {
    let mut parts = vec!["tmux attach-session".to_string()];
    if readonly {
        parts.push("-r".to_string());
    }
    parts.push("-t".to_string());
    parts.push(tmux::shell_quote(&session.tmux.session));
    if let Some(window) = session.tmux.window {
        parts.push("\\; select-window -t".to_string());
        parts.push(tmux::shell_quote(&window.to_string()));
    }
    if let Some(pane) = session.tmux.pane {
        parts.push("\\; select-pane -t".to_string());
        parts.push(tmux::shell_quote(&pane.to_string()));
    }
    parts.join(" ")
}
