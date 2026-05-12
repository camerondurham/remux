# remux

`remux` is a local-first CLI/TUI for finding, inspecting, and attaching to tmux panes across local and SSH hosts.

![remux TUI demo: default browse view, filter entry, selection moving, and the help overlay](docs/assets/remux-tui-demo.gif)

![remux TUI: single-line summary, live pane table (NAME/AGE/CMD/PREVIEW) with colored state glyphs, and a right-hand context rail showing the selected pane](docs/assets/remux-tui.png)

`remux` is for engineers who keep shells, builds, coding agents, bots, and debug sessions alive in tmux across multiple machines and want one factual place to find them again.

## Quick start

### Install

#### Homebrew

```bash
brew tap camerondurham/tap
brew install remux
```

#### Nix

```bash
nix profile install github:camerondurham/remux
```

#### Build from source

```bash
cargo install --path .
mkdir -p ~/.config/remux
cp examples/config.yaml ~/.config/remux/config.yaml
```

### Configure

Generate a starter config:

```bash
remux onboard
remux onboard --write
```

If you want to limit the generated SSH hosts:

```bash
remux onboard --hosts pi,prod --write
```

`remux onboard` scans `~/.ssh/config`, generates a minimal `~/.config/remux/config.yaml`, and leaves a commented watch example you can fill in later.

Or edit `~/.config/remux/config.yaml` directly:

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

### Use it

```bash
remux onboard
remux doctor
remux list
remux tui
```

Once you know a pane target or add a watch ID, you can inspect or attach directly:

```bash
remux inspect 'pi/work:0.1'
remux attach --readonly 'pi/work:0.1'
```

Pane targets look like:

```text
pi/work:0.1
```

Direct pane targets work anywhere a watch ID works:

```bash
remux inspect 'pi/work:0.1'
remux capture 'pi/work:0.1'
remux attach --readonly 'pi/work:0.1'
remux attach 'pi/work:0.1'
```

## What it does

- indexes tmux panes across local and SSH hosts
- shows command, cwd, repo, activity, and match state
- lets you inspect output, capture panes, and jump in read-only or read-write
- gives friendly watch IDs for important panes you revisit often

## What it does not do

- no daemon
- no cloud sync
- no agent orchestration
- no AI summaries or scoring

## Core commands

```bash
remux onboard [--hosts HOST[,HOST...]] [--write] [--force]
remux hosts
remux doctor [--json]
remux list [--json] [--group panes|sessions]
remux sessions [--host HOST] [--json]
remux snapshot <host> [--json]
remux inspect <watch-id-or-pane-target> [--json] [--color]
remux capture <watch-id-or-pane-target> [--lines N] [--color]
remux attach --readonly <watch-id-or-pane-target>
remux attach <watch-id-or-pane-target>
remux pick [--host HOST] [--filter TEXT] [--sessions] [--color] [--no-fzf]
remux tui [--host HOST] [--filter TEXT]
remux new <host> <session-name> [--cwd PATH] [--window-name NAME]
remux kill <watch-id-or-pane-target> [--yes]
```

Aliases:

```bash
remux ls
remux p
remux i <watch-id-or-pane-target>
remux a [--readonly] <watch-id-or-pane-target>
```

## Requirements

- tmux on each monitored host
- ssh for remote hosts
- fzf only for `remux pick`
- git only if repo metadata is configured
- Rust only when building from source

## Why

When you run long-lived tmux sessions across local and remote machines, it gets annoyingly easy to lose track of what is running where.

`remux` gives you one factual view over those panes without asking you to adopt a daemon, cloud service, or new orchestration model. It reuses tmux and SSH.

## Configuration notes

- hosts can be local or SSH
- `remux onboard` reuses your SSH aliases by default, so `ssh pi` can become `target: pi`
- watches give important panes stable IDs
- match fields are exact and combined with AND semantics
- default config path is `~/.config/remux/config.yaml`
- legacy `sessions` entries are still accepted as exact tmux-coordinate watches

## TUI keys

Main keys:

```text
[↑↓] move  [Enter] attach ro  [a] jump rw  [i] refresh  [/] filter  [d] details  [?] help  [x] kill  [q] quit
```

More keys available in the help overlay (`?`):

| Key | Action |
| --- | --- |
| `j` / `k` | Select next / previous row |
| `r` | Re-poll every configured host |
| `s` | Cycle the table sort mode |
| `c` | Capture selected pane output into the detail view |
| `e` | Rename the selected session |
| `n` | Create a new tmux session on a host (`<host>/<session>`) |
| `p` | Spawn a new pane in an existing session |
| `d` | Toggle the detail pane |
| `Esc` | Close the current overlay |

## Status semantics

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

The TUI collapses activity + match status into a smaller vocabulary:

| TUI state | Derived from |
| --- | --- |
| `ready` | `active` |
| `busy` | `quiet` |
| `stale` | `idle` |
| `drift` | `shadowed` |
| `missing` | `missing` or `unreachable` |
| `ambiguous` | `ambiguous` |
| `-` | `unknown` with no other signal |

## SSH and security

`remux` uses your system `ssh` binary and normal SSH config. It does not install a remote daemon or open inbound ports.

For SSH hosts, observation is limited to generated commands for:

- `tmux list-panes -a`
- `tmux capture-pane`
- `git rev-parse` and `git status --porcelain=v1`

SSH polling defaults to `BatchMode=yes`, `ConnectTimeout=<poll.ssh_timeout>`, and `poll.command_timeout`. Host key checking is not disabled by default. Remote commands run as the configured SSH user.

Attach is explicit:

- `remux attach --readonly ...` uses `tmux attach-session -r`
- read-write attach only happens when requested
- `remux new` and `remux kill` are explicit mutations

### Speed up remote polling with SSH multiplexing

```text
Host your-remote-hosts
    ControlMaster auto
    ControlPath ~/.ssh/cm-%r@%h:%p
    ControlPersist 10m
```

That keeps subsequent SSH polls fast by reusing the connection.

## Releases

Tagged `v*` pushes publish prebuilt archives to GitHub Releases for:

- Linux x86_64
- Linux aarch64
- macOS aarch64

Each release includes per-archive SHA-256 files plus a combined `SHA256SUMS` manifest.

## Known limitations

- alpha-quality TUI
- no Windows support claimed
- activity state is based on output hash changes, not semantic task state
- pane capture may write recent terminal output into the local cache
- the name `remux` may need reconsideration before crates.io publishing

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

Integration tests use fake `ssh` and `tmux` binaries, so they do not require a live remote host.
