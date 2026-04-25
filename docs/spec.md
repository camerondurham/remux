---
title: remux v0 Initial Build Spec
created: 2026-04-25
status: draft
owner: Cam
project: remux
---

# remux v0 Initial Build Spec

## 0. Bottom line

Build `remux`: a local-first Rust CLI for monitoring and entering tmux sessions across local and SSH hosts.

This is **not** an AI agent framework. It is a factual remote session inventory and jump surface.

Core use case:

> I have coding agents, shells, and long-running work spread across tmux sessions on multiple machines. I want one local command to show what is running, what pane/repo/process it maps to, recent visible output, and a reliable way to attach to the correct session or pane.

The v0 should prioritize correctness, boring facts, and reliable attach behavior over UI polish.

---

## 1. Product shape

### 1.1 One-line description

`remux` is `abtop`/`btop`-style visibility for local and remote tmux sessions.

### 1.2 Primary user

A CLI-first backend/software engineer who:

- uses tmux heavily
- works across local and remote machines
- runs coding agents or long-running shells in separate sessions
- wants a factual control surface without adopting a full agent orchestration platform

### 1.3 Design principles

1. **Factual, not interpretive.** Show what is happening. Do not score risk or infer intent.
2. **Local-first.** Config, cache, and UI live on the user’s machine.
3. **SSH-native.** Reuse the user’s existing SSH setup.
4. **tmux-native.** Treat tmux as the remote session API.
5. **Read-only by default.** Observation must not mutate remote session state.
6. **Fast failure.** A broken host must not hang the whole dashboard.
7. **CLI before TUI.** Build the state engine first; the TUI sits on top later.

---

## 2. Non-goals for v0

Do not build these in v0:

- AI summaries
- risk scoring
- trust scoring
- prompt packs
- agent-to-agent orchestration
- autonomous spawning of agents
- remote daemon
- web dashboard
- SaaS sync
- SQLite history
- tmux control-mode streaming
- terminal emulator / ANSI-faithful rendering
- arbitrary remote execution framework
- worktree manager

The tool may execute narrow, known remote commands needed for tmux/session inspection. It should not become a general remote automation product.

---

## 3. Required CLI interface

### 3.1 Commands

Implement these v0 commands:

```bash
remux hosts
remux snapshot <host> [--json]
remux list [--json]
remux inspect <session-id> [--json]
remux capture <session-id> [--lines N]
remux attach <session-id>
remux attach --readonly <session-id>
```

Optional aliases:

```bash
remux ls
remux i <session-id>
remux a <session-id>
```

### 3.2 Command behavior

#### `remux hosts`

Shows configured hosts and last-known status if available.

Example:

```text
HOST      TYPE    TARGET        STATUS       LAST POLL
local     local   -             ok           12s ago
devbox    ssh     cam@devbox    ok           18s ago
vps       ssh     cam@vps       unreachable  11m ago
```

#### `remux snapshot <host>`

Polls exactly one host and prints a snapshot.

Default output is human-readable. `--json` prints structured JSON.

Required behavior:

- works for `local`
- works for SSH host
- fails fast on unreachable host
- does not prompt interactively
- includes tmux pane metadata
- includes configured repo metadata where available
- includes recent pane output preview

#### `remux list`

Polls all hosts or reads fresh snapshots depending on implementation simplicity.

Required output columns:

```text
HOST      SESSION          PANE   CMD      REPO             STATE        LAST OUT   DIRTY
local     codex-harness    0.1    codex    agent-harness    active       1m ago     3
devbox    reindex-agent    0.0    kiro     reindex-tool     quiet        34m ago    7
devbox    scratch          1.0    zsh      -                idle         2h ago     -
vps       deploy           -      -        -                unreachable  -          -
```

#### `remux inspect <session-id>`

Shows detailed factual state for one configured session.

Example:

```text
Session:      reindex-agent
Host:         devbox
Tmux target:  reindex:0.0
Agent hint:   kiro
Command:      kiro
PID:          12345
CWD:          /home/cam/work/reindex-tool
Repo:         /home/cam/work/reindex-tool
Branch:       feature/reindex-monitor
Dirty files:  7
State:        quiet
Last output:  34m ago

Recent output:
────────────────────────────────────────────────────────────
Running cluster health check...
Contributor backlog: 12439
Last successful write: 2026-04-25T18:39:22
────────────────────────────────────────────────────────────

Changed files:
  M src/main.rs
  M src/snapshot.rs
  ?? notes/debug.md

Commands:
  remux capture reindex-agent --lines 200
  remux attach --readonly reindex-agent
  remux attach reindex-agent
```

