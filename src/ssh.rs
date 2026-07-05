use crate::config::HostConfig;
use crate::exec;
use anyhow::{Context, Result, anyhow};
use std::process::Command;
use std::time::Duration;

pub fn run(
    host: &HostConfig,
    remote_command: &str,
    ssh_timeout: Duration,
    command_timeout: Duration,
) -> Result<String> {
    let mut command = base_command(host, ssh_timeout, false)?;
    append_remote_command(&mut command, host, remote_command)?;
    let output = exec::output(
        &mut command,
        command_timeout,
        format!("ssh command for host `{}`", host.id),
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("ssh exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(anyhow!(
            "failed to run remote command on host `{}`: {message}\nhint: verify `ssh {}` works non-interactively",
            host.id,
            host.ssh()?
                .target()
                .unwrap_or_else(|| "<target>".to_string())
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn run_interactive(
    host: &HostConfig,
    remote_command: &str,
    default_timeout: Duration,
) -> Result<()> {
    let mut command = base_command(host, default_timeout, true)?;
    append_remote_command(&mut command, host, remote_command)?;
    let status = command
        .status()
        .with_context(|| format!("failed to start interactive ssh for host `{}`", host.id))?;

    if status.success() {
        Ok(())
    } else if status.code() == Some(255) {
        Err(anyhow!(
            "ssh connection failed for host `{}` with status {status}",
            host.id
        ))
    } else {
        Err(anyhow!(
            "remote interactive command on host `{}` exited with status {status}",
            host.id
        ))
    }
}

fn base_command(host: &HostConfig, default_timeout: Duration, tty: bool) -> Result<Command> {
    let ssh = host.ssh()?;
    let target = ssh
        .target()
        .ok_or_else(|| anyhow!("host `{}` is missing ssh target", host.id))?;

    let mut command = Command::new("ssh");
    if let Some(config_file) = &ssh.config_file {
        command.arg("-F").arg(config_file);
    }
    let mut options = ssh.ssh_options(default_timeout);
    if tty {
        // A long-lived interactive attach should not reuse a stale multiplexed
        // master connection after sleep/wake. Polling may still opt into muxing
        // for speed, and explicit remux ssh.options can override this default.
        options
            .entry("ControlMaster".to_string())
            .or_insert_with(|| "no".to_string());
    }
    for (key, value) in options {
        command.arg("-o").arg(format!("{key}={value}"));
    }
    if let Some(port) = ssh.port {
        command.arg("-p").arg(port.to_string());
    }
    if tty {
        command.arg("-t");
    }
    command.arg(target);
    Ok(command)
}

fn append_remote_command(
    command: &mut Command,
    host: &HostConfig,
    remote_command: &str,
) -> Result<()> {
    let ssh = host.ssh()?;
    match &ssh.remote_shell {
        Some(shell) if !shell.is_empty() => {
            // SSH concatenates all post-target args with spaces on the remote
            // side, so the shell invocation must be a single ssh argument with
            // the command string properly quoted.
            let mut joined = String::new();
            for (i, part) in shell.iter().enumerate() {
                if i > 0 {
                    joined.push(' ');
                }
                joined.push_str(&shell_single_quote(part));
            }
            joined.push(' ');
            joined.push_str(&shell_single_quote(remote_command));
            command.arg(joined);
        }
        _ => {
            command.arg(remote_command);
        }
    }
    Ok(())
}

/// Wrap `input` in POSIX single quotes, escaping any existing single quotes.
fn shell_single_quote(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('\'');
    for ch in input.chars() {
        if ch == '\'' {
            out.push_str(r"'\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use crate::config::{HostConfig, HostKind, SshConfig};
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::time::Duration;

    #[test]
    fn ssh_options_default_to_non_interactive_timeout() {
        let host = ssh_host(BTreeMap::new());

        let options = host.ssh.unwrap().ssh_options(Duration::from_secs(5));
        assert_eq!(options.get("BatchMode").unwrap(), "yes");
        assert_eq!(options.get("ConnectTimeout").unwrap(), "5");
        assert_eq!(options.get("ServerAliveInterval").unwrap(), "3");
        assert_eq!(options.get("ServerAliveCountMax").unwrap(), "2");
    }

    #[test]
    fn interactive_ssh_disables_control_master_by_default() {
        let host = ssh_host(BTreeMap::new());

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(has_ssh_option(&args, "ControlMaster=no"));
    }

    #[test]
    fn non_interactive_ssh_does_not_disable_control_master_by_default() {
        let host = ssh_host(BTreeMap::new());

        let command = super::base_command(&host, Duration::from_secs(5), false).unwrap();
        let args = command_args(&command);

        assert!(!args.iter().any(|arg| arg.starts_with("ControlMaster=")));
    }

    #[test]
    fn explicit_control_master_option_overrides_interactive_default() {
        let mut options = BTreeMap::new();
        options.insert("ControlMaster".to_string(), "auto".to_string());
        let host = ssh_host(options);

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(has_ssh_option(&args, "ControlMaster=auto"));
        assert!(!has_ssh_option(&args, "ControlMaster=no"));
    }

    fn ssh_host(options: BTreeMap<String, String>) -> HostConfig {
        HostConfig {
            id: "pi".to_string(),
            kind: HostKind::Ssh,
            tmux_socket: None,
            session_roots: Vec::new(),
            ssh: Some(SshConfig {
                target: Some("cam@192.168.0.197".to_string()),
                host: None,
                user: None,
                port: None,
                config_file: None,
                options,
                remote_shell: None,
            }),
        }
    }

    fn command_args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn has_ssh_option(args: &[String], expected: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == "-o" && window[1] == expected)
    }
}
