---
title: remux Picker, Lifecycle, Session Rollup, Color, TUI Kill
created: 2026-04-26
status: draft
owner: Cam
project: remux
---

# remux Picker, Lifecycle, Session Rollup, Color, TUI Kill

Five additive features, strictly ordered so every read-only capability ships
before any mutating one. Read-only by default stays the product rule.

## Principles

- Observation commands never mutate tmux state.
- Mutation is explicit, named, and confirmed.
- Optional dependencies (fzf) degrade gracefully, never panic.
- No new daemons. Still plain `tmux` + `ssh` + on-disk cache.

## Rollout order

1. `remux pick` — fzf picker with live preview (read-only).
2. `remux sessions` — session-level rollup (read-only).
3. `--color` capture (read-only).
4. `remux new` / `remux kill` — CLI lifecycle (mutating).
5. TUI `k` — kill selected row with confirm (mutating).

Phase A = 1–3. Phase B = 4. Phase C = 5. Each phase is independently
shippable.

---

## 1. `remux pick` — picker with live preview

### Interface

```text
remux pick [--host HOST] [--filter TEXT] [--sessions] [--color] [--no-fzf]
remux p    # alias
```

### Behavior

- Lists all panes (or sessions if `--sessions`) as fzf input rows.
- Live preview on the right showing recent pane output, updated as the
  selection changes.
- Row format keeps the pane target in field 1 so fzf can pass it to the
  preview and action commands.
- Keybindings:

  | Key      | Action |
  | -------- | ------ |
  | `enter`  | Read-only peek (attach with tmux `-r`) |
  | `ctrl-j` | Read-write jump (attach) |
  | `ctrl-o` | Print selected target to stdout and exit |
  | `ctrl-r` | Re-snapshot all hosts and reload rows |
  | `ctrl-c` / `esc` | Abort |

- Preview and dispatch both invoke the current binary (`current_exe()`),
  not `remux` on PATH, so config and behavior stay consistent.
- Attach is **only** triggered by `enter` or `ctrl-j`. Lifecycle keys are
  intentionally absent here.

### fzf-missing handling

- Detect by attempting to spawn `fzf --version`. Treat `NotFound` as
  missing.
- On missing fzf, or when `--no-fzf` is passed:
  - Print a one-shot remediation message to stderr listing install commands
    for macOS, Debian, and Arch.
  - Emit the same tabular rows the picker would have shown to stdout so
    the invocation still produces useful output.
  - Exit with status `2` so scripts can distinguish "fzf absent" from
    generic failure (`1`) and success (`0`).
- No feature of `remux` outside of `pick` depends on fzf.

### Read-only guarantees

- Only runs `tmux list-panes` and `tmux capture-pane` on hosts.
- Never calls `new-session`, `kill-session`, `kill-pane`, or `send-keys`.

### Test strategy

- Fallback path: with fzf unavailable (or `--no-fzf`), assert the rows are
  printed and exit status is `2`.
- Row contract: the first tab-separated field is a valid pane target
  parseable by the existing `PaneTarget::parse`.
- Dispatch: a fake fzf that echoes an expected-key header plus a chosen
  row routes to the correct action (peek vs jump vs print).
- `--sessions` produces one row per `host/session` and selecting it still
  yields a valid pane target.

---

## 2. `remux sessions` — session rollup

### Interface

```text
remux sessions [--host HOST] [--json]
remux list --group sessions   # synonym
```

### Behavior

Pivots the existing pane inventory into one row per `host/session`. No
extra polling — pure view over cached pane data plus one new tmux field
(`session_attached`).

Columns:

| Column      | Meaning |
| ----------- | ------- |
| `host`      | Configured host id |
| `session`   | tmux session name |
| `windows`   | Distinct window count |
| `panes`     | Pane count |
| `attached`  | `true` if any client is attached |
| `state`     | Most-active pane state: `active` > `quiet` > `idle` > `unknown` > `missing` > `unreachable` |
| `match`     | `matched` if any pane matches a watch, else `orphan` |
| `active_cmd`| `pane_current_command` of the session's active pane |
| `repo`      | Repo value if all watched panes agree, else `-` |

### Read-only guarantees

No new remote commands beyond extending the existing `list-panes` format
string with one field.

### Test strategy

