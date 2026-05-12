use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn doctor_checks_local_tools_and_hosts() {
    let env = TestEnv::new();

    let doctor = env.remux(["--config", env.config_path(), "doctor"]);
    assert_failure(&doctor);
    let out = stdout(&doctor);
    assert!(out.contains("remux doctor"));
    assert!(out.contains("local tmux"));
    assert!(out.contains("local git"));
    assert!(out.contains("local fzf"));
    assert!(out.contains("ssh access"));
    assert!(out.contains("remote tmux"));
    assert!(out.contains("remote git"));
    assert!(out.contains("overall: fail"));
    assert!(stderr(&doctor).contains("doctor found issues"));

    let doctor_json = env.remux(["--config", env.config_path(), "doctor", "--json"]);
    assert_failure(&doctor_json);
    let report: Value = serde_json::from_str(&stdout(&doctor_json)).unwrap();
    assert_eq!(report["hosts"][0]["host"], "pi");
    assert_eq!(report["hosts"][0]["checks"][0]["name"], "ssh access");
}

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
    assert!(inspect_stdout.contains("Session") && inspect_stdout.contains("codex-agent"));
    assert!(inspect_stdout.contains("Command") && inspect_stdout.contains("node"));
    assert!(inspect_stdout.contains("Branch") && inspect_stdout.contains("main"));
    assert!(inspect_stdout.contains("Dirty files") && inspect_stdout.contains("2"));
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
    let capture_out = stdout(&capture);
    assert!(capture_out.contains("Captured: codex-agent"));
    assert!(capture_out.contains("line one"));
    assert!(capture_out.contains("hello-remux"));

    let color_capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "codex-agent",
        "--lines",
        "2",
        "--color",
    ]);
    assert_success(&color_capture);
    assert!(stdout(&color_capture).contains("\x1b[31mhello-remux\x1b[0m"));

    let attach = env.remux([
        "--config",
        env.config_path(),
        "attach",
        "--readonly",
        "codex-agent",
    ]);
    assert_success(&attach);

    let jump = env.remux(["--config", env.config_path(), "attach", "codex-agent"]);
    assert_success(&jump);
}

#[test]
fn sessions_roll_up_panes_by_tmux_session() {
    let env = TestEnv::new();

    let sessions = env.remux(["--config", env.config_path(), "sessions", "--json"]);
    assert_success(&sessions);
    let sessions: Value = serde_json::from_str(&stdout(&sessions)).unwrap();
    let sessions = sessions.as_array().unwrap();
    let work = sessions
        .iter()
        .find(|session| session["session"] == "work")
        .unwrap();
    assert_eq!(work["host"], "pi");
    assert_eq!(work["windows"], 1);
    assert_eq!(work["panes"], 2);
    assert_eq!(work["attached"], true);
    assert_eq!(work["match_status"], "matched");

    let grouped = env.remux(["--config", env.config_path(), "list", "--group", "sessions"]);
    assert_success(&grouped);
    assert!(stdout(&grouped).contains("work"));
}

#[test]
fn picker_no_fzf_falls_back_to_rows_and_exit_two() {
    let env = TestEnv::new();

    let pick = env.remux(["--config", env.config_path(), "pick", "--no-fzf"]);
    assert_code(&pick, 2);
    assert!(stderr(&pick).contains("fzf is not available"));
    let first_row = stdout(&pick).lines().next().unwrap().to_string();
    let first_target = first_row.split('\t').next().unwrap();
    assert!(first_target.starts_with("pi/"));
    assert!(first_target.contains(':'));
}

#[test]
fn lifecycle_new_and_kill_route_to_tmux_commands() {
    let env = TestEnv::new();

    let create = env.remux([
        "--config",
        env.config_path(),
        "new",
        "pi",
        "new-work",
        "--cwd",
        "/tmp/new-work",
        "--window-name",
        "main",
    ]);
    assert_success(&create);

    let duplicate = env.remux(["--config", env.config_path(), "new", "pi", "work"]);
    assert_code(&duplicate, 2);
    assert!(stderr(&duplicate).contains("already exists"));

    let refuse = env.remux(["--config", env.config_path(), "kill", "pi/work:0.1"]);
    assert_code(&refuse, 2);
    assert!(stderr(&refuse).contains("without --yes"));

    let kill_pane = env.remux([
        "--config",
        env.config_path(),
        "kill",
        "pi/work:0.1",
        "--yes",
    ]);
    assert_success(&kill_pane);

    let kill_session = env.remux(["--config", env.config_path(), "kill", "pi/scratch", "--yes"]);
    assert_success(&kill_session);

    let missing_host = env.remux([
        "--config",
        env.config_path(),
        "kill",
        "missing/work",
        "--yes",
    ]);
    assert_code(&missing_host, 3);
    assert!(stderr(&missing_host).contains("could not be resolved"));
}

