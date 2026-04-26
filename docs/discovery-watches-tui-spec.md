---
title: remux Discovery Watches and TUI Spec
created: 2026-04-26
status: draft
owner: Cam
project: remux
---

# remux Discovery Watches and TUI Spec

## 0. Bottom line

The next version of `remux` should make tmux discovery the source of truth.

The current v0 config can hard-code friendly session IDs to exact tmux session/window/pane coordinates. That works for a quick dogfood pass, but it is too static: panes move, windows split, sessions die, and a useful viewer should still show what exists now.

New direction:

- `remux` always discovers live tmux panes from configured hosts.
- Config decorates live discovery with optional friendly names and matching rules.
- A new `remux tui` subcommand provides an interactive viewer over the same snapshot model used by `list`, `snapshot`, `inspect`, `capture`, and `attach`.
- Config writes only happen through explicit commands, never as a side effect of listing or viewing.

The core design rule:

> Config labels discovery. Config does not drive discovery.

---

## 1. Current problem

Dogfooding exposed that static session config is useful but brittle.

Example current config shape:

```yaml
sessions:
  - id: rpi2-kiro
    host: rpi2
    tmux:
      session: "0"
      window: 1
      pane: 0
    agent_hint: kiro
```

This means "`rpi2-kiro` is exactly `rpi2/0:1.0`." If that pane moves, the alias drifts. The current code can show `missing`, but it cannot rediscover the same logical thing if the command and working directory still identify it.

The desired behavior is:

- `remux list` shows live discovered panes even with no watches.
- A watch such as `rpi2-kiro` attaches to the current live pane matching `kiro-cli` and `/home/cam`, even if the tmux coordinates changed.
- If no pane matches a watch, the watch is shown as `missing`.
- If multiple panes match one watch, the watch is shown as `ambiguous`; do not auto-pick.
- If multiple watches match one pane, config order wins and lower-priority watches are `shadowed`.

---

## 2. Product principles

1. Discovery first.
   Every command should start from live host discovery when practical.

2. Config as overlay.
   Config can provide host definitions, friendly watch IDs, match hints, repo overrides, and display metadata. It should not be required to enumerate panes.

3. Exact before fuzzy.
   Matching should be deterministic and explainable. Start with exact command and cwd matches plus optional cwd prefix.

4. Read-only by default.
   `list`, `snapshot`, `inspect`, `capture`, and `tui` must not mutate tmux sessions or config. Attaching is user-initiated. Config writes require explicit `config` commands.

5. Same data model everywhere.
   The TUI should consume the same resolved snapshot model as the CLI and JSON output.

6. Boring failure modes.
   Unreachable hosts, missing watches, ambiguous watches, and command timeouts must be visible without blocking useful data from other hosts.

---

## 3. Config model

### 3.1 Hosts stay explicit

Hosts remain the configured boundary for discovery.

```yaml
hosts:
  - id: local
    type: local

  - id: rpi2
    type: ssh
    ssh:
      target: cam@192.168.0.197
      options:
        BatchMode: "yes"
        ConnectTimeout: "5"
```

### 3.2 Watches replace static sessions

Add a new top-level `watches` list.

```yaml
watches:
  - id: rpi2-kiro
    host: rpi2
    match:
      command: kiro-cli
      cwd: /home/cam
    agent_hint: kiro

  - id: rpi2-openclaw-agent
    host: rpi2
    match:
      command: node
      cwd_prefix: /home/cam/openclaw
    agent_hint: codex
```

### 3.3 Match fields

Initial supported fields:

```yaml
match:
  command: kiro-cli
  cwd: /home/cam
  cwd_prefix: /home/cam/openclaw
  tmux:
    session: "0"
```

Rules:

- `command` is exact.
- `cwd` is exact.
- `cwd_prefix` matches if the pane cwd is equal to the prefix or under it.
- `tmux.session` is exact.
- All configured match fields are ANDed.
- `cwd` and `cwd_prefix` should not both be set in the same watch.
- Empty `match` is invalid.

Optional later fields, not required in this phase:

```yaml
match:
  window: 1
  pane: 0
  pane_id: "%3"
  preview_contains: "Kiro"
```

Do not implement fuzzy text search in this phase.

### 3.4 Backward compatibility

Keep reading the existing `sessions` config for now if cheap.

Interpret a v0 `sessions` entry as a watch with an exact tmux coordinate match:

```yaml
sessions:
  - id: rpi2-kiro
    host: rpi2
    tmux:
      session: "0"
      window: 1
      pane: 0
```

Equivalent internal watch:

```yaml
watches:
  - id: rpi2-kiro
    host: rpi2
    match:
      tmux:
        session: "0"
        window: 1
        pane: 0
```

