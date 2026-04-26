mod attach;
mod cache;
mod cli;
mod config;
mod exec;
mod git;
mod host;
mod local;
mod render;
mod snapshot;
mod ssh;
mod tmux;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = config::Config::load(cli.config.as_deref())?;

    match cli.command {
        Command::Hosts => render::hosts(&config),
        Command::Snapshot { host, json } => {
            let snapshot = snapshot::snapshot_host(&config, &host)?;
            let unreachable = matches!(&snapshot.status, snapshot::SnapshotStatus::Unreachable);
            render::snapshot(&snapshot, json)?;
            if unreachable {
                anyhow::bail!("failed to poll host `{host}`")
            }
            Ok(())
        }
        Command::List { json } => {
            let snapshots = snapshot::snapshot_all(&config)?;
            render::list(&snapshots, json)
        }
        Command::Inspect { id, json } => {
            let detail = snapshot::inspect(&config, &id)?;
            render::inspect(&detail, json)
        }
        Command::Capture { id, lines } => {
            let output = snapshot::capture(&config, &id, lines)?;
            print!("{output}");
            Ok(())
        }
        Command::Attach { readonly, id } => attach::attach(&config, &id, readonly),
        Command::Tui { host, filter } => tui::run(&config, host, filter),
    }
}
