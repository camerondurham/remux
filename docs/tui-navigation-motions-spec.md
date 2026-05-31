---
title: remux TUI Navigation Motions
created: 2026-05-31
status: draft
owner: Cam
project: remux
---

# TUI Navigation Motions

## Goal

Make the TUI pane tree faster to navigate with familiar list and Vim-style motions, without making host/session/window group rows actionable.

The TUI already tracks selection as a pane-row index. The navigation work should preserve that model: movement keys select panes only, never group headers.

## Key Bindings

Add these bindings to normal browse mode:

| Key | Behavior |
| --- | --- |
| `Home` | Select first visible pane row. |
| `End` | Select last visible pane row. |
| `g g` | Select first visible pane row. |
| `G` | Select last visible pane row. |
| `PageUp` | Move up one visible page of pane rows. |
| `PageDown` | Move down one visible page of pane rows. |
| `Ctrl-u` | Move up half a visible page of pane rows. |
| `Ctrl-d` | Move down half a visible page of pane rows. |
| `H` | Select the first selectable pane on the current screen. |
| `M` | Select the selectable pane nearest the middle of the current screen. |
| `L` | Select the last selectable pane on the current screen. |

Keep existing keys:

- `j` / `Down`: next pane
- `k` / `Up`: previous pane
- `Esc`: close overlays or quit as currently implemented

`g g` should be implemented as a small key prefix state. Pressing `g` once sets a pending `g` prefix; pressing any key other than `g` clears that prefix and handles the key normally where possible.

## Implementation Notes

Update `src/tui.rs`.

- Add `table_offset: usize` to `App`.
- Add a small pending-key enum or flag for the `g g` motion.
- Keep `App.selected` as the selected pane index. Do not switch action methods to display-row indexes.
- Reuse `live_table_rows(&rows)` as the display-row model for table navigation. Add helper functions to convert:
  - selected pane index -> display row index
  - display row index -> nearest selectable pane index
- In `draw_live_table`, initialize `TableState` with both the selected display index and the persisted `table_offset`.
- In key handling, compute visible table rows from terminal height:
  - body height is terminal height minus summary and status rows
  - subtract one more row for the table header
  - clamp to at least `1`
- After every movement, update `table_offset` so the selected display row remains visible.
- Clamp all motion targets to the first/last selectable pane.

Suggested helpers:

- `App::select_first_pane()`
- `App::select_last_pane()`
- `App::move_selection_by(delta: isize)`
- `App::move_page(delta_pages: isize, visible_rows: usize)`
- `App::move_half_page(delta: isize, visible_rows: usize)`
- `App::select_visible_position(position: VisiblePosition, visible_rows: usize)`
- `App::ensure_selection_visible(visible_rows: usize)`

`VisiblePosition` can be `Top`, `Middle`, or `Bottom`.

For `H/M/L`, target display rows based on `table_offset` and `visible_rows`, then choose the nearest selectable pane:

- Top: first pane at or after `table_offset`.
- Middle: pane closest to `table_offset + visible_rows / 2`.
- Bottom: last pane at or before `table_offset + visible_rows - 1`.

If the visible range contains no selectable pane, fall back to the nearest selectable pane outside the range.

## Tests

Add focused unit tests in `src/tui.rs`:

- `Home`/`gg` selects the first pane.
- `End`/`G` selects the last pane.
- Page movement clamps at list boundaries.
- Half-page movement uses at least one row when the viewport is very small.
- `H/M/L` skip host/session/window group rows and select panes only.
- Pending `g` prefix clears after a non-`g` key.
- Existing selected-row actions still operate on panes after tree navigation.

Run:

```bash
just ci
```

If `just` is unavailable locally, run the exact expanded commands from the `ci` recipe in `justfile`; do not substitute `cargo test` alone.

## Acceptance Criteria

- Users can jump to the top or bottom of the pane list with `gg`/`G` and `Home`/`End`.
- Users can page through large pane trees with `PageUp`, `PageDown`, `Ctrl-u`, and `Ctrl-d`.
- Users can jump to top/middle/bottom of the current table viewport with `H/M/L`.
- Group rows remain non-selectable and never become attach/capture/kill/send targets.
- Help text and README key docs mention the new motions.
- `just ci` passes.