Deprecation can wait. Do not remove `sessions` in this work unless the user explicitly approves a breaking change.

---

## 4. Runtime model

### 4.1 Discovered pane

Discovery produces live panes independent of config.

Required fields:

```text
host
target
pane_id
tmux.session
tmux.window
tmux.pane
process.pid
process.command
process.cwd
output.preview
output.hash
output.last_output_at
repo
errors
```

`target` remains display-friendly:

```text
<host>/<session>:<window>.<pane>
```

Example:

```text
rpi2/0:1.0
```

### 4.2 Resolved session row

After watch matching, CLI and TUI should operate on resolved rows:

```text
display_id
raw_target
host
match_status
watch_id
watch_index
candidate_targets
pane
state
repo
errors
```

Suggested JSON shape:

```json
{
  "display_id": "rpi2-kiro",
  "raw_target": "rpi2/0:1.0",
  "host": "rpi2",
  "match_status": "matched",
  "watch_id": "rpi2-kiro",
  "candidate_targets": [],
  "tmux": {
    "session": "0",
    "window": "1",
    "pane": "0",
    "pane_id": "%3"
  },
  "process": {
    "pid": 276644,
    "command": "kiro-cli",
    "cwd": "/home/cam"
  },
  "repo": null,
  "output": {
    "preview": "/copy to clipboard",
    "last_output_at": "2026-04-26T02:42:13Z"
  },
  "errors": []
}
```

### 4.3 Match statuses

Use these statuses:

```text
matched
orphan
missing
ambiguous
shadowed
unreachable
unknown
```

Meanings:

- `matched`: a watch resolved to exactly one live pane.
- `orphan`: a live pane has no matching watch.
- `missing`: a watch has no matching live pane.
- `ambiguous`: a watch matched more than one live pane.
- `shadowed`: a watch matched a pane already claimed by an earlier watch.
- `unreachable`: host polling failed.
- `unknown`: insufficient data.

### 4.4 Conflict rules

If one watch matches multiple live panes:

- Emit one row for the watch.
- `match_status = ambiguous`.
- Include all candidate targets.
- Do not attach automatically.
- `inspect` should show candidate metadata and ask the user to use a raw target or narrow the watch.

If multiple watches match one live pane:

- Earlier config order wins.
- Winner is `matched`.
- Later watches are `shadowed`.
- The shadowed row should include the claimed pane target and the winning watch ID.

If a live pane matches no watch:

- Emit it as `orphan`.
- `display_id` should be the raw target.

---

## 5. Repo detection

Repo metadata should be inferred by default from pane CWD.

For each live pane with a cwd:

```bash
git -C <cwd> rev-parse --show-toplevel
git -C <repo> rev-parse --abbrev-ref HEAD
git -C <repo> status --porcelain=v1
```

Rules:

- If `cwd` is inside a git worktree, show repo metadata.
- If it is not a git repo, repo is null and this is not an error.
- Config may later override repo path, but inference is the default.
- Git calls must use the same host execution timeout behavior as tmux calls.
- Git failure should not fail the entire snapshot.

---

## 6. Command behavior

### 6.1 `remux list`

Always show all discovered panes plus configured watch rows.

Expected columns:

```text
HOST  ID/SESSION  MATCH       PANE     CMD       REPO      STATE    DIRTY  PREVIEW
rpi2  rpi2-kiro   matched     0:1.0    kiro-cli  -         active   -      /copy to clipboard
rpi2  rpi2/0:0.2  orphan      0:0.2    bash      openclaw  idle     2      cam@rpi-2:~/openclaw $
rpi1  rpi1-kiro   unreachable -        -         -         -        -      ssh: No route to host
```

Requirements:

- Empty `watches` must still show discovered panes.
- Configured watches should display friendly IDs when matched.
- Orphans should display raw targets.
- Missing, ambiguous, shadowed, and unreachable watches should appear.
- Do not block all hosts on one broken host.

### 6.2 `remux snapshot <host>`

Poll one host and print live discovered panes plus watch resolution for that host.

Human output should make watch state clear:

```text
Host: rpi2
Collected: ...

WATCH/SESSION           MATCH       TARGET       PANE_ID  CMD       CWD/PREVIEW
rpi2-kiro               matched     rpi2/0:1.0   %3       kiro-cli  /home/cam
  /copy to clipboard
rpi2-openclaw-agent     missing     -            -        -         watch did not match a live pane
rpi2/0:0.2              orphan      rpi2/0:0.2   %1       bash      /home/cam/openclaw
```

### 6.3 `remux inspect <id-or-target>`

Resolve in this order:

