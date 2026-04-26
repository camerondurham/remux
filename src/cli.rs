use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "remux",
    version,
    about = "Inspect tmux panes across local and SSH hosts"
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show configured hosts.
    Hosts,
    /// Discover tmux panes on a host.
    Snapshot {
        /// Configured host id.
        host: String,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// List watches and discovered panes across all hosts.
    #[command(alias = "ls")]
    List {
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect one watch id or discovered pane target.
    #[command(alias = "i")]
    Inspect {
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// Capture recent visible output from one watch or pane.
    Capture {
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
        /// Number of recent lines to capture.
        #[arg(long, default_value_t = 120)]
        lines: usize,
    },
    /// Attach interactively to a watch or pane.
    #[command(alias = "a")]
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
}
