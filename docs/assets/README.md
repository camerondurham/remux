# Demo Asset Workflow

The README image assets are generated, not hand-edited:

- `remux-tui.png`
- `remux-tui-demo.gif`

Use the script in this directory to regenerate both from deterministic mock TUI data:

```bash
node docs/assets/generate-demo-assets.mjs
```

Requirements:

- Node.js
- ImageMagick 7 with the `magick` command available

On NixOS, one-shot usage looks like:

```bash
nix shell nixpkgs#imagemagick nixpkgs#nodejs -c node docs/assets/generate-demo-assets.mjs
```

The script writes temporary SVG sources to `/tmp/remux-demo-assets` by default, then renders the checked-in assets in `docs/assets/`. To inspect or keep the intermediate SVG files somewhere else:

```bash
REMUX_ASSET_WORKDIR=/tmp/remux-assets node docs/assets/generate-demo-assets.mjs
```

The mock data covers local and SSH hosts, the single-line summary bar (pane counts, free/watched/issues, host poll status), the four-column live table with colored state glyphs and a dim `[window-name | pane-title]` chip after each id (sourced from tmux's `#W` / `#{pane_title}` and filtered by `friendly_label_suffix` in `src/tui.rs`), and the right-hand context rail (target/host/socket/state/cmd + captured output preview + hotkey hints). Update the data in `generate-demo-assets.mjs` when the README needs a new public demo story or when the TUI layout drifts from `src/tui.rs` (`draw_summary`, `draw_live_table`, `draw_context_rail`, `draw_status`) enough that the current assets are misleading.
