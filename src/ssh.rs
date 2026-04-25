use crate::config::HostConfig;
use anyhow::{Context, Result, anyhow};
use std::process::Command;

pub fn run(host: &HostConfig, remote_command: &str) -> Result<String> {
    let ssh = host.ssh()?;
    let target = ssh
        .target()
        .ok_or_else(|| anyhow!("host `{}` is missing ssh target", host.id))?;

    let mut command = Command::new("ssh");
    if let Some(config_file) = &ssh.config_file {
        command.arg("-F").arg(config_file);
    }
    for (key, value) in ssh.ssh_options() {
        command.arg("-o").arg(format!("{key}={value}"));
    }
    if let Some(port) = ssh.port {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(target).arg(remote_command);

    let output = command
        .output()
        .with_context(|| format!("failed to start ssh for host `{}`", host.id))?;

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
            ssh.target().unwrap_or_else(|| "<target>".to_string())
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use crate::config::{HostConfig, HostKind, SshConfig};
    use std::collections::BTreeMap;

    #[test]
    fn ssh_options_default_to_non_interactive_timeout() {
        let host = HostConfig {
            id: "pi".to_string(),
            kind: HostKind::Ssh,
            ssh: Some(SshConfig {
                target: Some("cam@192.168.0.197".to_string()),
                host: None,
                user: None,
                port: None,
                config_file: None,
                options: BTreeMap::new(),
            }),
        };

        let options = host.ssh.unwrap().ssh_options();
        assert_eq!(options.get("BatchMode").unwrap(), "yes");
        assert_eq!(options.get("ConnectTimeout").unwrap(), "5");
    }
}