1. Watch ID.
2. Raw pane target.

For a matched watch:

- Show watch metadata.
- Show current raw target.
- Show process metadata.
- Show repo metadata if inferred.
- Show recent output.

For an ambiguous watch:

- Show all candidate panes.
- Do not choose one.
- Include a hint to inspect by raw target.

For a missing watch:

- Show watch config and missing status.
- Do not attempt capture.

### 6.4 `remux capture <id-or-target>`

For a matched watch:

- Capture the current matched pane.

For a raw target:

- Capture that exact pane.

For ambiguous/missing/unreachable:

- Fail with a clear error.

### 6.5 `remux attach <id-or-target>`

For a matched watch:

- Attach to the current matched pane.
- Do not use stale config coordinates.

For ambiguous watch:

- Refuse and list candidates.

For missing watch:

- Refuse and say no live pane matched.

For raw target:

- Attach exact target.

Read-only behavior remains:

```bash
remux attach --readonly rpi2-kiro
```

### 6.6 `remux tui`

Launch the interactive TUI.

This is a new subcommand:

```bash
remux tui
remux tui --host rpi2
remux tui --filter kiro
```

---

## 7. TUI MVP

### 7.1 Scope

The first TUI should be a remote tmux inventory and jump surface, not an agent analytics dashboard.

Do include:

- Host health.
- Live discovered pane table.
- Watch match status.
- Current selected pane details.
- Recent output preview.
- Repo branch and dirty count.
- Attach actions.
- Manual refresh.
- Text filter.

Do not include yet:

- Token counts.
- Rate limits.
- AI summaries.
- Agent-specific history.
- Kill session actions.
- Web dashboard.
- Persistent output history.

### 7.2 Layout

Target a `ratatui` implementation similar in spirit to `../abtop`.

Suggested layout:

```text
+ remux - hosts -----------------------------------------------------+
| local ok   mini ok   rpi1 unreachable   rpi2 ok                   |
+-------------------------------------------------------------------+
+ sessions ---------------------------------------------------------+
| HOST  ID                 MATCH      CMD       REPO      STATE      |
| rpi2  rpi2-kiro          matched    kiro-cli  -         active     |
| rpi2  rpi2/0:0.2         orphan     bash      openclaw  idle       |
| rpi1  rpi1-kiro          unreachable -        -         -          |
+-------------------------------------------------------------------+
+ detail -----------------------------------------------------------+
| Target: rpi2/0:1.0  Pane: %3  PID: 276644  CWD: /home/cam         |
| Last output: 2026-04-26T02:42:13Z                                 |
|                                                                   |
| Recent output...                                                  |
+-------------------------------------------------------------------+
```

### 7.3 Key bindings

Initial keys:

```text
j/down      select next
k/up        select previous
r           refresh now
/           filter
enter       attach read-only to selected matched pane
a           attach read-write to selected matched pane
c           capture selected pane into detail panel
q           quit
?           help
```

Attach behavior:

- `enter` should prefer read-only attach.
- `a` should be explicit read-write attach.
- Ambiguous, missing, shadowed, and unreachable rows should not attach.

### 7.4 Polling

TUI polling should be concurrent.

Requirements:

- Use a small thread pool or bounded worker set.
- Limit concurrency with a config or constant. Initial default: 4.
- Each host poll gets normal per-command timeouts.
- Slow or broken hosts update their own status without freezing navigation.
- UI render should continue while polling.

Suggested config:

```yaml
poll:
  max_concurrency: 4
  ssh_timeout: 5s
  command_timeout: 15s
```

This can be TUI-only at first if simpler.

---

## 8. Config adopt commands

Config writes should only happen through explicit commands.

### 8.1 Initial command set

Add a `config` command group later in this feature line:

```bash
remux config suggest
remux config adopt <target> --id <watch-id>
remux config prune-missing
```

Only `suggest` and `adopt` are candidates for the next immediate phase. `prune-missing` can wait.

### 8.2 `remux config suggest`

Print YAML suggestions based on current discovery.

No writes.

Example:

```yaml
watches:
  - id: rpi2-kiro
    host: rpi2
    match:
      command: kiro-cli
      cwd: /home/cam
    agent_hint: kiro
```

### 8.3 `remux config adopt <target>`

Add or update one watch explicitly.

Example:

```bash
remux config adopt rpi2/0:1.0 --id rpi2-kiro --agent kiro
```

Initial behavior:

- Resolve the raw target from current discovery.
- Infer `host`, `command`, and `cwd`.
- Write a watch entry.
- Preserve existing config formatting as much as reasonable, but correctness matters more.
- If exact YAML preservation is too much work, use a documented rewrite format.

