mod attach;
mod cache;
mod cli;
mod config;
mod dir_picker;
mod doctor;
mod exec;
mod exit;
mod fzf;
mod git;
mod host;
mod launch_template;
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
use clap::{CommandFactory, Parser, error::ErrorKind};
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
    let args_were_provided = std::env::args_os().nth(1).is_some();
    let cli = Cli::parse();
    run_cli(cli, args_were_provided)
}

fn run_cli(cli: Cli, args_were_provided: bool) -> Result<()> {
    let config_path = cli.config.clone();
    let verbose = cli.verbose;
    let command = resolve_command(cli.command, args_were_provided).unwrap_or_else(|err| err.exit());

    if let Command::Onboard {
        hosts,
        write,
        force,
    } = &command
    {
        return onboard::run(config_path.as_deref(), hosts.as_deref(), *write, *force);
    }

    let config = config::Config::load(cli.config.as_deref())?;

    match command {
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
        Command::Start {
            host,
            preset,
            name,
            cwd,
            window_name,
            no_send,
        } => {
            lifecycle::start_launch_template(
                &config,
                &host,
                &preset,
                &name,
                lifecycle::LaunchTemplateStartOptions {
                    cwd: cwd.as_deref(),
                    window_name: window_name.as_deref(),
                    send_startup_keys: !no_send,
                    verbose,
                },
            )?;
            Ok(())
        }
        Command::Kill { target, yes } => lifecycle::kill(&config, &target, yes, verbose),
        Command::SendKeys {
            target,
            keys,
            no_enter,
        } => lifecycle::send_keys(&config, &target, &keys, !no_enter, verbose),
    }
}

fn resolve_command(
    command: Option<Command>,
    args_were_provided: bool,
) -> std::result::Result<Command, clap::Error> {
    match command {
        Some(command) => Ok(command),
        None if !args_were_provided => Ok(Command::Tui {
            host: None,
            filter: None,
        }),
        None => Err(Cli::command().error(
            ErrorKind::MissingSubcommand,
            "a subcommand is required when options are provided; to launch the TUI with options, use `remux tui`",
        )),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_cli_defaults_to_tui() {
        let cli = Cli::try_parse_from(["remux"]).unwrap();

        let command = resolve_command(cli.command, false).unwrap();

        assert!(matches!(
            command,
            Command::Tui {
                host: None,
                filter: None
            }
        ));
    }

    #[test]
    fn global_options_without_subcommand_do_not_default_to_tui() {
        let cli = Cli::try_parse_from(["remux", "--config", "config.yaml"]).unwrap();

        let err = resolve_command(cli.command, true).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::MissingSubcommand);
        assert!(
            err.to_string().contains("use `remux tui`"),
            "expected TUI hint in error: {err}"
        );
    }

    #[test]
    fn help_shows_documented_aliases_and_config_hint() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();

        assert!(help.contains("--config <PATH>"));
        assert!(help.contains("Path to a config file"));
        assert!(help.contains("list"));
        assert!(help.contains("ls"));
        assert!(help.contains("inspect 'pi/work:0.1'"));
    }

    #[test]
    fn capture_rejects_zero_lines_at_parse_time() {
        let err =
            Cli::try_parse_from(["remux", "capture", "codex-agent", "--lines", "0"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(
            err.to_string().contains("must be greater than zero"),
            "expected positive range hint in error: {err}"
        );
    }

    #[test]
    fn explicit_tui_subcommand_still_accepts_tui_options() {
        let cli =
            Cli::try_parse_from(["remux", "tui", "--host", "pi", "--filter", "codex"]).unwrap();

        let command = resolve_command(cli.command, true).unwrap();

        assert!(matches!(
            command,
            Command::Tui {
                host: Some(host),
                filter: Some(filter)
            } if host == "pi" && filter == "codex"
        ));
    }

    #[test]
    fn explicit_list_subcommand_stays_non_tui() {
        let cli = Cli::try_parse_from(["remux", "list", "--json"]).unwrap();

        let command = resolve_command(cli.command, true).unwrap();

        assert!(matches!(
            command,
            Command::List {
                json: true,
                group: ListGroup::Panes
            }
        ));
    }
}
