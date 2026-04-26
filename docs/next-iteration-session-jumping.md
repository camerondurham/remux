---
title: remux Next Iteration - Full Session Jumping
created: 2026-04-26
status: draft
owner: Cam
project: remux
---

# Next Iteration: Full Session Jumping

## Goal

Make `remux` a reliable jump surface for local and remote tmux sessions while keeping passive discovery read-only.

Observation commands must stay read-only:

- `remux hosts`
- `remux list`
- `remux snapshot`
- `remux inspect`
- `remux capture`
- passive `remux tui` polling

Intentional attach actions may enter a session:

- `remux attach --readonly <id-or-target>` peeks without input.
- `remux attach <id-or-target>` jumps read-write.
- TUI `enter` peeks read-only.
- TUI `a` jumps read-write.

## Build Tasks

### 1. Clarify Attach Semantics

- Keep `remux attach --readonly <id-or-target>` as read-only peek mode.
- Treat `remux attach <id-or-target>` as the explicit read-write jump mode.
- Consider adding `remux jump <id-or-target>` as an alias for read-write attach if it makes the UX clearer.
- Do not let passive discovery commands mutate remote tmux state.

### 2. Re-resolve TUI Jumps Before Attaching

- Update `src/tui.rs`.
- When the selected row has a `watch_id`, resolve it through the existing action path before attaching.
- Do not attach watched rows directly from stale cached `raw_target` values.
- Preserve direct raw-target attach for orphan rows such as `pi/work:0.1`.
- Refuse rows with these states:
  - `missing`
  - `ambiguous`
  - `shadowed`
  - `unreachable`
  - `unknown`
- Show a clear TUI status message explaining why a selected row cannot attach.

### 3. Add Read-write Attach Coverage

- Update `tests/e2e.rs`.
- Extend the fake SSH script to accept remote read-write attach:

```text
tmux attach-session -t 'work' \; select-window -t '0' \; select-pane -t '1'
```

- Add an e2e assertion for:

```bash
remux attach codex-agent
```

- Add a local e2e assertion for:

```bash
remux a local-agent
```

- Keep the existing read-only attach tests.

### 4. Make Attach Pane-id Aware

- Extend the attach action path so resolved watched rows can use `pane_id` when available.
- Prefer selecting by tmux pane id:

```text
select-pane -t '%3'
```

- Keep session/window/pane fallback for direct raw targets and older paths.
- Avoid changing the external raw target format unless there is a strong reason.

### 5. Improve Local tmux Behavior

- Update `src/attach.rs`.
- Detect when `remux` is already running inside tmux via `$TMUX`.
- Inside tmux, prefer `tmux switch-client -t <session>` followed by pane selection.
- Outside tmux, keep using `tmux attach-session`.
- Preserve read-only behavior for non-tmux terminals.

### 6. Harden Remote Interactive SSH

- Keep using system `ssh`.
- Ensure remote read-write attach uses a TTY with `ssh -t`.
- Keep configured SSH options, host aliases, ports, and config files working.
- Do not force interactive auth prompts for polling or jumping.
- Improve errors so a user can tell the difference between:
  - SSH connection failure
  - remote tmux command failure
  - unresolved watch or stale pane target

### 7. Update TUI Copy

- Keep the help panel explicit:
  - `enter` means read-only attach.
  - `a` means read-write jump.
- Update selected-row attach hints to show both actions for attachable rows:

```text
enter readonly | a jump
```

- For non-attachable rows, show the refusal reason:
  - missing live pane
  - ambiguous candidates
  - shadowed by another watch
  - host unreachable

### 8. Update README

- Document the distinction between peek and jump:

```bash
remux attach --readonly <id-or-target>
remux attach <id-or-target>
```

- Document TUI keys:

```text
enter readonly attach | a read-write jump | r refresh | / filter | c capture | i inspect | q quit
```

- Keep the security section explicit that read-write attach only happens from intentional attach/jump actions.

### 9. Validate

Run the standard local checks:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

Then validate against a real SSH tmux host:

```bash
remux list
remux attach --readonly <watch-id>
remux attach <watch-id>
remux tui
```

In the TUI, verify:

- `enter` attaches read-only.
- `a` attaches read-write.
- watched rows re-resolve before attaching.
- ambiguous, missing, shadowed, and unreachable rows do not attach.

## Acceptance Criteria

- Read-write CLI attach works for local and SSH hosts.
- Read-only CLI attach still works for local and SSH hosts.
- TUI read-write jump works from selected matched watches and orphan raw targets.
- TUI attach re-resolves watched rows before jumping.
- Pane-id selection is preferred when available.
- Stale or unsafe rows fail clearly instead of attaching to the wrong pane.
- Existing snapshot/list/inspect/capture behavior remains read-only.
- README accurately documents the jump behavior and safety model.
