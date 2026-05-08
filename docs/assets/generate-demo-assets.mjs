#!/usr/bin/env node
// Generate README assets (PNG screenshot + 4-frame GIF) that match the
// current remux TUI layout: a single-line top summary, a 4-column live
// table (NAME / AGE / CMD / PREVIEW) with a glyph + selector prefix, an
// optional right-side context rail, and a single-line footer.
//
// Source of truth: src/tui.rs (`draw_summary`, `draw_live_table`,
// `draw_context_rail`, `draw_status`). Keep this file in sync with those
// functions when the TUI layout changes.

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const assetDir = scriptDir;
const workDir = resolve(process.env.REMUX_ASSET_WORKDIR || "/tmp/remux-demo-assets");

// ---------------------------------------------------------------------------
// Canvas + geometry
// ---------------------------------------------------------------------------

const WIDTH = 1280;
const HEIGHT = 720;

const FONT_SIZE = 14;
const FONT_FAMILY = '"SFMono-Regular", "JetBrains Mono", "Menlo", "Consolas", "Liberation Mono", monospace';
const CHAR_W = FONT_SIZE * 0.6; // empirical avg for monospace 14px
const LINE_H = 20;

// Reserve 1 line at top for the summary, 1 line at bottom for the footer.
const SUMMARY_Y = 26; // text baseline
const FOOTER_Y = 706; // text baseline

// Body region between summary and footer.
const BODY_TOP = 46;
const BODY_BOTTOM = 690;
const BODY_H = BODY_BOTTOM - BODY_TOP;

// 68% / 32% horizontal split (matches Constraint::Percentage(68|32) in tui.rs).
const PAD_LEFT = 14;
const PAD_RIGHT = 14;
const BODY_W = WIDTH - PAD_LEFT - PAD_RIGHT;
const TABLE_W = Math.floor(BODY_W * 0.68);
const RAIL_X = PAD_LEFT + TABLE_W; // left edge of context rail (border lives here)
const RAIL_TEXT_X = RAIL_X + 14; // text inside rail starts past the border
const RAIL_W = WIDTH - PAD_RIGHT - RAIL_X;

// ---------------------------------------------------------------------------
// Palette — approximates ratatui's named colors as Apple Terminal / iTerm2
// would render them on a dark profile. These are chosen for SVG readability,
// not a cycle-accurate terminal reproduction.
// ---------------------------------------------------------------------------

const palette = {
  bg: "#0d1117",
  text: "#d7dde5",
  dim: "#7d8590",       // Color::DarkGray (muted)
  label: "#c9d1d9",     // Color::Gray bold (context rail labels)
  header: "#e6edf3",    // bold header row

  // canonical_state() color mapping (matches canonical_state_style in tui.rs)
  ready: "#7ee787",        // Color::Green
  stale: "#d0c040",        // Color::Yellow
  busy: "#ffe066",         // Color::LightYellow
  drift: "#8ab4ff",        // Color::LightBlue
  missingBold: "#ff6b6b",  // Color::Red + BOLD
  ambiguous: "#d58cff",    // Color::Magenta
  neutral: "#6e7681",      // Color::DarkGray ("-" state)

  // Summary / status line accents
  cyan: "#79d7ff",         // Color::Cyan (mode label, ◆ watched)
  white: "#e6edf3",        // Color::White (• free)
  red: "#ff6b6b",          // Color::Red (!N issues)

  // Selected row (row_highlight_style in draw_live_table)
  selectedBg: "#9ee7ff",   // Color::LightCyan bg
  selectedFg: "#081019",   // Color::Black fg (bold)

  // Socket indicator ("i " LightBlue bold, before name, when socket is set)
  socketBlue: "#8ab4ff",
};

// ---------------------------------------------------------------------------
// SVG primitives
// ---------------------------------------------------------------------------

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function rect(x, y, w, h, fill, opts = {}) {
  const rx = opts.rx == null ? 0 : opts.rx;
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${rx}" fill="${fill}"/>`;
}

function line(x1, y1, x2, y2, stroke, strokeWidth = 1) {
  return `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${stroke}" stroke-width="${strokeWidth}"/>`;
}

function tspan(value, { fill, weight } = {}) {
  const attrs = [];
  if (fill) attrs.push(`fill="${fill}"`);
  if (weight) attrs.push(`font-weight="${weight}"`);
  return `<tspan ${attrs.join(" ")}>${escapeXml(value)}</tspan>`;
}

