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
    assert!(snapshot_stdout.contains("codex-agent"));
    assert!(snapshot_stdout.contains("pi/work:0.1"));
    assert!(snapshot_stdout.contains("node"));
    assert!(snapshot_stdout.contains("/home/cam/work"));
    assert!(snapshot_stdout.contains("hello-remux"));

    let snapshot_json = env.remux(["--config", env.config_path(), "snapshot", "pi", "--json"]);
    assert_success(&snapshot_json);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot_json)).unwrap();
    assert_eq!(snapshot["host"], "pi");
    assert_eq!(snapshot["status"], "ok");
    assert_eq!(snapshot["sessions"][0]["session_id"], "codex-agent");
    assert_eq!(snapshot["sessions"][0]["target"], "pi/work:0.1");
    assert_eq!(snapshot["sessions"][0]["process"]["command"], "node");
    assert_eq!(snapshot["sessions"][0]["repo"]["branch"], "main");
    assert_eq!(snapshot["sessions"][0]["repo"]["dirty_count"], 2);
    assert_eq!(snapshot["sessions"][0]["output"]["preview"], "hello-remux");

    let list = env.remux(["--config", env.config_path(), "list"]);
    assert_success(&list);
    let list_stdout = stdout(&list);
    assert!(list_stdout.contains("codex-agent"));
    assert!(list_stdout.contains("node"));
    assert!(list_stdout.contains("2"));

    let inspect = env.remux(["--config", env.config_path(), "inspect", "codex-agent"]);
    assert_success(&inspect);
    let inspect_stdout = stdout(&inspect);
    assert!(inspect_stdout.contains("Session:      codex-agent"));
    assert!(inspect_stdout.contains("Command:      node"));
    assert!(inspect_stdout.contains("Branch:       main"));
    assert!(inspect_stdout.contains("Dirty files:  2"));
    assert!(inspect_stdout.contains("hello-remux"));

    let inspect_json = env.remux([
        "--config",
        env.config_path(),
        "inspect",
        "codex-agent",
        "--json",
    ]);
    assert_success(&inspect_json);
    let inspect: Value = serde_json::from_str(&stdout(&inspect_json)).unwrap();
    assert_eq!(inspect["session_id"], "codex-agent");
    assert!(
        inspect["recent_output"]
            .as_str()
            .unwrap()
            .contains("hello-remux")
    );

    let capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "codex-agent",
        "--lines",
        "2",
    ]);
    assert_success(&capture);
    assert_eq!(stdout(&capture), "line one\nhello-remux\n");

    let attach = env.remux([
        "--config",
        env.config_path(),
        "attach",
        "--readonly",
        "codex-agent",
    ]);
    assert_success(&attach);
}

#[test]
fn local_session_works_end_to_end() {
    let env = TestEnv::new();
    env.write_config(
        r#"
poll:
  capture_lines: 2
hosts:
  - id: local
    type: local
sessions:
  - id: local-agent
    host: local
    tmux:
      session: local
      window: 0
      pane: 0
"#,
    );

    let snapshot = env.remux(["--config", env.config_path(), "snapshot", "local", "--json"]);
    assert_success(&snapshot);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot)).unwrap();
    assert_eq!(snapshot["sessions"][0]["session_id"], "local-agent");
    assert_eq!(snapshot["sessions"][0]["process"]["command"], "bash");

    let list = env.remux(["--config", env.config_path(), "ls"]);
    assert_success(&list);
    assert!(stdout(&list).contains("local-agent"));

    let capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "local-agent",
        "--lines",
        "2",
    ]);
    assert_success(&capture);
    assert_eq!(stdout(&capture), "local line\nlocal-remux\n");

    let attach = env.remux([
        "--config",
        env.config_path(),
        "a",
        "--readonly",
        "local-agent",
    ]);
    assert_success(&attach);
}

#[test]
fn capture_failures_are_reported_without_fake_output() {
    let env = TestEnv::new();
    env.write_config(
        r#"
hosts:
  - id: local
    type: local
sessions:
  - id: broken-agent
    host: local
    tmux:
      session: broken
      window: 0
      pane: 0
"#,
    );

    let snapshot = env.remux(["--config", env.config_path(), "snapshot", "local", "--json"]);
    assert_success(&snapshot);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot)).unwrap();
    let session = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == "broken-agent")
        .unwrap();

    assert_eq!(session["state"], "unknown");
    assert!(session["output"].is_null());
    assert_eq!(session["errors"][0]["kind"], "capture");
    assert!(
        !serde_json::to_string(session)
            .unwrap()
            .contains("failed to capture pane output")
    );
}

