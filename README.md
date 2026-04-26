# remux

`remux` is a local-first CLI/TUI for finding, inspecting, and attaching to tmux panes across local and SSH hosts.

**Project status: alpha.** `remux` is usable for dogfooding, but the config format, TUI layout, and command behavior may still change.

It is for people who keep coding agents, shells, builds, and debug sessions alive in tmux across several machines. `remux` shows which panes exist, what they are running, where they live on disk, whether output is still changing, and how to attach to the right pane.

https://github.com/user-attachments/assets/7f6a95a3-522d-4037-9db0-697b388cd6d8

## Use Case

`remux` is a live index for tmux work spread across machines. It maps panes to names, process/cwd/repo/output state, and attach/capture commands.

Scope: inventory and jump surface only. No AI summaries, remote daemons, cloud sync, or agent orchestration.

## Quick Start

From this checkout:

```bash
cargo install --path .
mkdir -p ~/.config/remux
cp examples/config.yaml ~/.config/remux/config.yaml
```

Edit `~/.config/remux/config.yaml`, then run:

```bash
remux hosts
remux list
remux tui
```

Useful commands:

```bash
remux snapshot <host> [--json]
remux inspect <watch-id-or-pane-target> [--json]
remux capture <watch-id-or-pane-target> [--lines N]
remux attach [--readonly] <watch-id-or-pane-target>
```

Pane targets look like:

```text
pi/work:0.1
```

## Configuration

Hosts are local or SSH. Watches give live panes friendly IDs. Match fields are exact and combined with AND semantics.

```yaml
poll:
  active_after: 5m
  idle_after: 60m
  capture_lines: 120
  ssh_timeout: 5s
  command_timeout: 15s
  max_concurrency: 4

hosts:
  - id: local
    type: local

  - id: pi
    type: ssh
    ssh:
      target: cam@192.168.0.197

watches:
  - id: pi-agent
    host: pi
    match:
      command: node
      cwd_prefix: /home/cam/openclaw
    repo: /home/cam/openclaw
    agent_hint: codex
```

Default config path: `~/.config/remux/config.yaml`.

Legacy `sessions` entries are still accepted as exact tmux-coordinate watches.

## Status Semantics

Activity state:

| State | Meaning |
| --- | --- |
| `active` | Output changed within `poll.active_after`. |
| `quiet` | Output is unchanged past `active_after`, but before `idle_after`. |
| `idle` | Output is unchanged for at least `poll.idle_after`. |
| `missing` | A configured watch did not match a live pane. |
| `unreachable` | The host could not be polled. |
| `unknown` | No prior cache entry exists yet, or capture failed. |

Watch match state:

| Match | Meaning |
| --- | --- |
| `matched` | One watch resolved to one live pane. |
| `orphan` | A live pane has no matching watch. |
| `missing` | A watch matched no live panes. |
| `ambiguous` | A watch matched multiple panes. |
| `shadowed` | A later watch matched a pane claimed by an earlier watch. |
| `unreachable` | The host for this row could not be polled. |

State aging is based on captured output hashes cached at `~/.local/share/remux/cache.json`.

## SSH And Security

`remux` uses your system `ssh` binary and normal SSH config. It does not install a remote daemon or open inbound ports.

For SSH hosts, observation is limited to generated commands for:

- `tmux list-panes -a`
- `tmux capture-pane`
- `git rev-parse` and `git status --porcelain=v1`

SSH polling defaults to `BatchMode=yes`, `ConnectTimeout=<poll.ssh_timeout>`, and `poll.command_timeout`. Host key checking is not disabled by default. Remote commands run as the configured SSH user.

Attach is always explicit. `remux attach --readonly ...` uses `tmux attach-session -r`; read-write attach only happens when requested.

## TUI Keys

```text
enter attach | r refresh | / filter | c capture | i inspect | q quit
```

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The integration tests use fake `ssh` and `tmux` binaries, so they do not require a live remote host.
