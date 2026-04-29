# remux

`remux` is a local-first CLI/TUI for finding, inspecting, and attaching to tmux panes across local and SSH hosts.

**Status: alpha.** Built for personal/local use first. Tested with local and SSH tmux hosts. Expect rough edges.

It is for engineers who keep coding agents, shells, builds, bots, and debug sessions alive in tmux across multiple machines. `remux` gives you one factual index of those panes: command, cwd, repo, output activity, match state, and attach/capture targets.

It does not summarize, score, orchestrate, spawn agents, install a daemon, or sync to the cloud.

![remux TUI showing local and SSH tmux sessions](docs/assets/remux-tui.png)

![remux TUI demo filtering and inspecting panes](docs/assets/remux-tui-demo.gif)

## Why

When running multiple coding agents or long-running tasks in tmux across local and remote machines, it is easy to lose track of what is running where.

`remux` gives you a single factual view over those sessions without requiring a daemon, cloud account, new agent framework, or custom workflow. It reuses tmux and SSH.

## How is this different?

| Tool type | Examples | Focus | remux difference |
| --- | --- | --- | --- |
| AI-agent monitors | `abtop` | Local Claude Code/Codex telemetry: tokens, context, rate limits, ports, child processes | `remux` is process-agnostic tmux inventory across local and SSH hosts. |
| Claude tmux dashboards | `recon` | Managing Claude Code sessions in tmux, including switching/spawning/killing/resume workflows | `remux` does not manage agents; it inventories any tmux pane and gives attach/capture targets. |
| Agent orchestrators | Gas Town | Coordinating multiple AI coding agents and persistent multi-agent work state | `remux` does not orchestrate. It observes existing sessions and helps you jump into them. |
| Local tmux agent helpers | `amux`, fzf scripts, shell scripts | Local organization or launching of agent sessions | `remux` adds SSH hosts, watches, match states, repo metadata, capture, and activity aging. |
| Generic tmux wrappers | tmux aliases/wrappers | Shorter tmux commands | `remux` builds a remote session index over tmux panes instead of replacing tmux. |

## Requirements

- tmux on each monitored host
- fzf only for `remux pick`
- ssh for remote hosts
- git only if repo metadata is configured
- Rust only when building from source

## Quick Start

From this checkout:

```bash
cargo install --path .
mkdir -p ~/.config/remux
cp examples/config.yaml ~/.config/remux/config.yaml
```

Edit `~/.config/remux/config.yaml`, then run:

```bash
remux doctor
remux hosts
remux list
remux sessions
remux pick
remux attach --readonly pi-agent
remux attach pi-agent
remux tui
```

Useful commands:

```bash
remux snapshot <host> [--json]
remux inspect <watch-id-or-pane-target> [--json]
remux capture <watch-id-or-pane-target> [--lines N] [--color]
remux attach --readonly <watch-id-or-pane-target>
remux attach <watch-id-or-pane-target>
remux new <host> <session-name> [--cwd PATH] [--window-name NAME]
remux kill <watch-id-or-pane-target> --yes
```

Pane targets look like:

```text
pi/work:0.1
```

Direct pane targets also work:

```bash
remux inspect 'pi/work:0.1'
remux capture 'pi/work:0.1'
remux attach --readonly 'pi/work:0.1'
remux attach 'pi/work:0.1'
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

## Commands

```bash
remux hosts
remux doctor [--json]
remux snapshot <host> [--json]
remux list [--json] [--group panes|sessions]
remux sessions [--host HOST] [--json]
remux pick [--host HOST] [--filter TEXT] [--sessions] [--color] [--no-fzf]
remux inspect <watch-id-or-pane-target> [--json] [--color]
remux capture <watch-id-or-pane-target> [--lines N] [--color]
remux attach --readonly <watch-id-or-pane-target>
remux attach <watch-id-or-pane-target>
remux new <host> <session-name> [--cwd PATH] [--window-name NAME]
remux kill <watch-id-or-pane-target> [--yes]
remux tui [--host HOST] [--filter TEXT]
```

Aliases:

```bash
remux ls
remux p
remux i <watch-id-or-pane-target>
remux a [--readonly] <watch-id-or-pane-target>
```

`attach --readonly` is a peek. `attach` without `--readonly` is an explicit
read-write jump. `pick` uses fzf when available; without fzf, `pick --no-fzf`
prints the same tab-separated rows and exits `2`.

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

## Known Limitations

- Alpha-quality TUI.
- No remote daemon; polling uses generated SSH commands.
- No Windows support claimed.
- Activity state is based on captured output hash changes, not semantic task state.
- Pane capture may write recent terminal output into the local cache.
- No token/context tracking.
- No AI summaries or risk scoring.
- The name `remux` may need reconsideration before crates.io publishing.

## SSH And Security

`remux` uses your system `ssh` binary and normal SSH config. It does not install a remote daemon or open inbound ports.

For SSH hosts, observation is limited to generated commands for:

- `tmux list-panes -a`
- `tmux capture-pane`
- `git rev-parse` and `git status --porcelain=v1`

SSH polling defaults to `BatchMode=yes`, `ConnectTimeout=<poll.ssh_timeout>`, and `poll.command_timeout`. Host key checking is not disabled by default. Remote commands run as the configured SSH user.

Attach is always explicit. `remux attach --readonly ...` uses `tmux attach-session -r`; read-write attach only happens when requested.

Lifecycle commands are explicit mutations. `remux new` creates a detached tmux
session. `remux kill` kills a resolved session or pane, requires confirmation on
a TTY, and requires `--yes` from non-interactive scripts.

## TUI Keys

```text
enter readonly attach | a read-write jump | r refresh | / filter | c capture | i inspect | k kill | q quit
```

Passive discovery commands stay read-only: `hosts`, `list`, `snapshot`,
`inspect`, `capture`, and TUI polling do not enter a remote session. Read-write
attach only happens from intentional CLI attach/jump actions or the TUI `a`
key. Kill only happens from `remux kill` or a confirmed TUI `k` prompt.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The integration tests use fake `ssh` and `tmux` binaries, so they do not require a live remote host.