// Render a run of styled parts at (x, y). Each part is { value, fill?, weight? }.
function styledText(x, y, parts, { size = FONT_SIZE, defaultFill = palette.text } = {}) {
  const inner = parts
    .map((p) => tspan(p.value, { fill: p.fill || defaultFill, weight: p.weight }))
    .join("");
  return `<text x="${x}" y="${y}" font-size="${size}" fill="${defaultFill}" xml:space="preserve">${inner}</text>`;
}

// Plain single-color text.
function plainText(x, y, value, { fill = palette.text, weight, size = FONT_SIZE } = {}) {
  const w = weight ? ` font-weight="${weight}"` : "";
  return `<text x="${x}" y="${y}" font-size="${size}" fill="${fill}"${w} xml:space="preserve">${escapeXml(value)}</text>`;
}

function truncate(value, max) {
  const s = String(value);
  if (s.length <= max) return s;
  if (max <= 1) return s.slice(0, max);
  return `${s.slice(0, max - 1)}…`;
}

// ---------------------------------------------------------------------------
// Mock data
//
// The mock deliberately covers several canonical states so the screenshot
// shows the full color vocabulary: matched/ready, matched/busy (quiet),
// matched/stale (idle), orphan/ready, orphan/stale, and one unreachable
// host producing a missing row. Names are generic so this image can live
// in public docs.
// ---------------------------------------------------------------------------

// Host list mirrors `app.host_progress` in tui.rs. An "unreachable" host is
// still counted as "done" by the real TUI (its poll finished, just with an
// error), so all four hosts appear in the `N/N hosts` summary even though one
// will produce a missing row.
const hosts = [
  { id: "local", status: "ok" },
  { id: "pi", status: "ok" },
  { id: "lab", status: "ok" },
  { id: "buildbox", status: "unreachable" },
];

// Each row mirrors the fields draw_live_table + draw_context_rail read.
// Derived values:
//   - glyph from match ("matched" -> ◆, "orphan" -> •, else !)
//   - canonical state label + color from state (see canonical_state in tui.rs)
const rows = [
  {
    display: "local/agent:0.0",
    host: "local",
    socket: null,
    match: "matched",
    state: "ready",
    age: "5s",
    cmd: "codex",
    preview: "regenerating README assets",
    captured: "5s ago",
    output: [
      "cargo test --locked",
      "reading src/tui.rs",
      "updated four-panel dashboard",
      "regenerating README assets",
    ],
  },
  {
    display: "local/build:1.0",
    host: "local",
    socket: null,
    match: "matched",
    state: "stale",
    age: "4m",
    cmd: "cargo",
    preview: "Finished `dev` profile [unoptimized + debuginfo]",
    captured: "4m ago",
    output: [
      "cargo test --locked --all-targets",
      "   Compiling remux v0.1.0",
      "    Finished `dev` profile [unoptimized + debuginfo]",
    ],
  },
  {
    display: "local/ops:1.0",
    host: "local",
    socket: null,
    match: "orphan",
    state: "stale",
    age: "2h",
    cmd: "zsh",
    preview: "waiting for next maintenance window",
    captured: "2h ago",
    output: [
      "last deploy succeeded",
      "waiting for next maintenance window",
    ],
  },
  {
    display: "pi/agent:0.1",
    host: "pi",
    socket: null,
    match: "matched",
    state: "busy",
    age: "38s",
    cmd: "node",
    preview: "agent: editing transport adapter",
    captured: "38s ago",
    output: [
      "npm test",
      "agent: editing transport adapter",
      "all checks queued",
    ],
  },
  {
    display: "pi/debug:0.0",
    host: "pi",
    socket: null,
    match: "orphan",
    state: "ready",
    age: "12s",
    cmd: "bash",
    preview: "$ journalctl -u remux -f",
    captured: "12s ago",
    output: [
      "$ journalctl -u remux -f",
      "May 08 09:15:41 pi remux[8842]: polled 3/3 hosts",
    ],
  },
  {
    display: "pi/ops:0.0",
    host: "pi",
    socket: "~/.work-os/tmux.sock",
    match: "matched",
    state: "ready",
    age: "1m",
    cmd: "bash",
    preview: "serving 200 req/s",
    captured: "1m ago",
    output: [
      "while true; do curl -sf localhost:8080/health; sleep 5; done",
      "serving 200 req/s",
    ],
  },
  {
    display: "lab/ai:2.0",
    host: "lab",
    socket: null,
    match: "orphan",
    state: "busy",
    age: "6s",
    cmd: "python",
    preview: "processed 12 jobs",
    captured: "6s ago",
    output: [
      "python run_eval.py --suite smoke",
      "processed 12 jobs",
      "collecting traces",
    ],
  },
  {
    display: "lab/ai:2.1",
    host: "lab",
    socket: null,
    match: "orphan",
    state: "ready",
    age: "3s",
    cmd: "python",
    preview: "tail -f logs/trace.log",
    captured: "3s ago",
    output: [
      "tail -f logs/trace.log",
      "collecting traces",
    ],
  },
  {
    display: "buildbox/build:0.0",
    host: "buildbox",
    socket: null,
    match: "missing",
    state: "missing",
    age: "-",
    cmd: "-",
    preview: "ssh: connect: no route to host",
    captured: null,
    output: null,
  },
];

