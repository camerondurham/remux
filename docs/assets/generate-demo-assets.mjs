#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const assetDir = scriptDir;
const workDir = resolve(process.env.REMUX_ASSET_WORKDIR || "/tmp/remux-demo-assets");

const width = 1280;
const height = 760;

const palette = {
  bg: "#0d1117",
  panel: "#111820",
  panel2: "#0f151c",
  border: "#56606c",
  text: "#d7dde5",
  muted: "#7d8590",
  dim: "#6e7681",
  active: "#79d7ff",
  quiet: "#e3b341",
  idle: "#6e7681",
  missing: "#ff6b6b",
  selectedBg: "#9ee7ff",
  selectedText: "#081019",
  dirty: "#f2cc60",
  match: "#76e4f7",
  footerBg: "#0a0f14",
};

const rows = [
  [
    "local",
    "local-codex",
    "matched",
    "active",
    "codex",
    "remux",
    "1",
    "updated pane preview and footer",
  ],
  ["local", "local-build", "matched", "active", "cargo", "remux", "1", "Finished dev profile"],
  ["local", "local/ops:1.0", "orphan", "active", "bash", "-", "-", "last deploy succeeded"],
  ["pi", "pi-agent", "matched", "active", "node", "openclaw", "2", "all checks queued"],
  ["pi", "pi-service", "matched", "active", "bash", "-", "-", "heartbeat ok"],
  ["pi", "missing-debug", "missing", "missing", "-", "-", "-", "watch did not match a live pane"],
  ["pi", "pi/db:1.0", "orphan", "active", "psql", "-", "-", "42"],
  ["buildbox", "build-runner", "matched", "active", "bash", "remux", "1", "copying path to remote cache"],
  ["buildbox", "bot-worker", "matched", "active", "python", "bots", "0", "processed 12 jobs"],
];

const summaryAll = {
  hosts: "3/3 ok",
  panes: "9",
  matched: "6",
  active: "8",
  quiet: "0",
  idle: "0",
  missing: "1",
};

const summaryPi = {
  hosts: "3/3 ok",
  panes: "4",
  matched: "2",
  active: "3",
  quiet: "0",
  idle: "0",
  missing: "1",
};

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function text(x, y, value, options = {}) {
  return `<text x="${x}" y="${y}" fill="${options.color || palette.text}" font-size="${options.size || 17}" font-weight="${options.weight || 400}" opacity="${options.opacity == null ? 1 : options.opacity}">${escapeXml(value)}</text>`;
}

function rect(x, y, w, h, options = {}) {
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${options.rx == null ? 0 : options.rx}" fill="${options.fill || "none"}" stroke="${options.stroke || "none"}" stroke-width="${options.sw == null ? 1 : options.sw}"/>`;
}

function line(x1, y1, x2, y2, color = palette.border, strokeWidth = 1) {
  return `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${color}" stroke-width="${strokeWidth}"/>`;
}

function rowColors(row) {
  const match = row[2];
  const state = row[3];
  if (match === "missing") {
    return {
      row: "#171019",
      text: palette.missing,
      match: palette.missing,
      state: palette.missing,
    };
  }
  if (state === "idle") {
    return {
      row: "transparent",
      text: palette.dim,
      match: palette.dim,
      state: palette.idle,
    };
  }
  return {
    row: "transparent",
    text: palette.text,
    match: match === "matched" ? palette.match : palette.text,
    state: palette.active,
  };
}

function drawHeader(summary, filter) {
  const parts = [
    ["hosts ", palette.muted, 400],
    [summary.hosts, palette.active, 700],
    [" | panes ", palette.muted, 400],
    [summary.panes, palette.text, 700],
    [" | matched ", palette.muted, 400],
    [summary.matched, palette.match, 700],
    [" | active ", palette.muted, 400],
    [summary.active, palette.active, 700],
    [" | quiet ", palette.muted, 400],
    [summary.quiet, palette.quiet, 700],
    [" | idle ", palette.muted, 400],
    [summary.idle, palette.idle, 700],
    [" | filter: ", palette.muted, 400],
    [filter || "-", palette.text, 700],
    [" | missing ", palette.muted, 400],
    [summary.missing, palette.missing, 700],
  ];

  let x = 34;
  let output = "";
  for (const [value, color, weight] of parts) {
    output += text(x, 74, value, { color, weight, size: 18 });
    x += String(value).length * 10.5;
  }
  return output;
}

