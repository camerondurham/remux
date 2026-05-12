use crate::config;
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

pub fn run(
    config_path_arg: Option<&Path>,
    hosts_arg: Option<&str>,
    write: bool,
    force: bool,
) -> Result<()> {
    let config_path = config::resolve_config_path(config_path_arg)?;
    let discovered = discover_ssh_aliases()?;
    let selection = choose_hosts(hosts_arg, &discovered)?;
    let rendered = render_config(&selection.hosts);

    if write {
        write_config(&config_path, &rendered, force)?;
        print_written_summary(&config_path, &selection.hosts, discovered.is_empty());
        return Ok(());
    }

    print_preview(&config_path, &rendered, &discovered, &selection.hosts, selection.mode);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
    Explicit,
    Interactive,
    AutoAll,
    None,
}

#[derive(Debug)]
struct HostSelection {
    hosts: Vec<String>,
    mode: SelectionMode,
}

fn choose_hosts(hosts_arg: Option<&str>, discovered: &[String]) -> Result<HostSelection> {
    if let Some(raw) = hosts_arg {
        return Ok(HostSelection {
            hosts: parse_hosts_arg(raw),
            mode: SelectionMode::Explicit,
        });
    }

    if discovered.is_empty() {
        return Ok(HostSelection {
            hosts: Vec::new(),
            mode: SelectionMode::None,
        });
    }

    if should_prompt_interactively() {
        return Ok(HostSelection {
            hosts: prompt_for_hosts(discovered)?,
            mode: SelectionMode::Interactive,
        });
    }

    Ok(HostSelection {
        hosts: discovered.to_vec(),
        mode: SelectionMode::AutoAll,
    })
}

fn should_prompt_interactively() -> bool {
    if std::env::var_os("REMUX_ONBOARD_FORCE_INTERACTIVE").is_some() {
        return true;
    }
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_for_hosts(discovered: &[String]) -> Result<Vec<String>> {
    println!("Discovered SSH aliases from ~/.ssh/config:");
    for (idx, host) in discovered.iter().enumerate() {
        println!("  {}) {}", idx + 1, host);
    }
    println!();
    println!("Select hosts to include in remux.");
    println!("- Press Enter to include all discovered hosts");
    println!("- Type numbers like `1,3` to include specific hosts");
    println!("- Type `none` to generate a local-only config");
    print!("> ");
    io::stdout().flush().context("failed to flush prompt")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read host selection")?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Ok(discovered.to_vec());
    }
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for token in trimmed.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let index: usize = token
            .parse()
            .with_context(|| format!("invalid selection `{token}`; expected numbers like 1,3"))?;
        if index == 0 || index > discovered.len() {
            bail!(
                "selection `{token}` is out of range; choose between 1 and {}",
                discovered.len()
            );
        }
        let host = discovered[index - 1].clone();
        if seen.insert(host.clone()) {
            selected.push(host);
        }
    }
    Ok(selected)
}

fn print_preview(
    config_path: &Path,
    rendered: &str,
    discovered: &[String],
    selected: &[String],
    mode: SelectionMode,
) {
    println!("remux onboard");
    println!("config path: {}", config_path.display());
    println!();

    match mode {
        SelectionMode::Explicit => {
            println!("Using selected SSH aliases:");
            for host in selected {
                println!("- {host}");
            }
        }
        SelectionMode::Interactive => {
            if selected.is_empty() {
                println!("Selected SSH aliases: none");
                println!("Previewing a local-only starter config.");
            } else {
                println!("Selected SSH aliases:");
                for host in selected {
                    println!("- {host}");
                }
            }
        }
        SelectionMode::AutoAll => {
            println!("Discovered SSH aliases from ~/.ssh/config:");
            for host in discovered {
                println!("- {host}");
            }
            println!();
            println!("Previewing a starter config with `local` plus all discovered aliases.");
        }
        SelectionMode::None => {
            println!("No SSH aliases were discovered from ~/.ssh/config.");
            println!(
                "The starter config below includes only `local`. Add SSH hosts later by editing the file."
            );
        }
    }

    println!();
    println!("---");
    println!("{rendered}");
    println!("---");
    println!();

    if mode == SelectionMode::AutoAll {
        println!("To limit the generated SSH hosts:");
        println!("  remux onboard --hosts {}", discovered.join(","));
        println!();
    }

    println!("To write this config:");
    println!("  remux onboard --write");
    if !selected.is_empty() || !discovered.is_empty() {
        let host_list = if selected.is_empty() {
            discovered.join(",")
        } else {
            selected.join(",")
        };
        if !host_list.is_empty() {
            println!("  remux onboard --hosts {host_list} --write");
        }
    }
    println!();
    println!("Then run:");
    println!("  remux doctor");
    println!("  remux list");
    println!("  remux tui");
}