- Rollup counts windows and panes correctly given a fixture with multiple
  windows per session.
- `attached` reflects the inventory field.
- State rollup picks the most-active pane among the session's panes.
- JSON shape is stable and documented in `remux sessions --json` output.

---

## 3. `--color` capture

### Interface

Add `--color` to:

```text
remux capture <id> [--lines N] [--color]
remux pick            [--color]    # forwards into the preview
remux inspect <id>    [--color]    # affects only the human preview block
```

### Behavior

- With `--color`, the remote `tmux capture-pane` is invoked with `-e` so
  ANSI escapes are preserved.
- Default is **off** so existing ASCII consumers are unaffected.
- Activity-aging remains stable: the internal poller always captures
  without color so cache hashes do not shift when users ask for colored
  output.

### Test strategy

- Generated capture command includes `-e` only with `--color`.
- Hashes written to the cache are unchanged whether or not a user runs
  `remux capture --color` in parallel.

---

## 4. `remux new` / `remux kill` — lifecycle (CLI)

### Interface

```text
remux new  <host> <session-name> [--cwd PATH] [--window-name NAME]
remux kill <target>              [--yes]
```

`<target>` for `kill` resolves a watch id, `host/session`, or
`host/session:window.pane`.

### Behavior

- `new` runs `tmux new-session -d -s <name>` on the host (local or ssh).
  Refuses to run if a session with that name already exists in the latest
  snapshot. Never runs arbitrary startup commands.
- `kill` runs `tmux kill-session` for a session target or `tmux kill-pane`
  for a pane target.
- Confirmation:
  - On a TTY without `--yes`, prompts `kill <target>? [y/N]`.
  - On a non-TTY without `--yes`, refuses and exits `2`.
- Verbose mode (`-v`) prints the resolved remote command before executing.

### Exit codes

| Code | Meaning |
| ---- | ------- |
| 0    | Success |
| 1    | Generic failure (ssh/tmux error) |
| 2    | Refused (missing `--yes` on non-TTY; name already exists) |
| 3    | Target not found or ambiguous |

### Safety

- Never kills a watch that resolves to `missing`, `ambiguous`, `shadowed`,
  or `unreachable`; exits `3` with a message.
- Never sends `send-keys` or arbitrary input into panes.

### Test strategy

- `new` on local and ssh hosts issues a detached `new-session` and rejects
  duplicate names.
- `kill` routes session vs pane targets to the correct tmux subcommand.
- `kill` without `--yes` on a non-TTY exits `2` without invoking tmux.
- Ambiguous or missing targets exit `3`.

---

## 5. TUI `k` — kill with confirm

### Interface

Add one key to the TUI: `k`.

### Behavior

- `k` opens a single-line footer prompt:
  `kill <resolved-target>? (y/N)`.
- Only `y` / `Y` confirms. Any other key cancels.
- Target is the selected row — pane or session depending on current view.
- Unkillable rows (`missing`, `unreachable`, `ambiguous`, `shadowed`) show
  a status message explaining why and do not prompt.
- On success, the inventory refreshes and the status line shows
  `killed <target>` for one refresh cycle.

### Safety

Dispatches through the same lifecycle primitives as Feature 4. No new
remote commands beyond what Feature 4 already defines.

### Test strategy

- Prompt appears on `k`, cancels on `n`, executes on `y`.
- Unkillable states do not prompt and post a status message.
- After a successful kill, the next snapshot no longer contains the
  target.

---

## 6. Docs and config

- `README.md` gains entries for `remux pick`, `remux sessions`,
  `remux new`, `remux kill`, a TUI `k` line, and an fzf optional-dependency
  note.
- No changes to config schema.

## 7. Out of scope

- AI summaries, scoring, orchestration.
- `send-keys` or programmatic input.
- Renaming sessions.
- Windows support.
- Remote daemons.

## 8. Open questions

1. Should `remux pick` default to `--sessions` when launched from inside
   an existing tmux client?
2. Should the fzf fallback output match `remux list` exactly, or keep the
   richer tab row format so pipelines stay stable?
3. Should `--color` also flow into the TUI preview pane (requires an
   ANSI-to-TUI renderer dependency)?
4. Should `kill <host/session>` ever iterate panes instead of calling
   `kill-session`? Current answer: no, `kill-session` wins.