// ---------------------------------------------------------------------------
// Derived helpers
// ---------------------------------------------------------------------------

function rowGlyph(row) {
  if (row.match === "matched") return "◆";
  if (row.match === "orphan") return "•";
  return "!";
}

function stateColor(state) {
  switch (state) {
    case "ready": return palette.ready;
    case "stale": return palette.stale;
    case "busy": return palette.busy;
    case "drift": return palette.drift;
    case "ambiguous": return palette.ambiguous;
    case "missing": return palette.missingBold;
    default: return palette.neutral;
  }
}

function rowStateWeight(state) {
  return state === "missing" ? "700" : undefined;
}

function filterRows(filter) {
  const needle = (filter || "").trim().toLowerCase();
  if (!needle) return rows;
  return rows.filter((row) =>
    [row.display, row.host, row.cmd, row.preview].join(" ").toLowerCase().includes(needle),
  );
}

// ---------------------------------------------------------------------------
// Drawing functions
// ---------------------------------------------------------------------------

function drawBackground() {
  return rect(0, 0, WIDTH, HEIGHT, palette.bg);
}

// `remux | / <filter> | N panes  •F free  ◆W watched  [!P issues] | N/N hosts Xs | <mode>`
function drawSummary({ rows: currentRows, filter, mode, elapsed }) {
  const watched = currentRows.filter((r) => r.match === "matched").length;
  const free = currentRows.filter((r) => r.match === "orphan").length;
  const issues = currentRows.filter(
    (r) => !["matched", "orphan"].includes(r.match),
  ).length;
  const hostsUp = hosts.filter((h) => h.status === "ok").length;
  const hostsTotal = hosts.length;

  const parts = [
    { value: "remux", weight: "700" },
    { value: " | " },
    { value: `/ ${filter || "-"}` },
    { value: " | " },
    { value: `${currentRows.length} panes  ` },
    { value: `•${free} free`, fill: palette.white },
    { value: "  " },
    { value: `◆${watched} watched`, fill: palette.cyan },
  ];
  if (issues > 0) {
    parts.push({ value: "  " });
    parts.push({ value: `!${issues} issues`, fill: palette.red });
  }
  parts.push({ value: ` | ${hostsUp}/${hostsTotal} hosts ${elapsed}` });
  parts.push({ value: " | " });
  parts.push({ value: mode, fill: palette.cyan });

  return styledText(PAD_LEFT, SUMMARY_Y, parts);
}

// Table header row + body rows. Uses fixed column X offsets, not SVG columns.
function drawTable({ rows: currentRows, selectedIndex }) {
  // Column X offsets within the table pane (relative to PAD_LEFT).
  const col = {
    name: PAD_LEFT,
    age: PAD_LEFT + Math.floor(TABLE_W * 0.55),
    cmd: PAD_LEFT + Math.floor(TABLE_W * 0.62),
    preview: PAD_LEFT + Math.floor(TABLE_W * 0.74),
  };

  const nameCharMax = Math.floor((col.age - col.name - 4) / CHAR_W);
  const cmdCharMax = Math.floor((col.preview - col.cmd - 4) / CHAR_W) - 1;
  const previewCharMax = Math.floor((PAD_LEFT + TABLE_W - col.preview) / CHAR_W) - 1;

  // Header
  const headerY = BODY_TOP + LINE_H;
  let svg = "";
  svg += plainText(col.name, headerY, "NAME", { fill: palette.header, weight: "700" });
  svg += plainText(col.age, headerY, "AGE", { fill: palette.header, weight: "700" });
  svg += plainText(col.cmd, headerY, "CMD", { fill: palette.header, weight: "700" });
  svg += plainText(col.preview, headerY, "PREVIEW", {
    fill: palette.header,
    weight: "700",
  });

  // Rows
  const rowStartY = headerY + LINE_H;
  currentRows.forEach((row, i) => {
    const y = rowStartY + i * LINE_H;
    const selected = i === selectedIndex;

    if (selected) {
      // full-width highlight covers NAME through PREVIEW
      svg += rect(
        col.name - 4,
        y - LINE_H + 4,
        TABLE_W,
        LINE_H,
        palette.selectedBg,
      );
    }

    const glyph = rowGlyph(row);
    const selector = selected ? "› " : "  ";
    const stateFill = selected ? palette.selectedFg : stateColor(row.state);
    const nameFill = selected ? palette.selectedFg : stateColor(row.state);
    const weight = selected ? "700" : rowStateWeight(row.state);

    const nameParts = [
      { value: `${selector}${glyph} `, fill: stateFill, weight },
    ];
    if (row.socket) {
      nameParts.push({
        value: "i ",
        fill: selected ? palette.selectedFg : palette.socketBlue,
        weight: "700",
      });
    }
    nameParts.push({
      value: truncate(row.display, nameCharMax - (row.socket ? 2 : 0)),
      fill: nameFill,
      weight,
    });
    svg += styledText(col.name, y, nameParts);

    svg += plainText(col.age, y, row.age, {
      fill: selected ? palette.selectedFg : palette.text,
      weight: selected ? "700" : undefined,
    });
    svg += plainText(col.cmd, y, truncate(row.cmd, cmdCharMax), {
      fill: selected ? palette.selectedFg : palette.text,
      weight: selected ? "700" : undefined,
    });
    svg += plainText(col.preview, y, truncate(row.preview, previewCharMax), {
      fill: selected ? palette.selectedFg : palette.dim,
      weight: selected ? "700" : undefined,
    });
  });

  return svg;
}

