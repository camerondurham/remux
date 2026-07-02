use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "remux",
    version,
    about = "Inspect tmux panes across local and SSH hosts",
    after_help = "Run `remux` with no arguments to launch the TUI. Use `remux tui` when passing TUI options.\n\nExamples:\n  remux onboard --write\n  remux doctor\n  remux list --group sessions\n  remux inspect 'pi/work:0.1'\n  remux attach --readonly 'pi/work:0.1'\n  remux tui --host pi --filter codex"
)]
pub struct Cli {
    /// Path to a config file.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print resolved lifecycle commands before executing them.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate a starter config and onboarding steps.
    Onboard {
        /// Comma-separated SSH host aliases to include.
        #[arg(long, value_name = "HOST[,HOST...]")]
        hosts: Option<String>,
        /// Write the generated config to disk.
        #[arg(long)]
        write: bool,
        /// Overwrite an existing config file when used with --write.
        #[arg(long)]
        force: bool,
    },
    /// Show configured hosts.
    Hosts,
    /// Validate local tools and host connectivity.
    Doctor {
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// Discover tmux panes on a host.
    Snapshot {
        /// Configured host id.
        host: String,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// List watches and discovered panes across all hosts.
    #[command(
        visible_alias = "ls",
        after_help = "Examples:\n  remux list\n  remux list --group sessions\n  remux list --json"
    )]
    List {
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
        /// Group output by panes or tmux sessions.
        #[arg(long, value_enum, default_value_t = ListGroup::Panes)]
        group: ListGroup,
    },
    /// List one row per tmux session.
    Sessions {
        /// Poll only one configured host.
        #[arg(long)]
        host: Option<String>,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// Pick a pane or session through fzf.
    #[command(
        visible_alias = "p",
        after_help = "Examples:\n  remux pick\n  remux pick --host pi --filter codex\n  remux pick --sessions"
    )]
    Pick {
        /// Poll only one configured host.
        #[arg(long)]
        host: Option<String>,
        /// Initial text filter passed to fzf.
        #[arg(long)]
        filter: Option<String>,
        /// Pick one row per tmux session instead of one row per pane.
        #[arg(long)]
        sessions: bool,
        /// Preserve ANSI escape sequences in preview capture output.
        #[arg(long)]
        color: bool,
        /// Print picker rows without launching fzf.
        #[arg(long)]
        no_fzf: bool,
    },
    /// Inspect one watch id or discovered pane target.
    #[command(
        visible_alias = "i",
        after_help = "Examples:\n  remux inspect codex-agent\n  remux inspect 'pi/work:0.1'\n  remux inspect 'pi/work:0.1' --json"
    )]
    Inspect {
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
        /// Preserve ANSI escape sequences in the human preview block.
        #[arg(long)]
        color: bool,
    },
    /// Capture recent visible output from one watch or pane.
    #[command(
        after_help = "Examples:\n  remux capture codex-agent\n  remux capture 'pi/work:0.1' --lines 200\n  remux capture 'pi/work:0.1' --color"
    )]
    Capture {
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
        /// Number of recent lines to capture.
        #[arg(long, default_value_t = 120, value_parser = parse_positive_usize)]
        lines: usize,
        /// Preserve ANSI escape sequences.
        #[arg(long)]
        color: bool,
    },
    /// Attach interactively to a watch or pane.
    #[command(
        visible_alias = "a",
        after_help = "Examples:\n  remux attach --readonly codex-agent\n  remux attach --readonly 'pi/work:0.1'\n  remux attach 'pi/work:0.1'"
    )]
    Attach {
        /// Attach in tmux read-only mode.
        #[arg(long)]
        readonly: bool,
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
    },
    /// Launch the interactive terminal viewer.
    Tui {
        /// Poll only one configured host.
        #[arg(long)]
        host: Option<String>,
        /// Initial text filter.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Create a detached tmux session on a host.
    New {
        /// Configured host id.
        host: String,
        /// New tmux session name.
        session_name: String,
        /// Initial working directory for the session.
        #[arg(long)]
        cwd: Option<String>,
        /// Initial tmux window name.
        #[arg(long)]
        window_name: Option<String>,
    },
    /// Kill a tmux session or pane.
    Kill {
        /// Watch id, <host>/<session>, or <host>/<session>:<window>.<pane>.
        target: String,
        /// Execute without prompting.
        #[arg(long)]
        yes: bool,
    },
    /// Send literal keys to a tmux session or pane.
    SendKeys {
        /// Watch id, <host>/<session>, or <host>/<session>:<window>.<pane>.
        target: String,
        /// Literal text to send.
        keys: String,
        /// Do not press Enter after the literal text.
        #[arg(long)]
        no_enter: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListGroup {
    Panes,
    Sessions,
}

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|err| format!("invalid positive integer `{value}`: {err}"))?;
    if parsed == 0 {
        return Err("must be greater than zero".to_string());
    }
    Ok(parsed)
}