#### `remux capture <session-id> --lines N`

Prints the recent captured pane output for the configured session/pane.

Default:

```bash
remux capture reindex-agent
```

Equivalent to:

```bash
remux capture reindex-agent --lines 120
```

#### `remux attach <session-id>`

Attaches interactively to the configured tmux session.

For local host:

```bash
tmux attach-session -t '<session>' \; select-window -t '<window>' \; select-pane -t '<pane>'
```

For SSH host:

```bash
ssh -t <host> 'tmux attach-session -t <session> \; select-window -t <window> \; select-pane -t <pane>'
```

#### `remux attach --readonly <session-id>`

Same as attach, but read-only:

```bash
tmux attach-session -r -t '<session>'
```

Remote equivalent:

```bash
ssh -t <host> 'tmux attach-session -r -t <session> \; select-window -t <window> \; select-pane -t <pane>'
```

---

## 4. State definitions

### 4.1 Session state

Use factual state labels only.

```text
active       output changed within last active_after duration
quiet        output has not changed within active_after, but less than idle_after
idle         output has not changed for idle_after or more
missing      configured tmux session/pane was not found
unreachable  host poll failed
unknown      insufficient data
```

Default thresholds:

```yaml
active_after: 5m
idle_after: 60m
```

### 4.2 Last output detection

For v0, last output can be approximated from snapshot history if a cache exists.

Acceptable v0 simplification:

- If no prior snapshot exists, mark `unknown`.
- If pane output hash changed since prior snapshot, set `last_output_at = now`.
- If unchanged, preserve prior `last_output_at`.
- If no cache implemented yet, show `last_seen` instead of `last_output`.

Recommended v0.1:

- JSON cache file under `~/.local/share/remux/snapshots.json`
- no SQLite yet

---

## 5. Configuration

### 5.1 Config path

Default:

```text
~/.config/remux/config.yaml
```

Support override:

```bash
remux --config ./examples/config.yaml list
```

### 5.2 Example config

```yaml
poll:
  active_after: 5m
  idle_after: 60m
  capture_lines: 80
  ssh_timeout: 5s

hosts:
  - id: local
    type: local

  - id: devbox
    type: ssh
    ssh:
      host: devbox
      user: cam
      port: 22
      options:
        BatchMode: "yes"
        ConnectTimeout: "5"
        StrictHostKeyChecking: "accept-new"
        ControlMaster: "auto"
        ControlPersist: "10m"

sessions:
  - id: codex-harness
    host: local
    tmux:
      session: harness
      window: 0
      pane: 1
    repo: ~/code/agent-harness
    agent_hint: codex

  - id: reindex-agent
    host: devbox
    tmux:
      session: reindex
      window: 0
      pane: 0
    repo: ~/work/reindex-tool
    agent_hint: kiro
```

### 5.3 Config requirements

- Host IDs must be unique.
- Session IDs must be unique.
- Every session must reference an existing host.
- `local` host type requires no SSH config.
- `ssh` host type requires `ssh.host`.
- `repo` is optional.
- `agent_hint` is optional.
- `tmux.window` and `tmux.pane` are optional; if omitted, inspect the first pane in the session.

---

## 6. Data contracts

### 6.1 Rust-ish model

```rust
struct Config {
    poll: PollConfig,
    hosts: Vec<HostConfig>,
    sessions: Vec<SessionConfig>,
}

struct PollConfig {
    active_after: Duration,
    idle_after: Duration,
    capture_lines: usize,
    ssh_timeout: Duration,
}

struct HostConfig {
    id: String,
    kind: HostKind,
    ssh: Option<SshConfig>,
}

enum HostKind {
    Local,
    Ssh,
}

struct SshConfig {
    host: String,
    user: Option<String>,
    port: Option<u16>,
    options: BTreeMap<String, String>,
}

struct SessionConfig {
    id: String,
    host: String,
    tmux: TmuxTarget,
    repo: Option<String>,
    agent_hint: Option<String>,
}

struct TmuxTarget {
    session: String,
    window: Option<u32>,
    pane: Option<u32>,
}
```

