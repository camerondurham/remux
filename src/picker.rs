use crate::attach;
use crate::config::Config;
use crate::exit::ExitFailure;
use crate::fzf;
use crate::sessions;
use crate::snapshot::{self, SessionSnapshot};
use crate::tmux::{self, PaneTarget};
use anyhow::{Context, Result, anyhow};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct PickOptions {
    pub host: Option<String>,
    pub filter: Option<String>,
    pub sessions: bool,
    pub color: bool,
    pub no_fzf: bool,
}

pub fn run(config: &Config, config_path: Option<&Path>, options: PickOptions) -> Result<()> {
    let rows = picker_rows(config, options.host.as_deref(), options.sessions)?;

    if options.no_fzf || fzf::is_missing()? {
        print_fzf_remediation();
        print_rows(&rows)?;
        return Err(ExitFailure::quiet(2).into());
    }

    let selected = run_fzf(config_path, &options, &rows)?;
    dispatch_selection(config, selected.as_deref())
}

fn picker_rows(config: &Config, host: Option<&str>, sessions_mode: bool) -> Result<Vec<String>> {
    let snapshots = snapshot::snapshot_selected(config, host)?;

    if sessions_mode {
        return Ok(sessions::rollups_from_snapshots(&snapshots)
            .into_iter()
            .filter(|session| PaneTarget::parse(&session.target).is_ok())
            .map(|session| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    session.target,
                    session.host,
                    session.session,
                    session.state.as_str(),
                    session.match_status.as_str(),
                    session.active_cmd.unwrap_or_else(|| "-".to_string()),
                    session.repo.unwrap_or_else(|| "-".to_string())
                )
            })
            .collect());
    }

    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for snapshot in snapshots {
        for row in snapshot.sessions {
            let Some(target) = row.raw_target.clone() else {
                continue;
            };
            if !seen.insert(target.clone()) {
                continue;
            }
            rows.push(pane_row(&row, &target));
        }
    }
    Ok(rows)
}

fn pane_row(row: &SessionSnapshot, target: &str) -> String {
    let command = row.display_command().unwrap_or_else(|| "-".to_string());
    let cwd = row
        .process
        .as_ref()
        .map(|process| process.cwd.as_str())
        .unwrap_or("-");
    let preview = row
        .output
        .as_ref()
        .map(|output| output.preview.as_str())
        .unwrap_or("-");
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        target,
        row.display_id,
        row.host,
        row.match_status.as_str(),
        row.state.as_str(),
        command,
        compact_field(cwd, preview)
    )
}

fn compact_field(cwd: &str, preview: &str) -> String {
    if preview == "-" {
        cwd.to_string()
    } else {
        format!("{cwd} | {preview}")
    }
}

fn print_fzf_remediation() {
    eprintln!("{}", fzf::INSTALL_HINT);
}

fn print_rows(rows: &[String]) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    for row in rows {
        writeln!(stdout, "{row}")?;
    }
    Ok(())
}

fn run_fzf(
    config_path: Option<&Path>,
    options: &PickOptions,
    rows: &[String],
) -> Result<Option<String>> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let preview = preview_command(&exe, config_path, options.color);
    let reload = reload_command(&exe, config_path, options);

    let mut command = Command::new("fzf");
    command
        .arg("--delimiter")
        .arg("\t")
        .arg("--with-nth")
        .arg("2..")
        .arg("--preview")
        .arg(preview)
        .arg("--preview-window")
        .arg("right:60%:wrap")
        .arg("--expect")
        .arg("enter,ctrl-j,ctrl-o")
        .arg("--bind")
        .arg(format!("ctrl-r:reload({reload})"));
    if options.color {
        command.arg("--ansi");
    }
    if let Some(filter) = options
        .filter
        .as_deref()
        .filter(|filter| !filter.is_empty())
    {
        command.arg("--query").arg(filter);
    }

    let mut child = command
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
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn preview_command(exe: &Path, config_path: Option<&Path>, color: bool) -> String {
    let mut parts = binary_and_config_args(exe, config_path);
    parts.push("capture".to_string());
    parts.push("{1}".to_string());
    parts.push("--lines".to_string());
    parts.push("120".to_string());
    if color {
        parts.push("--color".to_string());
    }
    parts.join(" ")
}

fn reload_command(exe: &Path, config_path: Option<&Path>, options: &PickOptions) -> String {
    let mut parts = binary_and_config_args(exe, config_path);
    parts.push("pick".to_string());
    if let Some(host) = &options.host {
        parts.push("--host".to_string());
        parts.push(tmux::shell_quote(host));
    }
    if options.sessions {
        parts.push("--sessions".to_string());
    }
    if options.color {
        parts.push("--color".to_string());
    }
    parts.push("--no-fzf".to_string());
    format!("{} 2>/dev/null || true", parts.join(" "))
}

fn binary_and_config_args(exe: &Path, config_path: Option<&Path>) -> Vec<String> {
    let mut parts = vec![tmux::shell_quote(&exe.to_string_lossy())];
    if let Some(config_path) = config_path {
        parts.push("--config".to_string());
        parts.push(tmux::shell_quote(&config_path.to_string_lossy()));
    }
    parts
}

fn dispatch_selection(config: &Config, output: Option<&str>) -> Result<()> {
    let Some(output) = output else {
        return Ok(());
    };
    let mut lines = output.lines().filter(|line| !line.is_empty());
    let first = lines.next();
    let Some(first) = first else {
        return Ok(());
    };

    let (key, row) = if is_expected_key(first) {
        (first, lines.next())
    } else {
        ("enter", Some(first))
    };
    let Some(row) = row else {
        return Ok(());
    };
    let target = row
        .split('\t')
        .next()
        .ok_or_else(|| anyhow!("selected picker row was empty"))?;

    match key {
        "enter" => attach::attach(config, target, true),
        "ctrl-j" => attach::attach(config, target, false),
        "ctrl-o" => {
            println!("{target}");
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_expected_key(value: &str) -> bool {
    matches!(value, "enter" | "ctrl-j" | "ctrl-o")
}
