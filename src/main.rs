mod cli;
mod config;
mod render;
mod snapshot;
mod ssh;
mod tmux;

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
            render::snapshot(&snapshot, json)
        }
        Command::Inspect { pane_target, json } => {
            let target = tmux::PaneTarget::parse(&pane_target)?;
            let detail = snapshot::inspect_pane(&config, &target)?;
            render::inspect(&detail, json)
        }
        Command::Capture { pane_target, lines } => {
            let target = tmux::PaneTarget::parse(&pane_target)?;
            let output = snapshot::capture_pane(&config, &target, lines)?;
            print!("{output}");
            Ok(())
        }
    }
}