### 6.2 Snapshot JSON

`remux snapshot devbox --json` should emit:

```json
{
  "host": "devbox",
  "status": "ok",
  "collected_at": "2026-04-25T19:40:00-07:00",
  "sessions": [
    {
      "session_id": "reindex-agent",
      "host": "devbox",
      "state": "quiet",
      "agent_hint": "kiro",
      "tmux": {
        "session": "reindex",
        "window": "0",
        "pane": "0",
        "pane_id": "%1"
      },
      "process": {
        "pid": 12345,
        "command": "kiro",
        "cwd": "/home/cam/work/reindex-tool"
      },
      "repo": {
        "path": "/home/cam/work/reindex-tool",
        "branch": "feature/reindex-monitor",
        "dirty_count": 7,
        "changed_files": [
          "M src/main.rs",
          "M src/snapshot.rs"
        ]
      },
      "output": {
        "preview": "Running cluster health check...",
        "hash": "abc123",
        "last_output_at": "2026-04-25T19:06:00-07:00"
      }
    }
  ],
  "errors": []
}
```

Error case:

```json
{
  "host": "devbox",
  "status": "unreachable",
  "collected_at": "2026-04-25T19:40:00-07:00",
  "sessions": [],
  "errors": [
    {
      "kind": "ssh_timeout",
      "message": "SSH connection timed out after 5s"
    }
  ]
}
```

---

## 7. Remote command requirements

### 7.1 tmux inventory

Use this as the core tmux inventory command:

```bash
tmux list-panes -a -F '#S	#I	#P	#{pane_id}	#{pane_pid}	#{pane_current_command}	#{pane_current_path}'
```

Expected fields:

```text
session
window_index
pane_index
pane_id
pane_pid
pane_current_command
pane_current_path
```

### 7.2 tmux capture

Use:

```bash
tmux capture-pane -pt '<session>:<window>.<pane>' -S -80
```

Requirements:

- default to plain text
- do not preserve ANSI escape sequences in v0
- support configurable line count
- if pane does not exist, return a structured error

### 7.3 git metadata

Only run git commands for configured repos.

```bash
git -C <repo> rev-parse --abbrev-ref HEAD
git -C <repo> status --porcelain=v1
```

Requirements:

- if repo path does not exist, return repo error but do not fail whole snapshot
- dirty count is number of porcelain status lines
- changed files are raw porcelain lines for v0

### 7.4 SSH execution

Use system OpenSSH via Rust `openssh` crate if practical. Shelling out to `ssh` is acceptable for an MVP if faster.

Required SSH behavior:

- must use non-interactive mode
- must set connection timeout
- must not hang dashboard on password prompt
- should reuse existing `~/.ssh/config`
- should work with jump hosts configured by user

Recommended SSH options:

```text
BatchMode=yes
ConnectTimeout=5
StrictHostKeyChecking=accept-new
ControlMaster=auto
ControlPersist=10m
```

---

## 8. Suggested implementation stack

Required:

```text
Rust
clap
serde
serde_yaml
serde_json
anyhow or thiserror
chrono or time
sha2 or blake3
```

Recommended:

```text
tokio
openssh
comfy-table or tabled
dirs
shellexpand
tracing
tracing-subscriber
```

Later for TUI:

```text
ratatui
crossterm
```

Do not use SQLite in v0 unless the agent strongly needs it. Prefer JSON cache first.

---

## 9. Phase plan with validation

## Phase 0: Repo setup

### Build

Create Rust CLI project:

```bash
cargo new remux --bin
cd remux
```

Add basic CLI parsing with `clap`.

Commands should exist but may return `todo!()` initially:

```bash
remux hosts
remux snapshot <host>
remux list
remux inspect <session-id>
remux capture <session-id>
remux attach <session-id>
```