fn print_written_summary(config_path: &Path, selected: &[String], no_discovered_hosts: bool) {
    println!("remux onboard");
    println!("wrote {}", config_path.display());
    if selected.is_empty() {
        if no_discovered_hosts {
            println!("configured hosts: local");
        } else {
            println!("configured hosts: local only");
        }
    } else {
        println!("configured hosts: local, {}", selected.join(", "));
    }
    println!();
    println!("Next:");
    println!("  remux doctor");
    println!("  remux list");
    println!("  remux tui");
}

fn parse_hosts_arg(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn write_config(path: &Path, rendered: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "config already exists at {}; rerun with --force to overwrite",
            path.display()
        );
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path {} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn render_config(hosts: &[String]) -> String {
    let mut out = String::from(
        "poll:\n  active_after: 5m\n  idle_after: 60m\n  capture_lines: 120\n  ssh_timeout: 5s\n  command_timeout: 15s\n\nhosts:\n  - id: local\n    type: local\n",
    );

    for host in hosts {
        out.push_str(&format!(
            "\n  - id: {}\n    type: ssh\n    ssh:\n      target: {}\n",
            sanitize_id(host),
            host
        ));
    }

    out.push_str(
        "\n# Optional: give important panes stable IDs you can inspect/attach quickly.\n# watches:\n#   - id: prod-api\n#     host: prod\n#     match:\n#       command: node\n#       cwd_prefix: /srv/api\n#     repo: /srv/api\n#     agent_hint: codex\n",
    );

    out
}

fn sanitize_id(host: &str) -> String {
    let mut out = String::with_capacity(host.len());
    for ch in host.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn discover_ssh_aliases() -> Result<Vec<String>> {
    let home = match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home),
        None => return Ok(Vec::new()),
    };
    let path = home.join(".ssh/config");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_ssh_aliases(&raw))
}

fn parse_ssh_aliases(raw: &str) -> Vec<String> {
    let mut aliases = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for part in parts {
            if part.contains('*') || part.contains('?') || part.contains('!') {
                continue;
            }
            aliases.insert(part.to_string());
        }
    }
    aliases.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_ssh_aliases, parse_hosts_arg, render_config, sanitize_id};

    #[test]
    fn parses_host_aliases_and_ignores_wildcards() {
        let raw = r#"
Host pi prod
  User cam

Host *
  ServerAliveInterval 60

Host dev-?
  User builder

Host github.com
"#;
        assert_eq!(parse_ssh_aliases(raw), vec!["github.com", "pi", "prod"]);
    }

    #[test]
    fn render_config_includes_local_and_selected_hosts() {
        let rendered = render_config(&["pi".to_string(), "prod-box".to_string()]);
        assert!(rendered.contains("- id: local"));
        assert!(rendered.contains("- id: pi"));
        assert!(rendered.contains("target: pi"));
        assert!(rendered.contains("- id: prod-box"));
        assert!(rendered.contains("target: prod-box"));
        assert!(rendered.contains("# watches:"));
    }

    #[test]
    fn sanitize_id_normalizes_non_identifier_characters() {
        assert_eq!(sanitize_id("github.com"), "github-com");
        assert_eq!(sanitize_id("Prod_Box"), "prod_box");
    }

    #[test]
    fn parses_hosts_arg() {
        assert_eq!(parse_hosts_arg("pi, prod,,"), vec!["pi", "prod"]);
    }
}
