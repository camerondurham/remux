use clap::{Parser, Subcommand, ValueEnum};
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

    /// Print resolved lifecycle commands before executing them.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
    #[command(alias = "ls")]
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
    #[command(alias = "p")]
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
    #[command(alias = "i")]
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
    Capture {
        /// Watch id or <host>/<session>:<window>.<pane> target.
        id: String,
        /// Number of recent lines to capture.
        #[arg(long, default_value_t = 120)]
        lines: usize,
        /// Preserve ANSI escape sequences.
        #[arg(long)]
        color: bool,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListGroup {
    Panes,
    Sessions,
}