// The right-hand rail shown in wide terminals. Mirrors draw_context_rail.
function drawContextRail({ rows: currentRows, selectedIndex }) {
  let svg = "";
  // Left border (Borders::LEFT on the rail block)
  svg += line(RAIL_X, BODY_TOP, RAIL_X, BODY_BOTTOM, palette.dim, 1);

  // Title "context"
  const titleY = BODY_TOP + LINE_H;
  svg += plainText(RAIL_TEXT_X, titleY, "context", {
    fill: palette.label,
    weight: "700",
  });

  const row = currentRows[selectedIndex] || currentRows[0];
  if (!row) return svg;

  // Labels + values block
  const textMaxChars = Math.floor((RAIL_W - 14 - 8) / CHAR_W);
  const metaLines = [
    { label: "target", value: row.display },
    { label: "host", value: row.host },
    { label: "socket", value: row.socket || "default" },
    { label: "state", value: row.state, valueFill: stateColor(row.state) },
    { label: "cmd", value: row.cmd },
  ];

  let y = titleY + LINE_H;
  for (const meta of metaLines) {
    const labelText = `${meta.label} `;
    const valueText = truncate(meta.value, textMaxChars - labelText.length);
    svg += styledText(RAIL_TEXT_X, y, [
      { value: labelText, fill: palette.label, weight: "700" },
      { value: valueText, fill: meta.valueFill || palette.text },
    ]);
    y += LINE_H;
  }

  // Blank line between meta and capture age
  y += LINE_H / 2;

  if (row.captured) {
    svg += plainText(RAIL_TEXT_X, y, `captured ${row.captured}`, {
      fill: palette.dim,
    });
    y += LINE_H;

    if (row.output) {
      for (const outLine of row.output) {
        svg += plainText(RAIL_TEXT_X, y, truncate(outLine, textMaxChars), {
          fill: palette.text,
        });
        y += LINE_H;
      }
    }
  } else {
    svg += plainText(RAIL_TEXT_X, y, "no capture yet — press [i] to fetch", {
      fill: palette.dim,
    });
    y += LINE_H;
  }

  // Hotkey hint pinned near bottom of rail.
  const hintY = BODY_BOTTOM - 8;
  svg += plainText(
    RAIL_TEXT_X,
    hintY,
    "[i] refresh · [Enter] attach ro · [a] jump rw · [c] copy",
    { fill: palette.dim, size: 13 },
  );

  return svg;
}

// Full-body inspector content rendered when `?` help overlay is active.
function drawHelp() {
  let svg = "";
  svg += plainText(PAD_LEFT, BODY_TOP + LINE_H, "keys", {
    fill: palette.label,
    weight: "700",
  });
  const lines = [
    "[j/↓] select next",
    "[k/↑] select previous",
    "[r] refresh now",
    "[s] cycle table sort",
    "[/] filter",
    "[Enter] readonly attach",
    "[a] read-write jump",
    "[c] capture into detail",
    "[i] inspect and refresh detail",
    "[x] kill selected pane",
    "[e] rename selected session",
    "[n] create session (<host>/<session>)",
    "[p] spawn pane (<host>/<session>)",
    "[d] toggle detail pane",
    "[?] toggle this help",
    "[q] quit",
  ];
  let y = BODY_TOP + LINE_H * 3;
  for (const text of lines) {
    svg += plainText(PAD_LEFT, y, text, { fill: palette.text });
    y += LINE_H;
  }
  return svg;
}

