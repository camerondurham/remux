use crate::config::HostConfig;
use crate::exec;
use anyhow::{Context, Result, anyhow};
use std::collections::BTreeMap;
use std::path::Path;
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
        apply_interactive_options(&mut options, ssh.config_file.as_deref(), ssh.port, &target);
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

fn apply_interactive_options(
    options: &mut BTreeMap<String, String>,
    config_file: Option<&Path>,
    port: Option<u16>,
    target: &str,
) {
    // A long-lived interactive attach should not reuse a stale multiplexed
    // connection after sleep/wake. Polling may still opt into muxing for speed,
    // and explicit remux ssh.options can override these defaults.
    ensure_option(options, "ControlMaster", "no");
    ensure_option(options, "ControlPath", "none");
    disable_proxyjump_muxing(options, config_file, port, target);
}

fn ensure_option(options: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if !contains_option(options, key) {
        options.insert(key.to_string(), value.to_string());
    }
}

fn contains_option(options: &BTreeMap<String, String>, key: &str) -> bool {
    option_value(options, key).is_some()
}

fn option_value<'a>(options: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|(option_key, _)| option_key.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

fn remove_option(options: &mut BTreeMap<String, String>, key: &str) -> Option<String> {
    let existing = options
        .keys()
        .find(|option_key| option_key.eq_ignore_ascii_case(key))
        .cloned()?;
    options.remove(&existing)
}

fn disable_proxyjump_muxing(
    options: &mut BTreeMap<String, String>,
    config_file: Option<&Path>,
    port: Option<u16>,
    target: &str,
) {
    if contains_option(options, "ProxyCommand") {
        return;
    }
    let explicit_proxy_jump = option_value(options, "ProxyJump").map(str::to_string);
    let proxy_jump = explicit_proxy_jump
        .or_else(|| resolved_proxy_jump_from_ssh_config(config_file, port, target, options));
    let Some(proxy_jump) = proxy_jump else {
        return;
    };
    let Some(proxy_command) = proxyjump_proxy_command(&proxy_jump, options, config_file) else {
        return;
    };
    remove_option(options, "ProxyJump");
    options.insert("ProxyCommand".to_string(), proxy_command);
}

fn proxyjump_proxy_command(
    proxy_jump: &str,
    options: &BTreeMap<String, String>,
    config_file: Option<&Path>,
) -> Option<String> {
    if proxy_jump.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut jumps: Vec<&str> = proxy_jump
        .split(',')
        .map(str::trim)
        .filter(|jump| !jump.is_empty())
        .collect();
    let destination = jumps.pop()?;

    let mut parts = vec!["ssh".to_string()];
    if let Some(config_file) = config_file {
        parts.push("-F".to_string());
        parts.push(shell_single_quote(&config_file.to_string_lossy()));
    }
    for key in [
        "BatchMode",
        "ConnectTimeout",
        "ServerAliveInterval",
        "ServerAliveCountMax",
        "ControlMaster",
        "ControlPath",
    ] {
        if let Some(value) = option_value(options, key) {
            parts.push("-o".to_string());
            parts.push(shell_single_quote(&format!("{key}={value}")));
        }
    }
    if !jumps.is_empty() {
        parts.push("-J".to_string());
        parts.push(shell_single_quote(&jumps.join(",")));
    }
    parts.push("-W".to_string());
    parts.push(shell_single_quote("%h:%p"));
    parts.push(shell_single_quote(destination));
    Some(parts.join(" "))
}