#[test]
fn watches_resolve_drift_ambiguity_missing_and_shadowing() {
    let env = TestEnv::new();
    env.write_config(
        r#"
poll:
  capture_lines: 2
hosts:
  - id: pi
    type: ssh
    ssh:
      target: fake-pi
watches:
  - id: codex-live
    host: pi
    match:
      command: node
      cwd: /home/cam/work
    agent_hint: codex
  - id: node-ambiguous
    host: pi
    match:
      command: node
      cwd_prefix: /home/cam/work
  - id: codex-shadow
    host: pi
    match:
      command: node
      cwd: /home/cam/work
  - id: missing-kiro
    host: pi
    match:
      command: kiro-cli
      cwd: /home/cam
"#,
    );

    let snapshot_json = env.remux(["--config", env.config_path(), "snapshot", "pi", "--json"]);
    assert_success(&snapshot_json);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot_json)).unwrap();
    let sessions = snapshot["sessions"].as_array().unwrap();

    let matched = find_session(sessions, "codex-live");
    assert_eq!(matched["match_status"], "matched");
    assert_eq!(matched["raw_target"], "pi/work:0.1");

    let ambiguous = find_session(sessions, "node-ambiguous");
    assert_eq!(ambiguous["match_status"], "ambiguous");
    assert_eq!(ambiguous["candidate_targets"].as_array().unwrap().len(), 2);

    let shadowed = find_session(sessions, "codex-shadow");
    assert_eq!(shadowed["match_status"], "shadowed");
    assert_eq!(shadowed["shadowed_by"], "codex-live");

    let missing = find_session(sessions, "missing-kiro");
    assert_eq!(missing["match_status"], "missing");

    let orphan = find_session(sessions, "pi/work:0.2");
    assert_eq!(orphan["match_status"], "orphan");

    let capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "codex-live",
        "--lines",
        "2",
    ]);
    assert_success(&capture);
    assert_eq!(stdout(&capture), "line one\nhello-remux\n");

    let ambiguous_capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "node-ambiguous",
        "--lines",
        "2",
    ]);
    assert_failure(&ambiguous_capture);
    assert!(stderr(&ambiguous_capture).contains("ambiguous"));

    let attach = env.remux([
        "--config",
        env.config_path(),
        "attach",
        "--readonly",
        "codex-live",
    ]);
    assert_success(&attach);
}

struct TestEnv {
    root: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    path: OsString,
}

impl TestEnv {
    fn new() -> Self {
        let root = unique_temp_dir();
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let ssh_path = bin_dir.join("ssh");
        fs::write(&ssh_path, fake_ssh_script()).unwrap();
        make_executable(&ssh_path);

        let tmux_path = bin_dir.join("tmux");
        fs::write(&tmux_path, fake_tmux_script()).unwrap();
        make_executable(&tmux_path);

        let config = root.join("config.yaml");
        fs::write(
            &config,
            r#"
poll:
  capture_lines: 2
hosts:
  - id: pi
    type: ssh
    ssh:
      target: fake-pi
sessions:
  - id: codex-agent
    host: pi
    tmux:
      session: work
      window: 0
      pane: 1
    repo: /repo
    agent_hint: codex
"#,
        )
        .unwrap();

        let cache = root.join("cache.json");
        let mut path = OsString::from(bin_dir);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        Self {
            root,
            config,
            cache,
            path,
        }
    }

    fn config_path(&self) -> &str {
        self.config.to_str().unwrap()
    }

    fn write_config(&self, contents: &str) {
        fs::write(&self.config, contents).unwrap();
    }

    fn remux<const N: usize>(&self, args: [&str; N]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_remux"))
            .args(args)
            .env("PATH", &self.path)
            .env("REMUX_CACHE_PATH", &self.cache)
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
  printf 'work\t0\t2\t%%5\t1235\tnode\t/home/cam/work/sub\n'
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

if [[ "$remote" == "tmux capture-pane -pt 'work:0.2' -S -"* ]]; then
  printf 'second node output\n'
  exit 0
fi

if [[ "$remote" == "git -C '/repo' rev-parse --abbrev-ref HEAD" ]]; then
  printf 'main\n'
  exit 0
fi

if [[ "$remote" == "git -C '/repo' status --porcelain=v1" ]]; then
  printf ' M src/main.rs\n?? notes/debug.md\n'
  exit 0
fi

if [[ "$remote" == "tmux attach-session -r -t 'work' \\; select-window -t '0' \\; select-pane -t '1'" ]]; then
  exit 0
fi

echo "unexpected remote command: $remote" >&2
exit 43
"#
}

fn fake_tmux_script() -> &'static str {
    r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "list-panes" ]]; then
  printf 'local\t0\t0\t%%7\t4321\tbash\t/tmp/local\n'
  printf 'broken\t0\t0\t%%8\t5555\tbash\t/tmp/broken\n'
  exit 0
fi

if [[ "${1:-}" == "capture-pane" && "${3:-}" == "local:0.0" ]]; then
  printf 'local line\nlocal-remux\n'
  exit 0
fi

if [[ "${1:-}" == "capture-pane" && "${3:-}" == "broken:0.0" ]]; then
  echo "capture failed" >&2
  exit 45
fi

if [[ "${1:-}" == "attach-session" ]]; then
  args="$*"
  if [[ "$args" == *"-r"* && "$args" == *"local"* ]]; then
    exit 0
  fi
fi

echo "unexpected tmux command: $*" >&2
exit 44
"#
}

fn make_executable(path: &PathBuf) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
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

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn find_session<'a>(sessions: &'a [Value], display_id: &str) -> &'a Value {
    sessions
        .iter()
        .find(|session| session["display_id"] == display_id)
        .unwrap_or_else(|| panic!("missing session {display_id} in {sessions:#?}"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