function drawTable(tableRows, selectedId) {
  const x = 26;
  const y = 112;
  const w = 1228;
  const h = 334;
  const cols = [0, 94, 310, 430, 538, 650, 804, 874];
  const headers = ["HOST", "ID", "MATCH", "STATE", "CMD", "REPO", "DIRTY", "PREVIEW"];

  let output = rect(x, y, w, h, {
    fill: palette.panel,
    stroke: palette.border,
    rx: 6,
  });
  output += text(x + 16, y + 25, "sessions", {
    color: palette.muted,
    size: 15,
    weight: 700,
  });
  output += line(x + 14, y + 52, x + w - 14, y + 52, "#2b343d");

  headers.forEach((header, index) => {
    output += text(x + 18 + cols[index], y + 44, header, {
      color: palette.text,
      size: 15,
      weight: 700,
    });
  });

  tableRows.forEach((row, index) => {
    const rowY = y + 74 + index * 27;
    const selected = row[1] === selectedId;
    const colors = rowColors(row);
    if (selected) {
      output += rect(x + 10, rowY - 20, w - 20, 25, {
        fill: palette.selectedBg,
        rx: 3,
      });
    } else if (colors.row !== "transparent") {
      output += rect(x + 10, rowY - 20, w - 20, 25, {
        fill: colors.row,
        rx: 3,
      });
    }

    const base = selected ? palette.selectedText : colors.text;
    const matchColor = selected ? palette.selectedText : colors.match;
    const stateColor = selected ? palette.selectedText : colors.state;
    const dirtyColor = selected
      ? palette.selectedText
      : row[6] !== "-" && row[6] !== "0"
        ? palette.dirty
        : palette.dim;

    row.forEach((cell, col) => {
      const color =
        col === 2
          ? matchColor
          : col === 3
            ? stateColor
            : col === 6
              ? dirtyColor
              : base;
      const weight =
        selected ||
        col === 2 ||
        col === 3 ||
        (col === 6 && row[6] !== "-" && row[6] !== "0")
          ? 700
          : 400;
      output += text(x + 18 + cols[col], rowY, cell, {
        color,
        size: 15,
        weight,
      });
    });
  });

  return output;
}

function detailData(kind) {
  if (kind === "pi") {
    return {
      id: "pi-agent",
      target: "pi/work:0.1",
      tmux: "work:0.1",
      pane: "%11",
      pid: "5101",
      command: "node",
      cwd: "/home/cam/openclaw",
      repo: "/home/cam/openclaw",
      branch: "main",
      dirty: "2",
      output: [
        "npm test",
        "agent: editing transport adapter",
        "all checks queued",
        "",
        "Changed files (2)",
        " M src/agent.ts",
        "?? notes/plan.md",
      ],
    };
  }

  return {
    id: "local-codex",
    target: "local/agent:0.0",
    tmux: "agent:0.0",
    pane: "%1",
    pid: "4101",
    command: "codex",
    cwd: "/home/nixos/remux",
    repo: "/home/nixos/remux",
    branch: "feature/public-readiness-tui-polish",
    dirty: "1",
    output: [
      "codex check --locked",
      "reading src/tui.rs",
      "updated pane preview and footer",
      "",
      "Changed files (1)",
      " M src/tui.rs",
    ],
  };
}

