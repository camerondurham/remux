use anyhow::{Context, Result, anyhow};
use std::process::Command;

pub fn run(command: &str) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .with_context(|| format!("failed to start local command `{command}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("local command exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(anyhow!("failed to run local command: {message}"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
