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

The mock data intentionally shows local and SSH hosts, the KPI header, topology tree, live table, inspector detail, attention states, sort/filter state, repo dirty counts, output previews, attach/capture hints, and lifecycle prompts. Update the data in `generate-demo-assets.mjs` when the README needs a new public demo story or when the TUI layout changes enough that the current assets are misleading.