#[test]
fn session_grouped_list_warns_about_unreachable_hosts() {
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
  - id: bad
    type: ssh
    ssh:
      target: fake-bad
"#,
    );

    let grouped = env.remux(["--config", env.config_path(), "list", "--group", "sessions"]);
    assert_success(&grouped);
    assert!(stdout(&grouped).contains("work"));
    assert!(stderr(&grouped).contains("warning: bad: poll"));
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
    let capture_out = stdout(&capture);
    assert!(capture_out.contains("Captured: local-agent"));
    assert!(capture_out.contains("local line"));
    assert!(capture_out.contains("local-remux"));

    let attach = env.remux([
        "--config",
        env.config_path(),
        "a",
        "--readonly",
        "local-agent",
    ]);
    assert_success(&attach);

    let jump = env.remux(["--config", env.config_path(), "a", "local-agent"]);
    assert_success(&jump);
}

#[test]
fn local_host_can_inspect_configured_tmux_socket() {
    let env = TestEnv::new();
    env.write_config(
        r#"
poll:
  capture_lines: 2
hosts:
  - id: local
    type: local
    tmux_socket: /tmp/remux-custom.sock
sessions:
  - id: socket-agent
    host: local
    tmux:
      session: socketed
      window: 1
      pane: 0
"#,
    );

    let snapshot = env.remux(["--config", env.config_path(), "snapshot", "local", "--json"]);
    assert_success(&snapshot);
    let snapshot: Value = serde_json::from_str(&stdout(&snapshot)).unwrap();
    assert_eq!(snapshot["tmux_socket"], "/tmp/remux-custom.sock");
    assert_eq!(snapshot["sessions"][0]["session_id"], "socket-agent");
    assert_eq!(
        snapshot["sessions"][0]["tmux_socket"],
        "/tmp/remux-custom.sock"
    );
    assert_eq!(snapshot["sessions"][0]["target"], "local/socketed:1.0");
    assert_eq!(snapshot["sessions"][0]["output"]["preview"], "socket-remux");

    let inspect = env.remux(["--config", env.config_path(), "inspect", "socket-agent"]);
    assert_success(&inspect);
    let inspect_stdout = stdout(&inspect);
    assert!(inspect_stdout.contains("Session") && inspect_stdout.contains("socket-agent"));
    assert!(inspect_stdout.contains("socket-remux"));

    let capture = env.remux([
        "--config",
        env.config_path(),
        "capture",
        "socket-agent",
        "--lines",
        "2",
    ]);
    assert_success(&capture);
    assert!(stdout(&capture).contains("socket-remux"));

    let attach = env.remux_inside_other_tmux([
        "--config",
        env.config_path(),
        "attach",
        "--readonly",
        "socket-agent",
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
    let capture_out = stdout(&capture);
    assert!(capture_out.contains("Captured: codex-live"));
    assert!(capture_out.contains("line one"));
    assert!(capture_out.contains("hello-remux"));

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

    let rollups = env.remux(["--config", env.config_path(), "sessions", "--json"]);
    assert_success(&rollups);
    let rollups: Value = serde_json::from_str(&stdout(&rollups)).unwrap();
    let work = rollups
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session"] == "work")
        .unwrap();
    assert_eq!(work["panes"], 2);
}

#[test]
fn unreachable_host_times_out_within_bounds() {
    // Simulate a host whose SSH hangs (network partition, dead ControlMaster,
    // etc.). The fake ssh script sleeps for 30s; config sets command_timeout
    // to 2s. `remux snapshot` must return a timeout error within a few
    // seconds, not hang.
    let env = HangTestEnv::new(/* hang_secs */ 30);

    let start = std::time::Instant::now();
    let out = env.remux(["--config", env.config_path(), "snapshot", "slow"]);
    let elapsed = start.elapsed();

    assert_failure(&out);
    let err = stderr(&out);
    assert!(
        err.contains("timed out") || err.contains("failed to poll"),
        "expected timeout error in stderr, got: {err}"
    );
    // Generous upper bound: command_timeout (2s) + exec cleanup (0.5s) +
    // process spawn overhead. If this fires, the hang-recovery path is
    // broken.
    assert!(
        elapsed < std::time::Duration::from_secs(6),
        "snapshot took {elapsed:?} — expected bounded recovery under ~3s"
    );
}

#[test]
fn ssh_keepalive_options_are_present() {
    // Regression guard for Fix 1 (ServerAliveInterval / ServerAliveCountMax).
    // The fake ssh script here rejects any invocation missing those options,
    // so a successful snapshot implies remux passes them on every call.
    let env = HangTestEnv::new_asserting_keepalives();

    let out = env.remux(["--config", env.config_path(), "snapshot", "fast"]);
    assert_success(&out);
    assert!(stdout(&out).contains("fast"));
}

#[test]
fn onboard_previews_generated_config_from_ssh_aliases() {
    let env = OnboardEnv::new(
        r#"
Host pi prod
  User cam

Host *
  ServerAliveInterval 60
"#,
    );

    let out = env.remux(["onboard"]);
    assert_success(&out);
    let stdout = stdout(&out);
    assert!(stdout.contains("Using selected SSH aliases"));
    assert!(stdout.contains("- pi"));
    assert!(stdout.contains("- prod"));
    assert!(stdout.contains("target: pi"));
    assert!(stdout.contains("target: prod"));
    assert!(stdout.contains("remux onboard --write"));
}

#[test]
fn onboard_write_creates_config_for_selected_hosts() {
    let env = OnboardEnv::new(
        r#"
Host pi prod
  User cam
"#,
    );

    let out = env.remux(["onboard", "--hosts", "prod", "--write"]);
    assert_success(&out);
    let written = fs::read_to_string(env.config_path()).unwrap();
    assert!(written.contains("- id: local"));
    assert!(written.contains("- id: prod"));
    assert!(written.contains("target: prod"));
    assert!(!written.contains("target: pi"));
}

struct TestEnv {
    root: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    path: OsString,
}

struct OnboardEnv {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl OnboardEnv {
    fn new(ssh_config: &str) -> Self {
        let root = unique_temp_dir();
        let home = root.join("home");
        let ssh_dir = home.join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(ssh_dir.join("config"), ssh_config).unwrap();
        let config = home.join(".config/remux/config.yaml");
        Self { root, home, config }
    }

    fn config_path(&self) -> &PathBuf {
        &self.config
    }

    fn remux<const N: usize>(&self, args: [&str; N]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_remux"))
            .args(args)
            .env("HOME", &self.home)
            .output()
            .unwrap()
    }
}

impl Drop for OnboardEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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

        let git_path = bin_dir.join("git");
        fs::write(&git_path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        make_executable(&git_path);

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
        let mut path = OsString::from(&bin_dir);
        path.push(":");
        path.push("/usr/bin:/bin");

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

    fn remux_inside_other_tmux<const N: usize>(&self, args: [&str; N]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_remux"))
            .args(args)
            .env("PATH", &self.path)
            .env("REMUX_CACHE_PATH", &self.cache)
            .env("TMUX", "/tmp/default-tmux,123,0")
            .output()
            .unwrap()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Minimal test env for hang-recovery scenarios. Uses a parameterized fake
/// ssh that either sleeps forever (hang_secs > 0) or responds normally
/// depending on the requested host.
struct HangTestEnv {
    root: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    path: OsString,
}

impl HangTestEnv {
    fn new(hang_secs: u32) -> Self {
        Self::build(
            fake_hang_ssh_script(hang_secs),
            /* strict_keepalive */ false,
        )
    }

    fn new_asserting_keepalives() -> Self {
        Self::build(fake_hang_ssh_script(0), /* strict_keepalive */ true)
    }

    fn build(ssh_script_template: String, strict_keepalive: bool) -> Self {
        let root = unique_temp_dir();
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let script = if strict_keepalive {
            // Reject invocations missing the keepalive options.
            format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nargs=\"$*\"\n\
                 if [[ \"$args\" != *\"-o ServerAliveInterval=\"* ]]; then\n  \
                   echo 'missing ServerAliveInterval' >&2; exit 77\n\
                 fi\n\
                 if [[ \"$args\" != *\"-o ServerAliveCountMax=\"* ]]; then\n  \
                   echo 'missing ServerAliveCountMax' >&2; exit 77\n\
                 fi\n{body}",
                body = ssh_script_template
                    .strip_prefix("#!/usr/bin/env bash\nset -euo pipefail\n")
                    .unwrap_or(&ssh_script_template),
            )
        } else {
            ssh_script_template
        };

        let ssh_path = bin_dir.join("ssh");
        fs::write(&ssh_path, script).unwrap();
        make_executable(&ssh_path);

        // Stub tmux/git so host::run doesn't accidentally exec the real ones.
        let tmux_path = bin_dir.join("tmux");
        fs::write(&tmux_path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        make_executable(&tmux_path);
        let git_path = bin_dir.join("git");
        fs::write(&git_path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        make_executable(&git_path);

        let config = root.join("config.yaml");
        fs::write(
            &config,
            r#"
poll:
  command_timeout: 2s
  ssh_timeout: 1s
hosts:
  - id: slow
    type: ssh
    ssh:
      target: fake-slow
  - id: fast
    type: ssh
    ssh:
      target: fake-fast
"#,
        )
        .unwrap();

        let cache = root.join("cache.json");
        let mut path = OsString::from(&bin_dir);
        path.push(":");
        path.push("/usr/bin:/bin");

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

    fn remux<const N: usize>(&self, args: [&str; N]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_remux"))
            .args(args)
            .env("PATH", &self.path)
            .env("REMUX_CACHE_PATH", &self.cache)
            .output()
            .unwrap()
    }
}

impl Drop for HangTestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fake_hang_ssh_script(hang_secs: u32) -> String {
    // The fake ssh inspects its target: fake-slow → sleep and never respond,
    // fake-fast → return a minimal valid inventory so `remux snapshot fast`
    // succeeds. Using a single script covers both hosts in one binary.
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
args="$*"
if [[ "$args" == *"fake-slow"* ]]; then
  sleep {hang_secs}
  exit 0
fi
if [[ "$args" == *"fake-fast"* ]]; then
  remote="${{@: -1}}"
  if [[ "$remote" == NONCE=* ]]; then
    NONCE=testnonce
    printf '===REMUX-INVENTORY-%s-BEGIN===\n' "$NONCE"
    printf '===REMUX-INVENTORY-%s-END===\n' "$NONCE"
    printf '===REMUX-END-%s===\n' "$NONCE"
    exit 0
  fi
  printf ''
  exit 0
fi
echo "unknown host in args: $args" >&2
exit 42
"#
    )
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

if [[ "$remote" == "printf ok" ]]; then
  printf 'ok\n'
  exit 0
fi

if [[ "$remote" == "command -v tmux >/dev/null 2>&1" ]]; then
  exit 0
fi

if [[ "$remote" == "command -v git >/dev/null 2>&1" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux list-panes -a -F '#S"* ]]; then
  printf 'work\t0\t1\t%%3\t1234\tnode\t/home/cam/work\t1\n'
  printf 'work\t0\t2\t%%5\t1235\tnode\t/home/cam/work/sub\t1\n'
  printf 'scratch\t2\t0\t%%4\t2222\tbash\t/tmp\t0\n'
  exit 0
fi

if [[ "$remote" == NONCE=* ]]; then
  NONCE=testfixednonce
  printf '===REMUX-INVENTORY-%s-BEGIN===\n' "$NONCE"
  printf 'work\t0\t1\t%%3\t1234\tnode\t/home/cam/work\t1\n'
  printf 'work\t0\t2\t%%5\t1235\tnode\t/home/cam/work/sub\t1\n'
  printf 'scratch\t2\t0\t%%4\t2222\tbash\t/tmp\t0\n'
  printf '===REMUX-INVENTORY-%s-END===\n' "$NONCE"
  printf '===REMUX-CAPTURE-%s-%%3===\n' "$NONCE"
  printf 'line one\nhello-remux\n'
  printf '===REMUX-CAPTURE-%s-%%5===\n' "$NONCE"
  printf 'second node output\n'
  printf '===REMUX-CAPTURE-%s-%%4===\n' "$NONCE"
  printf 'scratch output\n'
  printf '===REMUX-GIT-%s-%%3-BEGIN===\n' "$NONCE"
  printf '/home/cam/work\n'
  printf 'main\n'
  printf '0\n'
  printf '===REMUX-GIT-%s-%%3-END===\n' "$NONCE"
  printf '===REMUX-GIT-%s-%%5-BEGIN===\n' "$NONCE"
  printf '/home/cam/work\n'
  printf 'main\n'
  printf '0\n'
  printf '===REMUX-GIT-%s-%%5-END===\n' "$NONCE"
  printf '===REMUX-END-%s===\n' "$NONCE"
  exit 0
fi

if [[ "$remote" == "tmux capture-pane -e -pt 'work:0.1' -S -"* ]]; then
  printf 'line one\n\x1b[31mhello-remux\x1b[0m\n'
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

if [[ "$remote" == "tmux attach-session -r -t 'work' \\; select-window -t '0' \\; select-pane -t '%3'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux attach-session -t 'work' \\; select-window -t '0' \\; select-pane -t '%3'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux attach-session -r -t 'work' \\; select-window -t '0' \\; select-pane -t '1'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux attach-session -t 'work' \\; select-window -t '0' \\; select-pane -t '1'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux new-session -d -s 'new-work' -c '/tmp/new-work' -n 'main'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux kill-pane -t 'work:0.1'" ]]; then
  exit 0
fi

if [[ "$remote" == "tmux kill-session -t 'scratch'" ]]; then
  exit 0
fi

echo "unexpected remote command: $remote" >&2
exit 43
"#
}

fn fake_tmux_script() -> &'static str {
    r##"#!/usr/bin/env bash
set -euo pipefail

socket=""
if [[ "${1:-}" == "-S" ]]; then
  socket="${2:-}"
  shift 2
fi
args="$*"

if [[ "$socket" == "/tmp/remux-custom.sock" && "${1:-}" == "list-panes" && "$args" == *"pane_id}"* && "$args" != *"#S"* ]]; then
  echo '%70'
  exit 0
fi

if [[ "$socket" == "/tmp/remux-custom.sock" && "${1:-}" == "list-panes" ]]; then
  printf 'socketed\t1\t0\t%%70\t7777\tzsh\t/tmp/socketed\t1\n'
  exit 0
fi

if [[ "$socket" == "/tmp/remux-custom.sock" && "${1:-}" == "capture-pane" && ("${3:-}" == "socketed:1.0" || "${3:-}" == "%70") ]]; then
  printf 'socket line\nsocket-remux\n'
  exit 0
fi

if [[ "$socket" == "/tmp/remux-custom.sock" && "${1:-}" == "attach-session" ]]; then
  if [[ -n "${TMUX:-}" ]]; then
    echo "TMUX should be unset for cross-socket attach" >&2
    exit 47
  fi
  if [[ "$args" == *"socketed"* && "$args" == *"select-pane -t %70"* ]]; then
    exit 0
  fi
fi

if [[ "$socket" == "/tmp/remux-custom.sock" ]]; then
  echo "unexpected tmux socket command: $*" >&2
  exit 46
fi

if [[ "${1:-}" == "list-panes" && "$args" == *"pane_id}"* && "$args" != *"#S"* ]]; then
  echo '%7'
  echo '%8'
  exit 0
fi

if [[ "${1:-}" == "list-panes" ]]; then
  printf 'local\t0\t0\t%%7\t4321\tbash\t/tmp/local\t0\n'
  printf 'broken\t0\t0\t%%8\t5555\tbash\t/tmp/broken\t0\n'
  exit 0
fi

if [[ "${1:-}" == "capture-pane" && ("${3:-}" == "local:0.0" || "${3:-}" == "%7") ]]; then
  printf 'local line\nlocal-remux\n'
  exit 0
fi

if [[ "${1:-}" == "capture-pane" && ("${3:-}" == "broken:0.0" || "${3:-}" == "%8") ]]; then
  echo "capture failed" >&2
  exit 45
fi

if [[ "${1:-}" == "attach-session" ]]; then
  args="$*"
  if [[ "$args" == *"local"* && "$args" == *"select-pane -t %7"* ]]; then
    exit 0
  fi
fi

if [[ "${1:-}" == "switch-client" ]]; then
  args="$*"
  if [[ "$args" == *"-t %7"* ]]; then
    exit 0
  fi
  if [[ "$args" == *"local"* && "$args" == *"select-pane -t %7"* ]]; then
    exit 0
  fi
fi

echo "unexpected tmux command: $*" >&2
exit 44
"##
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

fn assert_code(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "unexpected exit code\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
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