### Validation

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- --help
cargo run -- hosts --help
cargo run -- snapshot --help
```

Pass criteria:

- all commands appear in help
- no clippy warnings
- tests pass
- binary exits with code 0 for `--help`

---

## Phase 1: Config loading and validation

### Build

Implement:

- default config path
- `--config <path>`
- YAML parsing
- environment/home expansion for paths
- validation errors with actionable messages

Validation rules:

- duplicate host ID is error
- duplicate session ID is error
- session references missing host is error
- SSH host missing `ssh.host` is error
- invalid durations are errors

### Validation

Create fixtures:

```text
fixtures/config/valid.yaml
fixtures/config/duplicate-host.yaml
fixtures/config/duplicate-session.yaml
fixtures/config/missing-host-ref.yaml
fixtures/config/invalid-duration.yaml
```

Run:

```bash
cargo test config
cargo run -- --config fixtures/config/valid.yaml hosts
cargo run -- --config fixtures/config/duplicate-host.yaml hosts
cargo run -- --config fixtures/config/missing-host-ref.yaml hosts
```

Pass criteria:

- valid config prints hosts
- invalid configs exit non-zero
- error messages identify the bad field and config item
- no panic on malformed YAML

---

## Phase 2: Local tmux snapshot

### Build

Implement local tmux probing:

```bash
tmux list-panes -a -F '#S	#I	#P	#{pane_id}	#{pane_pid}	#{pane_current_command}	#{pane_current_path}'
```

Parse output into structs.

Match configured sessions against discovered panes.

Implement:

```bash
remux snapshot local --json
remux list
```

### Validation

Automated parser test:

```bash
cargo test tmux_parser
```

Manual integration test:

```bash
tmux new-session -d -s remux-test 'sleep 300'
tmux split-window -t remux-test
cargo run -- --config fixtures/config/local-tmux.yaml snapshot local --json
cargo run -- --config fixtures/config/local-tmux.yaml list
tmux kill-session -t remux-test
```

Pass criteria:

- snapshot includes `remux-test`
- pane fields parse correctly
- configured session appears in list
- missing configured session is marked `missing`
- if tmux is not installed/running, command returns structured error, not panic

---

## Phase 3: Pane capture

### Build

Implement:

```bash
remux capture <session-id> --lines N
```

Use:

```bash
tmux capture-pane -pt '<session>:<window>.<pane>' -S -N
```

For v0, plain text output is enough.

Also include output preview/hash in snapshot.

### Validation

Manual integration test:

```bash
tmux new-session -d -s remux-capture 'printf "hello-remux\n"; sleep 300'
cargo run -- --config fixtures/config/capture.yaml capture remux-capture --lines 20
cargo run -- --config fixtures/config/capture.yaml snapshot local --json | jq .
tmux kill-session -t remux-capture
```

Pass criteria:

- capture output contains `hello-remux`
- snapshot includes non-empty output preview
- snapshot includes stable output hash
- invalid session ID exits non-zero with useful message
- missing tmux pane exits non-zero with useful message

---

## Phase 4: Git metadata

### Build

For sessions with configured `repo`, collect:

```bash
git -C <repo> rev-parse --abbrev-ref HEAD
git -C <repo> status --porcelain=v1
```

Add to `snapshot`, `list`, and `inspect`.

### Validation

Manual integration test:

```bash
tmpdir=$(mktemp -d)
git -C "$tmpdir" init
echo "x" > "$tmpdir/file.txt"
git -C "$tmpdir" add file.txt
git -C "$tmpdir" commit -m "init"
echo "y" >> "$tmpdir/file.txt"

