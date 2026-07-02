use anyhow::{Context, Result, anyhow, bail};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn output(
    command: &mut Command,
    timeout: Duration,
    description: impl AsRef<str>,
) -> Result<Output> {
    let description = description.as_ref().to_string();
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {description}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture stdout for {description}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture stderr for {description}"))?;

    let stdout_handle = thread::spawn(move || read_all(stdout));
    let stderr_handle = thread::spawn(move || read_all(stderr));
    let started = Instant::now();

    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {description}"))?
        {
            let deadline = Instant::now() + Duration::from_millis(500);
            let stdout =
                join_reader_with_deadline(stdout_handle, "stdout", &description, deadline)?;
            let stderr =
                join_reader_with_deadline(stderr_handle, "stderr", &description, deadline)?;
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let deadline = Instant::now() + Duration::from_millis(500);
            let _ = try_join_reader_with_deadline(stdout_handle, deadline);
            let stderr_bytes =
                try_join_reader_with_deadline(stderr_handle, deadline).unwrap_or_default();
            let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
            if stderr.is_empty() {
                bail!("{description} timed out after {}", format_duration(timeout));
            }
            bail!(
                "{description} timed out after {}: {stderr}",
                format_duration(timeout)
            );
        }

        thread::sleep(Duration::from_millis(20));
    }
}

fn read_all(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer)?;
    Ok(buffer)
}

fn join_reader(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
    description: &str,
) -> Result<Vec<u8>> {
    handle
        .join()
        .map_err(|_| anyhow!("failed to join {stream} reader for {description}"))?
        .with_context(|| format!("failed to read {stream} for {description}"))
}

fn join_reader_with_deadline(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
    description: &str,
    deadline: Instant,
) -> Result<Vec<u8>> {
    while Instant::now() < deadline {
        if handle.is_finished() {
            return join_reader(handle, stream, description);
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("{description} exited but {stream} did not close");
}

fn try_join_reader_with_deadline(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    deadline: Instant,
) -> Option<Vec<u8>> {
    while Instant::now() < deadline {
        if handle.is_finished() {
            return handle.join().ok().and_then(|r| r.ok());
        }
        thread::sleep(Duration::from_millis(20));
    }
    // Detach: the reader thread will exit when its pipe closes (or never, if a
    // ControlMaster SSH daemon holds the fd open). One leaked thread is
    // strictly better than blocking the caller forever.
    None
}

fn format_duration(duration: Duration) -> String {
    if duration.as_millis() < 1_000 {
        format!("{}ms", duration.as_millis())
    } else if duration.as_secs_f64().fract() == 0.0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{:.3}s", duration.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_does_not_block_forever_when_descendant_holds_stdout_open() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf ready; sleep 4 &");

        let started = Instant::now();
        let err = output(&mut command, Duration::from_secs(5), "leaky stdout").unwrap_err();

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "output waited for inherited pipe instead of failing fast"
        );
        assert!(format!("{err:#}").contains("stdout did not close"));
    }
}