Do not write config from `list`, `snapshot`, `inspect`, or `tui`.

---

## 9. Implementation phases

### Phase 1: Discovery and watch model

Goal: make dynamic discovery primary for CLI commands.

Tasks:

1. Add config structs for `watches`.
2. Convert old `sessions` entries into internal exact-coordinate watches.
3. Add `DiscoveredPane` and `ResolvedSession` models.
4. Implement watch matching.
5. Add match statuses: `matched`, `orphan`, `missing`, `ambiguous`, `shadowed`, `unreachable`.
6. Infer repo metadata from pane cwd by default.
7. Update `list`, `snapshot`, `inspect`, `capture`, and `attach` to use resolved live panes.
8. Update JSON output with `display_id`, `raw_target`, `match_status`, `watch_id`, and `candidate_targets`.
9. Add tests for matching and drift.

Phase 1 is complete when the existing dogfood config can remove most static exact pane aliases and still show useful rows.

### Phase 2: TUI

Goal: add `remux tui` over the resolved snapshot model.

Tasks:

1. Add `ratatui` and `crossterm` dependencies.
2. Add `Command::Tui` CLI variant.
3. Build a TUI app state around `ResolvedSession` rows.
4. Poll hosts concurrently with limited concurrency.
5. Render host health, session table, and selected detail panel.
6. Implement key bindings.
7. Attach selected matched pane.
8. Keep TUI read-only except explicit attach.
9. Add smoke tests for non-render data paths; full terminal behavior can be manually validated.

Phase 2 is complete when `remux tui` can replace the `snapshot`/`inspect`/`capture` loop for dogfooding `rpi2-kiro`.

### Phase 3: Config adoption

Goal: make it easy to add durable watches from discovery.

Tasks:

1. Add `remux config suggest`.
2. Add `remux config adopt <target> --id <watch-id>`.
3. Preserve or safely rewrite config.
4. Add tests with temporary config files.

Phase 3 can happen after the TUI if adoption is not blocking.

---

## 10. Acceptance tests

### 10.1 Discovery without watches

Given config with hosts and no watches:

```yaml
watches: []
```

`remux list --json` returns all discovered panes as `orphan`.

### 10.2 Exact watch match

Given one pane:

```text
host=rpi2 command=kiro-cli cwd=/home/cam target=rpi2/0:1.0
```

And watch:

```yaml
- id: rpi2-kiro
  host: rpi2
  match:
    command: kiro-cli
    cwd: /home/cam
```

Then output includes:

```text
display_id=rpi2-kiro
match_status=matched
raw_target=rpi2/0:1.0
```

### 10.3 Pane coordinate drift

Given the same command/cwd appears later at:

```text
rpi2/0:3.2
```

Then `remux attach rpi2-kiro` attaches to `rpi2/0:3.2`.

### 10.4 Missing watch

Given no pane matches `rpi2-kiro`, output includes:

```text
display_id=rpi2-kiro
match_status=missing
```

And capture/attach fail clearly.

### 10.5 Ambiguous watch

Given two panes match the same watch, output includes one `ambiguous` watch row with both candidate targets.

Attach by watch ID must refuse.

### 10.6 Shadowed watch

Given two watches match the same pane, the first is `matched` and the second is `shadowed`.

### 10.7 Unreachable host

Given `rpi1` is unreachable, `remux list` still returns local, mini, and rpi2 data.

Rows associated with rpi1 watches show `unreachable`.

### 10.8 Repo inference

Given pane cwd `/home/cam/openclaw/src` is inside a git repo rooted at `/home/cam/openclaw`, repo metadata is shown without config specifying `repo`.

### 10.9 TUI dogfood

In the current dogfood environment:

```bash
remux tui
```

Should show:

- `local`, `mini`, `rpi1`, and `rpi2` host statuses.
- `rpi2-kiro` as matched when Kiro is running.
- A selected detail panel with target, pid, command, cwd, last output, and recent output.
- `enter` attaches read-only to the current matched pane.

---

## 11. Notes for the next implementing agent

Start with the data model and matcher. Do not begin with UI rendering.

Suggested order:

1. Add `watches` config structs and validation.
2. Introduce internal discovered/resolved models.
3. Keep old output compiling by adapting render code.
4. Update JSON first; human rendering second.
5. Add tests with fake tmux/ssh before touching TUI.
6. Only start `remux tui` after `list --json` has the resolved model.

Be careful with these pitfalls:

- Do not lose raw discovered panes when watches exist.
- Do not auto-resolve ambiguous watches.
- Do not use tmux coordinates as the only cache identity for watches.
- Do not write config unless the user ran a `config` command.
- Do not make one unreachable host block the TUI.