function drawDetailPane(kind) {
  const x = 26;
  const y = 464;
  const w = 1228;
  const h = 232;
  const split = 520;
  const detail = detailData(kind);
  const leftX = x + 18;
  const rightX = x + split + 28;
  let yy = y + 58;

  let output = rect(x, y, w, h, {
    fill: palette.panel2,
    stroke: palette.border,
    rx: 6,
  });
  output += text(x + 16, y + 25, "pane preview", {
    color: palette.muted,
    size: 15,
    weight: 700,
  });
  output += line(x + split, y + 36, x + split, y + h - 16, "#2b343d");

  output += text(leftX, yy, detail.id, {
    color: palette.active,
    size: 17,
    weight: 700,
  });
  yy += 23;
  output += text(leftX, yy, "activity: ", {
    color: palette.muted,
    size: 15,
    weight: 700,
  });
  output += text(leftX + 86, yy, "active", {
    color: palette.active,
    size: 15,
    weight: 700,
  });
  output += text(leftX + 150, yy, "| match: ", {
    color: palette.muted,
    size: 15,
    weight: 700,
  });
  output += text(leftX + 230, yy, "matched", {
    color: palette.match,
    size: 15,
    weight: 700,
  });
  output += text(leftX + 306, yy, `| watch: ${detail.id}`, {
    color: palette.text,
    size: 15,
  });
  yy += 22;
  output += text(leftX, yy, `target: ${detail.target}`, {
    color: palette.text,
    size: 15,
  });
  yy += 22;
  output += text(leftX, yy, `tmux: ${detail.tmux} | pane id: ${detail.pane} | pid: ${detail.pid}`, {
    color: palette.text,
    size: 15,
  });
  yy += 29;
  output += text(leftX, yy, `command: ${detail.command}`, {
    color: palette.text,
    size: 15,
  });
  yy += 20;
  output += text(leftX, yy, `cwd: ${detail.cwd}`, {
    color: palette.text,
    size: 15,
  });
  yy += 20;
  output += text(leftX, yy, `repo: ${detail.repo}`, {
    color: palette.text,
    size: 15,
  });
  yy += 20;
  output += text(leftX, yy, "dirty: ", {
    color: palette.text,
    size: 15,
  });
  output += text(leftX + 60, yy, detail.dirty, {
    color: palette.dirty,
    size: 15,
    weight: 700,
  });
  output += text(leftX + 78, yy, ` | branch: ${detail.branch}`, {
    color: palette.text,
    size: 15,
  });

  yy = y + 58;
  output += text(rightX, yy, "Recent output preview", {
    color: palette.text,
    size: 17,
    weight: 700,
  });
  yy += 27;
  for (const value of detail.output) {
    if (value === "") {
      yy += 13;
      continue;
    }
    output += text(rightX, yy, value, {
      color: palette.text,
      size: 15,
      weight: value.startsWith("Changed files") ? 700 : 400,
    });
    yy += 20;
  }

  output += text(rightX, y + h - 22, `enter attach readonly | c capture | i inspect ${detail.id}`, {
    color: palette.muted,
    size: 15,
    weight: 700,
  });

  return output;
}

function drawFooter(status, filter = "-") {
  let output = rect(0, 710, width, 50, { fill: palette.footerBg });
  const keys = [
    ["enter", " attach | "],
    ["r", " refresh | "],
    ["/", " filter | "],
    ["c", " capture | "],
    ["i", " inspect | "],
    ["q", " quit"],
  ];

  let x = 28;
  for (const [key, label] of keys) {
    output += text(x, 738, key, {
      color: palette.active,
      size: 16,
      weight: 700,
    });
    x += key.length * 9.8;
    output += text(x, 738, label, { color: palette.text, size: 16 });
    x += label.length * 9.2;
  }
  output += text(680, 738, status, { color: palette.text, size: 16 });
  output += text(1100, 738, `filter: ${filter}`, {
    color: palette.text,
    size: 16,
  });
  return output;
}

function renderSvg({ tableRows, selectedId, summary, filter = "-", detail = "local", status }) {
  const visibleFilter = filter === "-" ? "" : filter;
  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <style>text { font-family: "SFMono-Regular", "Cascadia Mono", "Menlo", "Consolas", "Liberation Mono", monospace; }</style>
  ${rect(0, 0, width, height, { fill: palette.bg })}
  ${rect(18, 18, width - 36, 72, { fill: palette.panel, stroke: palette.border, rx: 6 })}
  ${text(34, 45, "remux alpha", { color: palette.text, size: 17, weight: 700 })}
  ${drawHeader(summary, visibleFilter)}
  ${drawTable(tableRows, selectedId)}
  ${drawDetailPane(detail)}
  ${drawFooter(status, filter)}
</svg>`;
}

async function writeSources() {
  await mkdir(workDir, { recursive: true });

  const screenshot = renderSvg({
    tableRows: rows,
    selectedId: "local-codex",
    summary: summaryAll,
    detail: "local",
    status: "ready | scan complete: 3/3 hosts | elapsed 287ms",
  });
  await writeFile(join(workDir, "remux-tui.svg"), screenshot);

  const piRows = rows.filter((row) => row[0] === "pi");
  const frames = [
    screenshot,
    renderSvg({
      tableRows: piRows,
      selectedId: "pi-agent",
      summary: summaryPi,
      detail: "pi",
      filter: "pi",
      status: "filter | pi",
    }),
    renderSvg({
      tableRows: piRows,
      selectedId: "pi-agent",
      summary: summaryPi,
      detail: "pi",
      filter: "pi",
      status: "ready | inspected pi-agent",
    }),
    renderSvg({
      tableRows: piRows,
      selectedId: "pi-agent",
      summary: summaryPi,
      detail: "pi",
      filter: "pi",
      status: "ready | attached to pi/work:0.1",
    }),
  ];

  for (const [index, frame] of frames.entries()) {
    await writeFile(join(workDir, `frame-${String(index).padStart(2, "0")}.svg`), frame);
  }
}

function runMagick(args) {
  const result = spawnSync("magick", args, {
    stdio: "inherit",
  });
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
    "110",
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
