mod attach;
mod cache;
mod cli;
mod config;
mod doctor;
mod exec;
mod exit;
mod git;
mod host;
mod lifecycle;
mod local;
mod onboard;
mod picker;
mod render;
mod sessions;
mod snapshot;
mod ssh;
mod tmux;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, ListGroup};

fn main() {
    if let Err(err) = run() {
        if let Some(exit) = err.downcast_ref::<exit::ExitFailure>() {
            if let Some(message) = exit.message() {
                eprintln!("error: {message}");
            }
            std::process::exit(exit.code());
        } else {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.clone();
    let verbose = cli.verbose;

    if let Command::Onboard {
        hosts,
        write,
        force,
    } = &cli.command
    {
        return onboard::run(config_path.as_deref(), hosts.as_deref(), *write, *force);
    }

    let config = config::Config::load(cli.config.as_deref())?;

    match cli.command {
        Command::Onboard { .. } => unreachable!(),
        Command::Hosts => render::hosts(&config),
        Command::Doctor { json } => doctor::run(&config, json),
        Command::Snapshot { host, json } => {
            let snapshot = snapshot::snapshot_host(&config, &host)?;
            let unreachable = matches!(&snapshot.status, snapshot::SnapshotStatus::Unreachable);
            render::snapshot(&snapshot, json)?;
            if unreachable {
                anyhow::bail!("failed to poll host `{host}`")
            }
            Ok(())
        }
        Command::List { json, group } => {
            let snapshots = snapshot::snapshot_all(&config)?;
            match group {
                ListGroup::Panes => render::list(&snapshots, json),
                ListGroup::Sessions => render_session_rollups(&snapshots, json),
            }
        }
        Command::Sessions { host, json } => {
            let snapshots = snapshot::snapshot_selected(&config, host.as_deref())?;
            render_session_rollups(&snapshots, json)
        }
        Command::Pick {
            host,
            filter,
            sessions,
            color,
            no_fzf,
        } => picker::run(
            &config,
            config_path.as_deref(),
            picker::PickOptions {
                host,
                filter,
                sessions,
                color,
                no_fzf,
            },
        ),
        Command::Inspect { id, json, color } => {
            let detail = snapshot::inspect_with_color(&config, &id, color && !json)?;
            render::inspect(&detail, json)
        }
        Command::Capture { id, lines, color } => {
            let output = snapshot::capture(&config, &id, lines, color)?;
            render::capture_output(&id, &output, None, color);
            Ok(())
        }
        Command::Attach { readonly, id } => attach::attach(&config, &id, readonly),
        Command::Tui { host, filter } => tui::run(&config, host, filter),
        Command::New {
            host,
            session_name,
            cwd,
            window_name,
        } => lifecycle::new_session(
            &config,
            &host,
            &session_name,
            cwd.as_deref(),
            window_name.as_deref(),
            verbose,
        ),
        Command::Kill { target, yes } => lifecycle::kill(&config, &target, yes, verbose),
    }
}

fn render_session_rollups(snapshots: &[snapshot::HostSnapshot], json: bool) -> Result<()> {
    let rollups = sessions::rollups_from_snapshots(snapshots);
    render::sessions(&rollups, json)?;
    if !json {
        render::warn_snapshot_errors(snapshots);
    }
    Ok(())
}
