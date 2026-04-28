use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(name: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{name}-{ts}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_exe(path: &PathBuf, contents: &str) {
    fs::write(path, contents).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn kill_legacy_session_id_targets_tmux_session_not_stale_pane_coordinates() {
    let root = unique_temp_dir("remux-kill-watch");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = root.join("tmux.log");

    write_exe(
        &bin.join("tmux"),
        &format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> '{}'
if [[ "$1" == "list-panes" ]]; then
  printf 'work\t1\t7\t%%42\t1234\tbash\t/tmp\t0\n'
  exit 0
fi
if [[ "$1" == "capture-pane" ]]; then
  printf 'hello\n'
  exit 0
fi
if [[ "$1" == "kill-session" ]]; then
  if [[ "$3" == 'work' ]]; then
    exit 0
  fi
  echo "expected kill-session target work, got $3" >&2
  exit 97
fi
exit 0
"#,
            log.display()
        ),
    );

    let config = root.join("config.yaml");
    fs::write(
        &config,
        r#"
poll:
  capture_lines: 1
hosts:
  - id: local
    type: local
sessions:
  - id: agent
    host: local
    tmux:
      session: work
      window: 0
      pane: 0
"#,
    )
    .unwrap();

    let output = Command::new("cargo")
        .args(["run", "--quiet", "--", "--config"])
        .arg(&config)
        .args(["kill", "agent", "--yes"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log_contents = fs::read_to_string(&log).unwrap();
    assert!(
        log_contents.contains("kill-session -t work")
            || log_contents.contains("kill-session -t 'work'"),
        "log:\n{log_contents}"
    );
    assert!(!log_contents.contains("kill-pane"), "log:\n{log_contents}");
}

#[test]
fn command_match_trims_inventory_command_whitespace() {
    let root = unique_temp_dir("remux-command-trim");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    write_exe(
        &bin.join("tmux"),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "list-panes" ]]; then
  printf 'work\t0\t0\t%%9\t1234\tnode \t/tmp/project\t0\n'
  exit 0
fi
if [[ "$1" == "capture-pane" ]]; then
  printf 'output\n'
  exit 0
fi
exit 0
"#,
    );

    let config = root.join("config.yaml");
    fs::write(
        &config,
        r#"
poll:
  capture_lines: 1
hosts:
  - id: local
    type: local
watches:
  - id: node-agent
    host: local
    match:
      command: node
      cwd_prefix: /tmp/project
"#,
    )
    .unwrap();

    let output = Command::new("cargo")
        .args(["run", "--quiet", "--", "--config"])
        .arg(&config)
        .args(["inspect", "node-agent", "--json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"match_status\": ")
            || stdout.contains("\"match_status\":\"matched\"")
            || stdout.contains("\"match_status\": \"matched\""),
        "stdout:\n{stdout}"
    );
}
