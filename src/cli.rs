use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "remux", version, about = "Inspect tmux panes across SSH hosts")]
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
    /// Inspect one discovered pane target, e.g. pi/codex:0.1.
    Inspect {
        /// Discovered pane target in <host>/<session>:<window>.<pane> form.
        pane_target: String,
        /// Emit structured JSON.
        #[arg(long)]
        json: bool,
    },
    /// Capture recent visible output from one pane.
    Capture {
        /// Discovered pane target in <host>/<session>:<window>.<pane> form.
        pane_target: String,
        /// Number of recent lines to capture.
        #[arg(long, default_value_t = 120)]
        lines: usize,
    },
}
