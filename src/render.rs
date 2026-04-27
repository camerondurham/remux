use crate::cache::Cache;
use crate::config::{Config, HostKind};
use crate::sessions::SessionRollup;
use crate::snapshot::{HostSnapshot, PaneDetail, SessionSnapshot, SnapshotStatus};
use anyhow::Result;

pub fn hosts(config: &Config) -> Result<()> {
    let cache_load = Cache::load_with_warning();
    if let Some(warning) = &cache_load.warning {
        eprintln!("warning: cache: {warning}");
    }
    let cache = cache_load.cache;
    println!(
        "{:<14} {:<6} {:<24} {:<12} LAST POLL",
        "HOST", "TYPE", "TARGET", "STATUS"
    );
    for host in &config.hosts {
        let target = match host.kind {
            HostKind::Local => "-".to_string(),
            HostKind::Ssh => host
                .ssh()
                .ok()
                .and_then(|ssh| ssh.target())
                .unwrap_or_else(|| "-".to_string()),
        };
        let kind = match host.kind {
            HostKind::Local => "local",
            HostKind::Ssh => "ssh",
        };
        let (status, last_poll) = cache
            .hosts
            .get(&host.id)
            .map(|entry| (entry.status.as_str(), entry.last_poll_at.to_rfc3339()))
            .unwrap_or(("-", "-".to_string()));
        println!(
            "{:<14} {:<6} {:<24} {:<12} {}",
            host.id, kind, target, status, last_poll
        );
    }
    Ok(())
}

pub fn snapshot(snapshot: &HostSnapshot, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshot)?);
        return Ok(());
    }

    match snapshot.status {
        SnapshotStatus::Ok => {
            println!("Host: {}", snapshot.host);
            println!("Collected: {}", snapshot.collected_at);
            print_snapshot_errors(snapshot);
            println!();
            print_session_table(&snapshot.sessions);
        }
        SnapshotStatus::Unreachable => {
            println!("Host: {}", snapshot.host);
            println!("Status: unreachable");
            print_snapshot_errors(snapshot);
            if !snapshot.sessions.is_empty() {
                println!();
                print_session_table(&snapshot.sessions);
            }
        }
    }

    Ok(())
}

pub fn list(snapshots: &[HostSnapshot], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshots)?);
        return Ok(());
    }

    println!(
        "{:<12} {:<24} {:<11} {:<10} {:<10} {:<18} {:<12} {:<8} PREVIEW",
        "HOST", "ID/SESSION", "MATCH", "PANE", "CMD", "REPO", "STATE", "DIRTY"
    );
    for snapshot in snapshots {
        for session in &snapshot.sessions {
            let pane = session
                .tmux
                .window
                .as_ref()
                .zip(session.tmux.pane.as_ref())
                .map(|(window, pane)| format!("{window}.{pane}"))
                .unwrap_or_else(|| "-".to_string());
            let command = session
                .process
                .as_ref()
                .map(|process| process.command.as_str())
                .unwrap_or("-");
            let repo = session
                .repo
                .as_ref()
                .map(|repo| repo.path.as_str())
                .unwrap_or("-");
            let dirty = session
                .repo
                .as_ref()
                .and_then(|repo| repo.dirty_count)
                .map(|dirty| dirty.to_string())
                .unwrap_or_else(|| "-".to_string());
            let preview = session
                .output
                .as_ref()
                .map(|output| output.preview.as_str())
                .unwrap_or_else(|| first_error_message(session).unwrap_or("-"));
            println!(
                "{:<12} {:<24} {:<11} {:<10} {:<10} {:<18} {:<12} {:<8} {}",
                session.host,
                session.display_id,
                session.match_status.as_str(),
                pane,
                command,
                basename(repo),
                session.state.as_str(),
                dirty,
                preview
            );
        }
    }
    warn_snapshot_errors(snapshots);
    Ok(())
}

pub fn sessions(sessions: &[SessionRollup], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(sessions)?);
        return Ok(());
    }

    println!(
        "{:<12} {:<24} {:<7} {:<6} {:<8} {:<12} {:<9} {:<12} REPO",
        "HOST", "SESSION", "WINDOWS", "PANES", "ATTACHED", "STATE", "MATCH", "ACTIVE CMD"
    );
    for session in sessions {
        println!(
            "{:<12} {:<24} {:<7} {:<6} {:<8} {:<12} {:<9} {:<12} {}",
            session.host,
            session.session,
            session.windows,
            session.panes,
            session.attached,
            session.state.as_str(),
            session.match_status.as_str(),
            session.active_cmd.as_deref().unwrap_or("-"),
            session.repo.as_deref().map(basename).unwrap_or("-")
        );
    }
    Ok(())
}