# point fixture config repo to $tmpdir
cargo run -- --config fixtures/config/git.yaml inspect git-test
```

Pass criteria:

- branch is displayed
- dirty count is at least 1
- changed file line is displayed
- non-git path does not crash snapshot
- missing repo path reports repo error but does not fail whole host snapshot

---

## Phase 5: SSH host snapshot

### Build

Implement SSH transport for:

```bash
remux snapshot <ssh-host> --json
```

Requirements:

- use non-interactive SSH
- use timeout
- preserve structured errors
- support existing SSH aliases from user config
- run tmux inventory remotely
- run capture remotely
- run git metadata remotely for configured repos

### Validation

Minimum local SSH validation if available:

```bash
ssh localhost true
cargo run -- --config fixtures/config/ssh-localhost.yaml snapshot localhost --json
```

If no local SSH server exists, mock command runner tests are acceptable:

```bash
cargo test ssh_command_builder
cargo test remote_snapshot_parser
```

Manual remote validation:

```bash
ssh devbox 'tmux new-session -d -s remux-remote "printf remote-ok; sleep 300"'
cargo run -- --config fixtures/config/devbox.yaml snapshot devbox --json
cargo run -- --config fixtures/config/devbox.yaml list
ssh devbox 'tmux kill-session -t remux-remote'
```

Pass criteria:

- reachable SSH host returns snapshot
- unreachable SSH host exits non-zero for `snapshot`, but `list` marks host/session `unreachable`
- command never prompts for password
- command times out within configured timeout
- remote tmux session fields parse correctly
- remote capture works

---

## Phase 6: Inspect command

### Build

Implement:

```bash
remux inspect <session-id>
remux inspect <session-id> --json
```

Inspect should show:

- session ID
- host
- tmux target
- agent hint
- command
- PID
- cwd
- repo branch
- dirty count
- changed files
- state
- recent output preview
- recommended follow-up commands

### Validation

Run:

```bash
cargo run -- --config fixtures/config/local-full.yaml inspect remux-test
cargo run -- --config fixtures/config/local-full.yaml inspect remux-test --json | jq .
cargo run -- --config fixtures/config/local-full.yaml inspect does-not-exist
```

Pass criteria:

- valid session prints full detail
- JSON output is valid
- unknown session exits non-zero with useful message
- missing session is clearly marked `missing`
- unreachable host is clearly marked `unreachable`

---

## Phase 7: Attach command

### Build

Implement:

```bash
remux attach <session-id>
remux attach --readonly <session-id>
```

Local attach:

```bash
tmux attach-session -t '<session>' \; select-window -t '<window>' \; select-pane -t '<pane>'
```

Remote attach:

```bash
ssh -t <host> 'tmux attach-session -t <session> \; select-window -t <window> \; select-pane -t <pane>'
```

Read-only uses `-r`.

### Validation

Manual local validation:

```bash
tmux new-session -d -s remux-attach 'sleep 300'
cargo run -- --config fixtures/config/attach.yaml attach --readonly remux-attach
# verify it attaches read-only
tmux kill-session -t remux-attach
```

Manual remote validation:

```bash
ssh devbox 'tmux new-session -d -s remux-attach "sleep 300"'
cargo run -- --config fixtures/config/devbox.yaml attach --readonly remux-attach
ssh devbox 'tmux kill-session -t remux-attach'
```

Pass criteria:

- local attach works
- local readonly attach works
- remote attach works
- remote readonly attach works
- configured window/pane selection is applied
- unknown session exits with useful error
- missing remote host exits with useful error

---

## Phase 8: Basic cache for last output state

### Build

Implement optional JSON cache:

```text
~/.local/share/remux/cache.json
```

Store:

- host
- session ID
- pane ID
- last output hash
- last output changed timestamp
- last successful poll timestamp

Use it to compute:

- `active`
- `quiet`
- `idle`

### Validation

Manual test:

```bash
tmux new-session -d -s remux-cache 'while true; do date; sleep 10; done'
cargo run -- --config fixtures/config/cache.yaml list
sleep 15
cargo run -- --config fixtures/config/cache.yaml list
tmux kill-session -t remux-cache
```

Also test stable output:

```bash
tmux new-session -d -s remux-quiet 'printf done; sleep 300'
cargo run -- --config fixtures/config/cache.yaml list
sleep 10
cargo run -- --config fixtures/config/cache.yaml list
tmux kill-session -t remux-quiet
```

Pass criteria:

- changing output becomes/remains `active`
- unchanged output transitions toward `quiet`
- cache file is created
- corrupt cache does not crash app; app warns and rebuilds cache
- cache can be disabled or ignored if needed

---

## Phase 9: Packaging and developer quality

### Build

Add:

- README with install/use examples
- example config
- shell completion if easy
- Makefile or justfile
- release profile
- basic CI workflow if hosted on GitHub

Suggested commands:

```bash
just fmt
just lint
just test
just run-list
```

### Validation

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
cargo run -- --help
```

Pass criteria:

- clean fmt
- clean clippy
- all tests pass
- release build works
- README contains a 5-minute local setup path

---

## 10. Error handling requirements

Every user-facing error should include:

- what failed
- host/session involved
- command category involved, not necessarily raw command
- suggested next check when obvious

Examples:

```text
error: failed to poll host devbox: SSH connection timed out after 5s
hint: verify `ssh devbox true` works and that BatchMode auth is configured
```

```text
error: configured session reindex-agent was not found on host devbox
hint: verify `tmux list-sessions` on devbox
```

