use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn ssh_tracer_bullet_works_end_to_end() {
    let env = TestEnv::new();

    let hosts = env.remux(["--config", env.config_path(), "hosts"]);
    assert_success(&hosts);
    let hosts_stdout = stdout(&hosts);
    assert!(hosts_stdout.contains("pi"));
    assert!(hosts_stdout.contains("fake-pi"));

    let snapshot = env.remux(["--config", env.config_path(), "snapshot", "pi"]);
    assert_success(&snapshot);
    let snapshot_stdout = stdout(&snapshot);
    assert!(snapshot_stdout.contains("pi/work:0.1"));
    assert!(snapshot_stdout.contains("node"));
    assert!(snapshot_stdout.contains("/home/cam/work"));
    assert!(snapshot_stdout.contains("hello-remux"));

    let snapshot_json = env.remux(["--config", env.config_path(), "snapshot", "pi", "--json"]);
    assert_success(&snapshot_json);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot_json)).unwrap();
    assert_eq!(snapshot["host"], "pi");
    assert_eq!(snapshot["status"], "ok");
    assert_eq!(snapshot["panes"][0]["target"], "pi/work:0.1");
    assert_eq!(snapshot["panes"][0]["command"], "node");
    assert_eq!(snapshot["panes"][0]["output"]["preview"], "hello-remux");

    let inspect = env.remux(["--config", env.config_path(), "inspect", "pi/work:0.1"]);
    assert_success(&inspect);
    let inspect_stdout = stdout(&inspect);
    assert!(inspect_stdout.contains("Target:       pi/work:0.1"));
    assert!(inspect_stdout.contains("Command:      node"));
    assert!(inspect_stdout.contains("hello-remux"));

    let inspect_json = env.remux([
        "--config",
        env.config_path(),
        "inspect",
        "pi/work:0.1",
        "--json",
    ]);
    assert_success(&inspect_json);
    let inspect: Value = serde_json::from_str(&stdout(&inspect_json)).unwrap();
    assert_eq!(inspect["target"], "pi/work:0.1");
    assert!(inspect["output"].as_str().unwrap().contains("hello-remux"));
    assert_eq!(inspect["output_hash"].as_str().unwrap().len(), 64);

    let capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "pi/work:0.1",
        "--lines",
        "2",
    ]);
    assert_success(&capture);
    assert_eq!(stdout(&capture), "line one\nhello-remux\n");
}

struct TestEnv {
    root: PathBuf,
    config: PathBuf,
    path: OsString,
}

impl TestEnv {
    fn new() -> Self {
        let root = unique_temp_dir();
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let ssh_path = bin_dir.join("ssh");
        fs::write(&ssh_path, fake_ssh_script()).unwrap();
        let mut permissions = fs::metadata(&ssh_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&ssh_path, permissions).unwrap();

        let config = root.join("config.yaml");
        fs::write(
            &config,
            r#"
hosts:
  - id: pi
    type: ssh
    ssh:
      target: fake-pi
"#,
        )
        .unwrap();

        let mut path = OsString::from(bin_dir);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        Self { root, config, path }
    }

    fn config_path(&self) -> &str {
        self.config.to_str().unwrap()
    }

    fn remux<const N: usize>(&self, args: [&str; N]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_remux"))
            .args(args)
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("remux-e2e-{}-{nanos}", std::process::id()))
}

fn fake_ssh_script() -> &'static str {
    r#"#!/usr/bin/env bash
set -euo pipefail

args="$*"
if [[ "$args" != *"-o BatchMode=yes"* ]]; then
  echo "missing BatchMode=yes" >&2
  exit 42
fi
if [[ "$args" != *"-o ConnectTimeout=5"* ]]; then
  echo "missing ConnectTimeout=5" >&2
  exit 42
fi
if [[ "$args" != *"fake-pi"* ]]; then
  echo "missing target fake-pi" >&2
  exit 42
fi

remote="${@: -1}"

if [[ "$remote" == tmux\ list-panes* ]]; then
  printf 'work\t0\t1\t%%3\t1234\tnode\t/home/cam/work\n'
  printf 'scratch\t2\t0\t%%4\t2222\tbash\t/tmp\n'
  exit 0
fi

if [[ "$remote" == "tmux capture-pane -pt 'work:0.1' -S -"* ]]; then
  printf 'line one\nhello-remux\n'
  exit 0
fi

if [[ "$remote" == "tmux capture-pane -pt 'scratch:2.0' -S -"* ]]; then
  printf 'scratch output\n'
  exit 0
fi

echo "unexpected remote command: $remote" >&2
exit 43
"#
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout(output),
        stderr(output)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