// Bottom line: key hints + mode + short status.
function drawFooter({ mode, status, editingFilter }) {
  const keysText =
    "[↑↓] move  [Enter] attach ro  [a] jump rw  [i] refresh  [/] filter  [d] details  [?] help  [x] kill  [q] quit   ";
  const parts = [
    { value: keysText, fill: palette.text },
    { value: mode, fill: palette.cyan },
    { value: "  " },
    { value: status, fill: palette.dim },
  ];
  if (editingFilter) {
    parts.push({ value: " _", fill: palette.selectedBg });
  }
  return styledText(PAD_LEFT, FOOTER_Y, parts);
}

// ---------------------------------------------------------------------------
// Frame composition
// ---------------------------------------------------------------------------

function renderFrame({
  filter = "",
  selectedIndex = 0,
  summaryMode = "browse",
  footerMode = "ready",
  status = "scan complete: 3/3 hosts | polled in 0.4s · 2s ago",
  elapsed = "polled in 0.4s · 2s ago",
  help = false,
  editingFilter = false,
}) {
  const currentRows = filterRows(filter);
  const clampedSel = Math.min(Math.max(selectedIndex, 0), Math.max(currentRows.length - 1, 0));

  let body = "";
  if (help) {
    body = drawHelp();
  } else {
    body = drawTable({ rows: currentRows, selectedIndex: clampedSel }) +
      drawContextRail({ rows: currentRows, selectedIndex: clampedSel });
  }

  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}">
  <style>text { font-family: ${FONT_FAMILY}; font-variant-ligatures: none; }</style>
  ${drawBackground()}
  ${drawSummary({ rows: currentRows, filter, mode: summaryMode, elapsed })}
  ${body}
  ${drawFooter({ mode: footerMode, status, editingFilter })}
</svg>`;
}

// ---------------------------------------------------------------------------
// Asset pipeline
// ---------------------------------------------------------------------------

async function writeSources() {
  await mkdir(workDir, { recursive: true });

  // Still-frame screenshot: default browse view with a matched row selected.
  const screenshot = renderFrame({
    selectedIndex: 0, // local/agent:0.0
    // summaryMode/footerMode/status use defaults that match real TUI output.
  });
  await writeFile(join(workDir, "remux-tui.svg"), screenshot);

  // GIF story (four frames):
  //   0: default browse, first row selected
  //   1: filter entry — `/` pressed and "pi" typed, cursor visible
  //   2: filter committed, cursor moved to a different pi row
  //   3: `?` help overlay replaces the body
  const frames = [
    screenshot,
    renderFrame({
      filter: "pi",
      selectedIndex: 0,
      summaryMode: "filter",
      footerMode: "filter",
      status: "3 rows match pi",
      editingFilter: true,
    }),
    renderFrame({
      filter: "pi",
      selectedIndex: 2, // inside filtered set -> pi/ops:0.0
      status: "selected pi/ops:0.0",
    }),
    renderFrame({
      status: "help",
      help: true,
    }),
  ];

  for (const [index, frame] of frames.entries()) {
    await writeFile(
      join(workDir, `frame-${String(index).padStart(2, "0")}.svg`),
      frame,
    );
  }
}

function runMagick(args) {
  const result = spawnSync("magick", args, { stdio: "inherit" });
  if (result.error) {
    throw new Error(`failed to run magick: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`magick exited with status ${result.status}`);
  }
}

async function main() {
  await writeSources();

  runMagick([
    join(workDir, "remux-tui.svg"),
    "-strip",
    "+set",
    "date:create",
    "+set",
    "date:modify",
    "-define",
    "png:exclude-chunk=time",
    join(assetDir, "remux-tui.png"),
  ]);

  runMagick([
    "-delay",
    "140",
    "-loop",
    "0",
    join(workDir, "frame-00.svg"),
    join(workDir, "frame-01.svg"),
    join(workDir, "frame-02.svg"),
    join(workDir, "frame-03.svg"),
    "-layers",
    "Optimize",
    join(assetDir, "remux-tui-demo.gif"),
  ]);

  console.log(`wrote ${join(assetDir, "remux-tui.png")}`);
  console.log(`wrote ${join(assetDir, "remux-tui-demo.gif")}`);
  console.log(`kept SVG sources in ${workDir}`);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