```text
error: repo path does not exist for session reindex-agent: ~/work/reindex-tool
hint: update ~/.config/remux/config.yaml or remove repo from this session
```

---

## 11. Security and safety requirements

- Never pass untrusted user input to remote shell without quoting/escaping.
- Keep remote command set narrow.
- Do not run arbitrary user-specified remote commands in v0.
- Do not weaken SSH host-key checking by default.
- Do not require agent forwarding.
- Do not expose a network listener.
- Observation commands must not type into tmux panes.
- Read-only attach must use tmux `-r`.

---

## 12. Suggested file layout

```text
remux/
  Cargo.toml
  README.md
  examples/
    config.yaml
  fixtures/
    config/
      valid.yaml
      duplicate-host.yaml
      duplicate-session.yaml
      missing-host-ref.yaml
      invalid-duration.yaml
      local-tmux.yaml
  src/
    main.rs
    cli.rs
    config.rs
    model.rs
    error.rs
    host.rs
    transport/
      mod.rs
      local.rs
      ssh.rs
    tmux/
      mod.rs
      parser.rs
      command.rs
    git.rs
    snapshot.rs
    render.rs
    attach.rs
    cache.rs
```

---

## 13. Definition of done for v0

v0 is done when:

```bash
remux list
remux inspect <session-id>
remux capture <session-id>
remux attach --readonly <session-id>
remux attach <session-id>
```

work for:

- one local tmux session
- one SSH remote tmux session

And:

- unreachable hosts do not hang
- missing sessions are clearly marked
- output is factual
- no risk/trust scoring exists
- validation commands pass
- README explains setup in under 5 minutes

---

## 14. First task for build agent

Start with a remote tracer bullet instead of a config-only foundation.

The first milestone should prove that the riskiest end-to-end workflow works:

```bash
remux snapshot pi --json
remux snapshot pi
remux inspect pi/<session>:<window>.<pane>
remux inspect pi/<session>:<window>.<pane> --json
remux capture pi/<session>:<window>.<pane> --lines 120
```

Use this example host:

```yaml
hosts:
  - id: pi
    type: ssh
    ssh:
      target: cam@192.168.0.197
      # Optional escape hatch for environments with broken system SSH config.
      # Omit this to reuse normal SSH config.
      config_file: /dev/null
      options:
        BatchMode: "yes"
        ConnectTimeout: "5"
```

### Build

Implement:

- Rust CLI skeleton with `hosts`, `snapshot <host> [--json]`, `inspect <pane-target> [--json]`, and `capture <pane-target> [--lines N]`.
- Minimal YAML config loading from `~/.config/remux/config.yaml`, with `--config <path>` override.
- Host validation for this milestone.
- SSH execution through the system `ssh` command.
- Non-interactive SSH defaults: `BatchMode=yes` and `ConnectTimeout=5`.
- Remote tmux inventory using:

```bash
tmux list-panes -a -F '#S\t#I\t#P\t#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}'
```

- Discovered pane target IDs:

```text
<host>/<tmux-session>:<window>.<pane>
```

Example:

```text
pi/codex:0.1
```

- `snapshot <host>` discovers all tmux panes on that host and renders both human and JSON output.
- `inspect <pane-target>` parses a discovered target ID, refreshes the host inventory, finds the pane, captures recent output, and renders human or JSON detail.
- `capture <pane-target> --lines N` captures recent output from the remote pane.

Do not require configured `sessions` for the tracer bullet. Friendly session IDs, repo/git metadata, cache-based state, local support, attach behavior, and TUI work are follow-up phases.

### Validation

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- --config fixtures/config/pi.yaml hosts
cargo run -- --config fixtures/config/pi.yaml snapshot pi
cargo run -- --config fixtures/config/pi.yaml snapshot pi --json
cargo run -- --config fixtures/config/pi.yaml inspect 'pi/<session>:<window>.<pane>'
cargo run -- --config fixtures/config/pi.yaml capture 'pi/<session>:<window>.<pane>' --lines 120
```

Pass criteria:

- `hosts` shows `pi` with SSH target `cam@192.168.0.197`.
- `snapshot pi` lists discovered remote tmux panes.
- `snapshot pi --json` emits valid structured JSON.
- `inspect` shows factual pane metadata and recent output for a selected target.
- `capture` prints recent pane output.
- SSH failures are non-interactive and time out quickly.