pub fn inspect(detail: &PaneDetail, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(detail)?);
        return Ok(());
    }

    let session = &detail.session;
    println!("Session:      {}", session.display_id);
    println!(
        "Target:       {}",
        session.raw_target.as_deref().unwrap_or("-")
    );
    println!("Host:         {}", session.host);
    println!("Match:        {}", session.match_status.as_str());
    if let Some(watch_id) = &session.watch_id {
        println!("Watch:        {watch_id}");
    }
    if let Some(shadowed_by) = &session.shadowed_by {
        println!("Shadowed by:  {shadowed_by}");
    }
    println!("State:        {}", session.state.as_str());
    println!(
        "Tmux target:  {}:{}",
        session.tmux.session,
        session
            .tmux
            .window
            .as_ref()
            .zip(session.tmux.pane.as_ref())
            .map(|(window, pane)| format!("{window}.{pane}"))
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Pane ID:      {}",
        session.tmux.pane_id.as_deref().unwrap_or("-")
    );
    println!(
        "Agent hint:   {}",
        session.agent_hint.as_deref().unwrap_or("-")
    );
    if let Some(process) = &session.process {
        println!(
            "PID:          {}",
            process
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        println!("Command:      {}", process.command);
        println!("CWD:          {}", process.cwd);
    }
    if let Some(repo) = &session.repo {
        println!("Repo:         {}", repo.path);
        println!("Branch:       {}", repo.branch.as_deref().unwrap_or("-"));
        println!(
            "Dirty files:  {}",
            repo.dirty_count
                .map(|dirty| dirty.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        if let Some(error) = &repo.error {
            println!("Repo error:   {error}");
        }
    }
    if let Some(output) = &session.output {
        println!(
            "Last output:  {}",
            output
                .last_output_at
                .map(|timestamp| timestamp.to_rfc3339())
                .unwrap_or_else(|| "-".to_string())
        );
    }
    for error in &session.errors {
        println!("{} error: {}", titlecase(&error.kind), error.message);
    }
    if !session.candidate_targets.is_empty() {
        println!("Candidates:");
        for target in &session.candidate_targets {
            println!("  {target}");
        }
    }
    println!();
    println!("Recent output:");
    println!("{}", detail.recent_output);
    if let Some(repo) = &session.repo
        && !repo.changed_files.is_empty()
    {
        println!("Changed files:");
        for file in &repo.changed_files {
            println!("  {file}");
        }
    }
    println!("Commands:");
    if session.match_status == crate::snapshot::MatchStatus::Matched {
        println!("  remux capture {} --lines 200", session.display_id);
        println!("  remux attach --readonly {}", session.display_id);
        println!("  remux attach {}", session.display_id);
    } else if let Some(raw_target) = &session.raw_target {
        println!("  remux capture '{}' --lines 200", raw_target);
        println!("  remux attach --readonly '{}'", raw_target);
        println!("  remux attach '{}'", raw_target);
    }

    Ok(())
}

fn print_snapshot_errors(snapshot: &HostSnapshot) {
    for error in &snapshot.errors {
        println!("{}: {}", error.kind, error.message);
    }
}

fn warn_snapshot_errors(snapshots: &[HostSnapshot]) {
    for snapshot in snapshots {
        for error in &snapshot.errors {
            eprintln!(
                "warning: {}: {}: {}",
                snapshot.host, error.kind, error.message
            );
        }
    }
}

fn print_session_table(sessions: &[SessionSnapshot]) {
    println!(
        "{:<24} {:<11} {:<24} {:<10} {:<8} {:<10} CWD/PREVIEW",
        "WATCH/SESSION", "MATCH", "TARGET", "PANE_ID", "CMD", "STATE"
    );
    for session in sessions {
        let command = session
            .process
            .as_ref()
            .map(|process| process.command.as_str())
            .unwrap_or("-");
        println!(
            "{:<24} {:<11} {:<24} {:<10} {:<8} {:<10} {}",
            session.display_id,
            session.match_status.as_str(),
            session.raw_target.as_deref().unwrap_or("-"),
            session.tmux.pane_id.as_deref().unwrap_or("-"),
            command,
            session.state.as_str(),
            session
                .process
                .as_ref()
                .map(|process| process.cwd.as_str())
                .unwrap_or("-")
        );
        if let Some(output) = &session.output
            && !output.preview.is_empty()
        {
            println!("{:<24} {}", "", output.preview);
        }
        if !session.candidate_targets.is_empty() {
            println!(
                "{:<24} candidates: {}",
                "",
                session.candidate_targets.join(", ")
            );
        }
        for error in &session.errors {
            println!("{:<24} {}: {}", "", error.kind, error.message);
        }
    }
}

fn titlecase(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
}

fn first_error_message(session: &SessionSnapshot) -> Option<&str> {
    session.errors.first().map(|error| error.message.as_str())
}
