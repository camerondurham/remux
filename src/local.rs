use crate::exec;
use anyhow::{Context, Result, anyhow};
use std::process::Command;
use std::time::Duration;

pub fn run(command: &str, timeout: Duration) -> Result<String> {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    let output = exec::output(&mut process, timeout, format!("local command `{command}`"))
        .with_context(|| format!("failed to run local command `{command}`"))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_commands_time_out() {
        let error = run("sleep 1", Duration::from_millis(20)).unwrap_err();
        assert!(format!("{error:#}").contains("timed out"));
    }
}
