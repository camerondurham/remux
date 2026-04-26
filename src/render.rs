use crate::cache::Cache;
use crate::config::{Config, HostKind};
use crate::snapshot::{HostSnapshot, PaneDetail, SessionSnapshot, SnapshotStatus};
use anyhow::Result;

pub fn hosts(config: &Config) -> Result<()> {
    let cache = Cache::load();
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
            println!();
            print_session_table(&snapshot.sessions);
        }
        SnapshotStatus::Unreachable => {
            println!("Host: {}", snapshot.host);
            println!("Status: unreachable");
            for error in &snapshot.errors {
                println!("{}: {}", error.kind, error.message);
            }
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
        "{:<12} {:<22} {:<10} {:<10} {:<18} {:<12} {:<8}",
        "HOST", "SESSION", "PANE", "CMD", "REPO", "STATE", "DIRTY"
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
            println!(
                "{:<12} {:<22} {:<10} {:<10} {:<18} {:<12} {:<8}",
                session.host,
                session.session_id,
                pane,
                command,
                basename(repo),
                session.state.as_str(),
                dirty
            );
        }
    }
    Ok(())
}

pub fn inspect(detail: &PaneDetail, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(detail)?);
        return Ok(());
    }

    let session = &detail.session;
    println!("Session:      {}", session.session_id);
    println!("Target:       {}", session.target);
    println!("Host:         {}", session.host);
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
    println!("  remux capture {} --lines 200", session.session_id);
    if !session.session_id.contains('/') {
        println!("  remux attach --readonly {}", session.session_id);
        println!("  remux attach {}", session.session_id);
    }

    Ok(())
}

fn print_session_table(sessions: &[SessionSnapshot]) {
    println!(
        "{:<24} {:<24} {:<10} {:<8} {:<10} CWD/PREVIEW",
        "SESSION", "TARGET", "PANE_ID", "CMD", "STATE"
    );
    for session in sessions {
        let command = session
            .process
            .as_ref()
            .map(|process| process.command.as_str())
            .unwrap_or("-");
        println!(
            "{:<24} {:<24} {:<10} {:<8} {:<10} {}",
            session.session_id,
            session.target,
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
        for error in &session.errors {
            println!("{:<24} {}: {}", "", error.kind, error.message);
        }
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
}
