use crate::config::{Config, HostKind};
use crate::snapshot::{HostSnapshot, PaneDetail, SnapshotStatus};
use anyhow::Result;

pub fn hosts(config: &Config) -> Result<()> {
    println!("{:<14} {:<6} TARGET", "HOST", "TYPE");
    for host in &config.hosts {
        match host.kind {
            HostKind::Ssh => {
                let target = host
                    .ssh()
                    .ok()
                    .and_then(|ssh| ssh.target())
                    .unwrap_or_else(|| "-".to_string());
                println!("{:<14} {:<6} {}", host.id, "ssh", target);
            }
        }
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
            println!(
                "{:<28} {:<10} {:<8} {:<8} CWD",
                "TARGET", "PANE_ID", "PID", "CMD"
            );
            for pane in &snapshot.panes {
                println!(
                    "{:<28} {:<10} {:<8} {:<8} {}",
                    pane.pane.target,
                    pane.pane.pane_id,
                    pane.pane
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    pane.pane.command,
                    pane.pane.cwd
                );
                if !pane.output.preview.is_empty() {
                    println!("{:<28} {}", "", pane.output.preview);
                }
            }
        }
        SnapshotStatus::Unreachable => {
            println!("Host: {}", snapshot.host);
            println!("Status: unreachable");
            for error in &snapshot.errors {
                println!("{}: {}", error.kind, error.message);
            }
        }
    }

    Ok(())
}

pub fn inspect(detail: &PaneDetail, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(detail)?);
        return Ok(());
    }

    println!("Target:       {}", detail.pane.target);
    println!("Host:         {}", detail.pane.host);
    println!(
        "Tmux target:  {}:{}.{}",
        detail.pane.session, detail.pane.window, detail.pane.pane
    );
    println!("Pane ID:      {}", detail.pane.pane_id);
    println!(
        "PID:          {}",
        detail
            .pane
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!("Command:      {}", detail.pane.command);
    println!("CWD:          {}", detail.pane.cwd);
    println!("Output hash:  {}", detail.output_hash);
    println!();
    println!("Recent output:");
    println!("{}", detail.output);

    Ok(())
}
