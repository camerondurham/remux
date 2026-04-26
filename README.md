# remux

`remux` is a local-first Rust CLI for monitoring tmux sessions across local and SSH hosts.

It is intentionally factual: it inventories tmux panes, captures recent visible output, shows configured repo metadata, and gives you a reliable jump target. It does not summarize, score, orchestrate, or run arbitrary remote commands.

## Quick Start

Create a config:

```bash
mkdir -p ~/.config/remux
cp examples/config.yaml ~/.config/remux/config.yaml
```

Edit the hosts and sessions, then run:

```bash
cargo run -- hosts
cargo run -- list
cargo run -- snapshot pi
cargo run -- inspect pi-agent
cargo run -- capture pi-agent --lines 200
```

For a direct discovered pane target, use:

```bash
cargo run -- inspect 'pi/work:0.1'
cargo run -- capture 'pi/work:0.1'
```

## Config

Default config path:

```text
~/.config/remux/config.yaml
```

Use another config with:

```bash
remux --config ./examples/config.yaml list
```

Hosts can be local or SSH. SSH uses the system `ssh` command and defaults to non-interactive behavior:

```yaml
hosts:
  - id: local
    type: local

  - id: pi
    type: ssh
    ssh:
      target: cam@192.168.0.197
      options:
        BatchMode: "yes"
        ConnectTimeout: "5"
```

Configured sessions get friendly IDs:

```yaml
sessions:
  - id: pi-agent
    host: pi
    tmux:
      session: work
      window: 0
      pane: 1
    repo: /home/cam/openclaw
    agent_hint: codex
```

## Commands

```bash
remux hosts
remux snapshot <host> [--json]
remux list [--json]
remux inspect <session-id-or-pane-target> [--json]
remux capture <session-id-or-pane-target> [--lines N]
remux attach [--readonly] <session-id>
```

Aliases:

```bash
remux ls
remux i <session-id>
remux a <session-id>
```

## Development

Run the local checks:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

Or use `just`:

```bash
just ci
just run-hosts
just run-list fixtures/config/pi.yaml
```

The integration test uses a fake `ssh` binary, so it does not require a live remote host.
