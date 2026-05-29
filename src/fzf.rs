use anyhow::{Context, Result};
use std::process::Command;

pub const INSTALL_HINT: &str = "fzf is not available; install it with one of:\n  macOS:  brew install fzf\n  Debian: sudo apt-get install fzf\n  Arch:   sudo pacman -S fzf";

pub fn is_missing() -> Result<bool> {
    match Command::new("fzf").arg("--version").output() {
        Ok(output) => Ok(!output.status.success()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).context("failed to check fzf availability"),
    }
}