fn resolved_proxy_jump_from_ssh_config(
    config_file: Option<&Path>,
    port: Option<u16>,
    target: &str,
    options: &BTreeMap<String, String>,
) -> Option<String> {
    let mut command = Command::new("ssh");
    command.arg("-G");
    if let Some(config_file) = config_file {
        command.arg("-F").arg(config_file);
    }
    for (key, value) in options {
        if key.eq_ignore_ascii_case("ProxyCommand") || key.eq_ignore_ascii_case("ProxyJump") {
            continue;
        }
        command.arg("-o").arg(format!("{key}={value}"));
    }
    if let Some(port) = port {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(target);

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ssh_g_proxy(&String::from_utf8_lossy(&output.stdout)).proxy_jump
}

#[derive(Default)]
struct SshConfigProxy {
    proxy_jump: Option<String>,
    proxy_command: Option<String>,
}

fn parse_ssh_g_proxy(raw: &str) -> SshConfigProxy {
    let mut proxy = SshConfigProxy::default();
    for line in raw.lines() {
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        if value.eq_ignore_ascii_case("none") {
            continue;
        }
        if key.eq_ignore_ascii_case("proxyjump") {
            proxy.proxy_jump = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("proxycommand") {
            proxy.proxy_command = Some(value.to_string());
        }
    }
    if proxy.proxy_command.is_some() {
        proxy.proxy_jump = None;
    }
    proxy
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
    use std::path::PathBuf;
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
    fn interactive_ssh_disables_multiplexing_by_default() {
        let host = ssh_host(BTreeMap::new());

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(has_ssh_option(&args, "ControlMaster=no"));
        assert!(has_ssh_option(&args, "ControlPath=none"));
    }

    #[test]
    fn non_interactive_ssh_does_not_disable_multiplexing_by_default() {
        let host = ssh_host(BTreeMap::new());

        let command = super::base_command(&host, Duration::from_secs(5), false).unwrap();
        let args = command_args(&command);

        assert!(!args.iter().any(|arg| arg.starts_with("ControlMaster=")));
        assert!(!args.iter().any(|arg| arg.starts_with("ControlPath=")));
    }

    #[test]
    fn explicit_mux_options_override_interactive_defaults_case_insensitively() {
        let mut options = BTreeMap::new();
        options.insert("controlmaster".to_string(), "auto".to_string());
        options.insert("Controlpath".to_string(), "/tmp/remux-ctl".to_string());
        let host = ssh_host(options);

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(has_ssh_option(&args, "controlmaster=auto"));
        assert!(has_ssh_option(&args, "Controlpath=/tmp/remux-ctl"));
        assert!(!has_ssh_option(&args, "ControlMaster=no"));
        assert!(!has_ssh_option(&args, "ControlPath=none"));
    }

    #[test]
    fn interactive_proxyjump_uses_proxycommand_with_mux_disabled() {
        let mut options = BTreeMap::new();
        options.insert("ProxyJump".to_string(), "jump".to_string());
        let host = ssh_host(options);

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(!args.iter().any(|arg| arg.starts_with("ProxyJump=")));
        let proxy_command = ssh_option(&args, "ProxyCommand").expect("ProxyCommand");
        assert!(proxy_command.contains("'ControlMaster=no'"));
        assert!(proxy_command.contains("'ControlPath=none'"));
        assert!(proxy_command.contains("-W '%h:%p' 'jump'"));
    }

    #[test]
    fn explicit_proxycommand_overrides_interactive_proxyjump_conversion() {
        let mut options = BTreeMap::new();
        options.insert("ProxyCommand".to_string(), "custom proxy".to_string());
        options.insert("ProxyJump".to_string(), "jump".to_string());
        let host = ssh_host(options);

        let command = super::base_command(&host, Duration::from_secs(5), true).unwrap();
        let args = command_args(&command);

        assert!(has_ssh_option(&args, "ProxyCommand=custom proxy"));
        assert!(has_ssh_option(&args, "ProxyJump=jump"));
    }

    #[test]
    fn parses_proxyjump_from_ssh_g_output() {
        let proxy =
            super::parse_ssh_g_proxy("hostname final.example\nproxyjump jump\nproxycommand none\n");

        assert_eq!(proxy.proxy_jump.as_deref(), Some("jump"));
        assert_eq!(proxy.proxy_command, None);
    }

    #[test]
    fn ssh_g_proxycommand_takes_precedence_over_proxyjump() {
        let proxy =
            super::parse_ssh_g_proxy("proxyjump jump\nproxycommand ssh -W final.example:22 jump\n");

        assert_eq!(proxy.proxy_jump, None);
        assert_eq!(
            proxy.proxy_command.as_deref(),
            Some("ssh -W final.example:22 jump")
        );
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
                config_file: Some(PathBuf::from("/dev/null")),
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

    fn ssh_option<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.windows(2)
            .filter(|window| window[0] == "-o")
            .filter_map(|window| window[1].split_once('='))
            .find(|(option_key, _)| option_key.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }
}
