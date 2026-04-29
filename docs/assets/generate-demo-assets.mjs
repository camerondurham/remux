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
  borderSoft: "#29323c",
  text: "#d7dde5",
  muted: "#7d8590",
  dim: "#6e7681",
  active: "#79d7ff",
  quiet: "#e3b341",
  idle: "#8b949e",
  missing: "#ff6b6b",
  ambiguous: "#d58cff",
  shadowed: "#8ab4ff",
  selectedBg: "#9ee7ff",
  selectedText: "#081019",
  dirty: "#f2cc60",
  match: "#76e4f7",
  footerBg: "#0a0f14",
};

const hosts = [
  { id: "local", status: "ok" },
  { id: "pi", status: "ok" },
  { id: "lab", status: "ok" },
  { id: "buildbox", status: "unreachable" },
];

const rows = [
  {
    host: "buildbox",
    id: "build-runner",
    target: "buildbox/build:0.0",
    match: "unreachable",
    state: "unreachable",
    last: "-",
    command: "bash",
    repo: "remux",
    dirty: "-",
    preview: "ssh: connect: no route",
    session: "build",
    pane: "0.0",
    paneId: "-",
    pid: "-",
    cwd: "/home/nixos/remux",
    branch: "-",
    errors: ["poll: ssh connect failed"],
  },
  {
    host: "pi",
    id: "missing-debug",
    target: "pi/debug",
    match: "missing",
    state: "missing",
    last: "-",
    command: "-",
    repo: "-",
    dirty: "-",
    preview: "watch did not match",
    session: "debug",
    pane: "-",
    paneId: "-",
    pid: "-",
    cwd: "-",
    branch: "-",
    errors: ["missing: watch did not match a live pane"],
  },
  {
    host: "lab",
    id: "lab-agent",
    target: "lab/ai",
    match: "ambiguous",
    state: "unknown",
    last: "-",
    command: "python",
    repo: "bots",
    dirty: "1",
    preview: "matched 2 live panes",
    session: "ai",
    pane: "-",
    paneId: "-",
    pid: "-",
    cwd: "/srv/bots",
    branch: "main",
    candidates: ["lab/ai:2.0", "lab/ai:2.1"],
  },
  {
    host: "pi",
    id: "pi-service",
    target: "pi/service:0.0",
    match: "shadowed",
    state: "active",
    last: "48s",
    command: "bash",
    repo: "service",
    dirty: "0",
    preview: "heartbeat ok",
    session: "service",
    pane: "0.0",
    paneId: "%21",
    pid: "6220",
    cwd: "/home/cam/service",
    branch: "main",
    shadowedBy: "pi-agent",
    output: ["while true; do curl -sf localhost:8080/health; sleep 5; done", "heartbeat ok"],
  },
  {
    host: "local",
    id: "local/ops:1.0",
    target: "local/ops:1.0",
    match: "orphan",
    state: "idle",
    last: "2h",
    command: "bash",
    repo: "-",
    dirty: "-",
    preview: "deploy shell idle",
    session: "ops",
    pane: "1.0",
    paneId: "%4",
    pid: "3911",
    cwd: "/home/nixos",
    branch: "-",
    output: ["last deploy succeeded", "waiting for next maintenance window"],
  },
  {
    host: "lab",
    id: "lab/ai:2.0",
    target: "lab/ai:2.0",
    match: "orphan",
    state: "active",
    last: "5s",
    command: "python",
    repo: "bots",
    dirty: "1",
    preview: "running eval job",
    session: "ai",
    pane: "2.0",
    paneId: "%8",
    pid: "8842",
    cwd: "/srv/bots",
    branch: "main",
    output: ["python run_eval.py --suite smoke", "processed 12 jobs", "running eval job"],
  },
  {
    host: "lab",
    id: "lab/ai:2.1",
    target: "lab/ai:2.1",
    match: "orphan",
    state: "active",
    last: "6s",
    command: "python",
    repo: "bots",
    dirty: "1",
    preview: "collecting traces",
    session: "ai",
    pane: "2.1",
    paneId: "%9",
    pid: "8843",
    cwd: "/srv/bots",
    branch: "main",
    output: ["tail -f logs/trace.log", "collecting traces"],
  },
  {
    host: "pi",
    id: "pi-agent",
    target: "pi/work:0.1",
    match: "matched",
    state: "active",
    last: "38s",
    command: "node",
    repo: "openclaw",
    dirty: "2",
    preview: "all checks queued",
    session: "work",
    pane: "0.1",
    paneId: "%11",
    pid: "5101",
    cwd: "/home/cam/openclaw",
    branch: "main",
    changed: [" M src/agent.ts", "?? notes/plan.md"],
    output: [
      "npm test",
      "agent: editing transport adapter",
      "all checks queued",
      "Changed files (2)",
      " M src/agent.ts",
      "?? notes/plan.md",
    ],
  },
  {
    host: "local",
    id: "local-codex",
    target: "local/agent:0.0",
    match: "matched",
    state: "active",
    last: "12s",
    command: "codex",
    repo: "remux",
    dirty: "1",
    preview: "updated four-panel dashboard",
    session: "agent",
    pane: "0.0",
    paneId: "%1",
    pid: "4101",
    cwd: "/home/nixos/remux",
    branch: "codex/update-readme-tui-assets",
    changed: [" M src/tui.rs", " M docs/assets/generate-demo-assets.mjs"],
    output: [
      "cargo test --locked --all-targets",
      "reading src/tui.rs",
      "updated four-panel dashboard",
      "regenerating README assets",
    ],
  },
  {
    host: "local",
    id: "local-build",
    target: "local/build:1.0",
    match: "matched",
    state: "quiet",
    last: "4m",
    command: "cargo",
    repo: "remux",
    dirty: "0",
    preview: "Finished dev profile",
    session: "build",
    pane: "1.0",
    paneId: "%2",
    pid: "4188",
    cwd: "/home/nixos/remux",
    branch: "main",
    output: ["cargo test --locked", "Finished dev profile"],
  },
];

const layout = {
  kpi: { x: 18, y: 18, w: 1244, h: 96 },
  topology: { x: 18, y: 128, w: 245, h: 570 },
  table: { x: 273, y: 128, w: 600, h: 570 },
  inspector: { x: 883, y: 128, w: 379, h: 570 },
  footer: { x: 0, y: 710, w: 1280, h: 50 },
};

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function text(x, y, value, options = {}) {
  return `<text x="${x}" y="${y}" fill="${options.color || palette.text}" font-size="${options.size || 15}" font-weight="${options.weight || 400}" opacity="${options.opacity == null ? 1 : options.opacity}">${escapeXml(value)}</text>`;
}

function rect(x, y, w, h, options = {}) {
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${options.rx == null ? 0 : options.rx}" fill="${options.fill || "none"}" stroke="${options.stroke || "none"}" stroke-width="${options.sw == null ? 1 : options.sw}"/>`;
}

function line(x1, y1, x2, y2, color = palette.borderSoft, strokeWidth = 1) {
  return `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="${color}" stroke-width="${strokeWidth}"/>`;
}

function charLimit(widthPx, size = 13) {
  return Math.max(1, Math.floor(widthPx / (size * 0.62)));
}

function truncate(value, maxChars) {
  const string = String(value);
  if (string.length <= maxChars) {
    return string;
  }
  if (maxChars <= 1) {
    return string.slice(0, maxChars);
  }
  return `${string.slice(0, maxChars - 1)}~`;
}

function rowText(row) {
  return [
    row.host,
    row.id,
    row.target,
    row.match,
    row.state,
    row.command,
    row.repo,
    row.preview,
    row.cwd,
    row.branch,
  ]
    .join(" ")
    .toLowerCase();
}

function visibleRows(filter) {
  const needle = filter.trim().toLowerCase();
  if (!needle) {
    return rows;
  }
  return rows.filter((row) => rowText(row).includes(needle));
}

function styleForMatch(match) {
  if (match === "matched") return palette.match;
  if (match === "missing" || match === "unreachable") return palette.missing;
  if (match === "ambiguous") return palette.ambiguous;
  if (match === "shadowed") return palette.shadowed;
  return palette.text;
}

function styleForState(state) {
  if (state === "active") return palette.active;
  if (state === "quiet") return palette.quiet;
  if (state === "idle") return palette.idle;
  if (state === "missing" || state === "unreachable") return palette.missing;
  return palette.dim;
}

function rowFill(row) {
  if (row.match === "unreachable") return "#1c1014";
  if (row.match === "missing") return "#1b1018";
  if (row.match === "ambiguous") return "#171328";
  if (row.match === "shadowed") return "#111a2a";
  return "transparent";
}

function dirtyColor(value) {
  return value !== "-" && value !== "0" ? palette.dirty : palette.dim;
}

function summary() {
  const hostOk = hosts.filter((host) => host.status === "ok").length;
  const hostTotal = hosts.length;
  const countState = (state) => rows.filter((row) => row.state === state).length;
  const countMatch = (match) => rows.filter((row) => row.match === match).length;
  const errors = rows.filter((row) => row.errors?.length).length;
  const attention =
    hosts.filter((host) => host.status === "unreachable").length +
    countState("missing") +
    countMatch("ambiguous") +
    errors;
  return {
    hostOk,
    hostTotal,
    panes: rows.length,
    active: countState("active"),
    quiet: countState("quiet"),
    idle: countState("idle"),
    missing: countState("missing"),
    ambiguous: countMatch("ambiguous"),
    shadowed: countMatch("shadowed"),
    errors,
    attention,
  };
}

function drawPanel(area, title, fill = palette.panel) {
  return (
    rect(area.x, area.y, area.w, area.h, {
      fill,
      stroke: palette.border,
      rx: 6,
    }) +
    text(area.x + 16, area.y + 25, title, {
      color: palette.muted,
      size: 15,
      weight: 700,
    })
  );
}

function richLine(x, y, parts, size = 17) {
  let cursor = x;
  let output = "";
  for (const part of parts) {
    output += text(cursor, y, part.value, {
      color: part.color,
      size,
      weight: part.weight,
    });
    cursor += String(part.value).length * size * 0.62;
  }
  return output;
}

function drawKpi(filter, sortMode) {
  const box = layout.kpi;
  const data = summary();
  let output = drawPanel(box, "kpi", palette.panel);
  output += richLine(
    box.x + 16,
    box.y + 54,
    [
      { value: "hosts ", color: palette.muted },
      { value: `${data.hostOk}/${data.hostTotal} up`, color: data.hostOk === data.hostTotal ? palette.active : palette.missing, weight: 700 },
      { value: " | panes ", color: palette.muted },
      { value: data.panes, color: palette.text, weight: 700 },
      { value: " | active ", color: palette.muted },
      { value: data.active, color: palette.active, weight: 700 },
      { value: " | quiet ", color: palette.muted },
      { value: data.quiet, color: palette.quiet, weight: 700 },
      { value: " | idle ", color: palette.muted },
      { value: data.idle, color: palette.idle, weight: 700 },
      { value: " | missing ", color: palette.muted },
      { value: data.missing, color: palette.missing, weight: 700 },
      { value: " | ambiguous ", color: palette.muted },
      { value: data.ambiguous, color: palette.ambiguous, weight: 700 },
      { value: " | shadowed ", color: palette.muted },
      { value: data.shadowed, color: palette.shadowed, weight: 700 },
    ],
    17,
  );
  output += richLine(
    box.x + 16,
    box.y + 80,
    [
      { value: "attention ", color: palette.muted },
      { value: data.attention, color: palette.dirty, weight: 700 },
      { value: " | errors ", color: palette.muted },
      { value: data.errors, color: palette.missing, weight: 700 },
      { value: " | sort ", color: palette.muted },
      { value: sortMode, color: palette.text, weight: 700 },
      { value: " | filter ", color: palette.muted },
      { value: filter || "-", color: palette.text, weight: 700 },
    ],
    17,
  );
  return output;
}

function drawTopology(tableRows, selectedId) {
  const box = layout.topology;
  let output = drawPanel(box, "topology", palette.panel2);
  const byHost = new Map();
  for (const row of tableRows) {
    if (!byHost.has(row.host)) byHost.set(row.host, []);
    byHost.get(row.host).push(row);
  }

  let y = box.y + 54;
  for (const host of hosts) {
    const hostRows = byHost.get(host.id) || [];
    if (hostRows.length === 0) continue;
    output += text(box.x + 14, y, "> ", { color: palette.muted, size: 13 });
    output += text(box.x + 34, y, host.id, { color: palette.text, size: 13, weight: 700 });
    output += text(box.x + 34 + host.id.length * 8.1, y, ` (${host.status})`, {
      color: host.status === "ok" ? palette.active : palette.missing,
      size: 13,
    });
    y += 22;

    const sessions = new Map();
    for (const row of hostRows) {
      if (!sessions.has(row.session)) sessions.set(row.session, []);
      sessions.get(row.session).push(row);
    }
    for (const [session, sessionRows] of sessions) {
      output += text(box.x + 32, y, `> session ${truncate(session, 18)}`, {
        color: palette.text,
        size: 13,
      });
      y += 20;
      for (const row of sessionRows) {
        const marker = row.id === selectedId ? ">" : " ";
        output += text(box.x + 47, y, marker, {
          color: row.id === selectedId ? palette.active : palette.muted,
          size: 13,
          weight: 700,
        });
        output += text(box.x + 62, y, truncate(row.pane, 5), {
          color: palette.muted,
          size: 13,
        });
        output += text(box.x + 104, y, truncate(row.command, 8), {
          color: palette.text,
          size: 13,
        });
        output += text(box.x + 172, y, truncate(row.state, 10), {
          color: styleForState(row.state),
          size: 13,
          weight: 700,
        });
        y += 19;
        if (y > box.y + box.h - 16) return output;
      }
    }
  }
  return output;
}

function drawTable(tableRows, selectedId) {
  const box = layout.table;
  const cols = [
    { key: "id", label: "ID", x: 14, w: 90 },
    { key: "target", label: "TARGET", x: 104, w: 116 },
    { key: "match", label: "MATCH", x: 220, w: 70 },
    { key: "state", label: "STATE", x: 290, w: 52 },
    { key: "last", label: "LAST", x: 342, w: 38 },
    { key: "command", label: "CMD", x: 381, w: 49 },
    { key: "repo", label: "REPO", x: 430, w: 50 },
    { key: "dirty", label: "DIRTY", x: 480, w: 44 },
    { key: "preview", label: "PREVIEW", x: 526, w: 64 },
  ];
  let output = drawPanel(box, "live table", palette.panel);
  output += line(box.x + 10, box.y + 44, box.x + box.w - 10, box.y + 44);
  for (const col of cols) {
    output += text(box.x + col.x, box.y + 41, col.label, {
      color: palette.text,
      size: 12,
      weight: 700,
    });
  }

  tableRows.slice(0, 18).forEach((row, index) => {
    const rowY = box.y + 66 + index * 25;
    const selected = row.id === selectedId;
    const fill = selected ? palette.selectedBg : rowFill(row);
    if (fill !== "transparent") {
      output += rect(box.x + 8, rowY - 17, box.w - 16, 22, {
        fill,
        rx: 3,
      });
    }
    const base = selected ? palette.selectedText : palette.text;
    for (const col of cols) {
      const value = row[col.key];
      const color = selected
        ? palette.selectedText
        : col.key === "match"
          ? styleForMatch(row.match)
          : col.key === "state"
            ? styleForState(row.state)
            : col.key === "dirty"
              ? dirtyColor(row.dirty)
              : col.key === "preview"
                ? palette.muted
                : base;
      const weight =
        selected ||
        col.key === "match" ||
        col.key === "state" ||
        (col.key === "dirty" && row.dirty !== "-" && row.dirty !== "0")
          ? 700
          : 400;
      output += text(
        box.x + col.x,
        rowY,
        truncate(value, charLimit(col.w, 12)),
        { color, size: 12, weight },
      );
    }
  });
  return output;
}

function selectedRow(selectedId) {
  return rows.find((row) => row.id === selectedId) || rows[0];
}

function drawInspector(selectedId, options = {}) {
  const box = layout.inspector;
  const row = selectedRow(selectedId);
  let output = drawPanel(box, options.help ? "keys" : "inspector", palette.panel2);
  if (options.help) {
    const help = [
      "j/down select next",
      "up select previous",
      "r refresh now",
      "s cycle table sort",
      "/ filter",
      "enter readonly attach",
      "a read-write jump",
      "c capture into detail",
      "i inspect and refresh detail",
      "k kill selected pane",
      "e rename selected session",
      "n create session",
      "p spawn pane",
      "q quit",
    ];
    help.forEach((lineText, index) => {
      output += text(box.x + 16, box.y + 56 + index * 22, lineText, {
        color: palette.text,
        size: 13,
      });
    });
    return output;
  }

  const split = box.x + 166;
  output += line(split, box.y + 45, split, box.y + box.h - 18);
  let y = box.y + 56;
  const leftX = box.x + 16;
  const rightX = split + 14;
  output += text(leftX, y, truncate(row.id, 16), {
    color: palette.active,
    size: 13,
    weight: 700,
  });
  y += 22;
  const metaLines = [
    `activity ${row.state}`,
    `match ${row.match}`,
    `watch ${row.match === "orphan" ? "-" : row.id}`,
    `target ${row.target}`,
    `tmux ${row.session}:${row.pane}`,
    `pane id ${row.paneId}`,
    `pid ${row.pid}`,
    "",
    `command ${row.command}`,
    `cwd ${row.cwd}`,
    `repo ${row.repo}`,
    `branch ${row.branch}`,
    `dirty ${row.dirty}`,
    "",
    row.match === "matched" || row.match === "orphan"
      ? `attach enter | a -> ${row.target}`
      : `attach unavailable, ${row.match}`,
    row.match === "matched" || row.match === "orphan"
      ? `capture c -> ${row.id}`
      : `capture unavailable, ${row.match}`,
  ];
  for (const lineText of metaLines) {
    if (lineText === "") {
      y += 12;
      continue;
    }
    output += text(leftX, y, truncate(lineText, 18), {
      color:
        lineText.includes(row.state)
          ? styleForState(row.state)
          : lineText.includes(row.match)
            ? styleForMatch(row.match)
            : palette.text,
      size: 12,
      weight: lineText.startsWith("activity") || lineText.startsWith("match") ? 700 : 400,
    });
    y += 18;
    if (y > box.y + box.h - 20) break;
  }

  y = box.y + 56;
  const statusLines = [];
  if (row.changed?.length) {
    statusLines.push(`Changed files (${row.changed.length})`, ...row.changed);
  }
  if (row.errors?.length) {
    if (statusLines.length) statusLines.push("");
    statusLines.push("Errors", ...row.errors);
  }
  if (row.candidates?.length) {
    if (statusLines.length) statusLines.push("");
    statusLines.push("Candidates", ...row.candidates);
  }
  if (row.shadowedBy) {
    if (statusLines.length) statusLines.push("");
    statusLines.push(`Shadowed by: ${row.shadowedBy}`);
  }
  for (const status of statusLines.slice(0, 9)) {
    if (status === "") {
      y += 10;
      continue;
    }
    output += text(rightX, y, truncate(status, 25), {
      color:
        status === "Errors"
          ? palette.missing
          : status === "Candidates" || status.startsWith("Shadowed")
            ? palette.dirty
            : palette.text,
      size: 12,
      weight: status.includes("(") || status === "Errors" || status === "Candidates" ? 700 : 400,
    });
    y += 18;
  }

  y += statusLines.length ? 14 : 0;
  output += text(rightX, y, "Recent output preview", {
    color: palette.text,
    size: 13,
    weight: 700,
  });
  y += 22;
  const outputLines = row.output?.length
    ? row.output
    : ["No recent output in cache.", "Capture or inspect this row."];
  for (const value of outputLines.slice(-8)) {
    output += text(rightX, y, truncate(value, 25), {
      color: palette.text,
      size: 12,
      weight: value.startsWith("Changed files") ? 700 : 400,
    });
    y += 18;
    if (y > box.y + box.h - 18) break;
  }
  return output;
}

function drawFooter(status, filter = "-", prompt = null) {
  const box = layout.footer;
  let output = rect(box.x, box.y, box.w, box.h, { fill: palette.footerBg });
  if (prompt) {
    output += text(28, 739, prompt.label, {
      color: palette.dirty,
      size: 15,
      weight: 700,
    });
    output += text(142, 739, prompt.value, {
      color: palette.text,
      size: 15,
    });
    output += text(142 + prompt.value.length * 9.3, 739, " _", {
      color: palette.active,
      size: 15,
      weight: 700,
    });
    output += text(382, 739, `| ${prompt.hint}`, {
      color: palette.muted,
      size: 15,
    });
    return output;
  }

  const keys = [
    ["enter", " readonly | "],
    ["a", " jump | "],
    ["r", " refresh | "],
    ["s", " sort | "],
    ["/", " filter | "],
    ["c", " capture | "],
    ["i", " inspect | "],
    ["k", " kill | "],
    ["e", " rename | "],
    ["n", " new | "],
    ["p", " pane | "],
    ["q", " quit"],
  ];

  let x = 24;
  for (const [key, label] of keys) {
    output += text(x, 731, key, {
      color: palette.active,
      size: 12,
      weight: 700,
    });
    const visibleLabel = label.trimStart();
    x += key.length * 7.4 + 5;
    output += text(x, 731, visibleLabel, { color: palette.text, size: 12 });
    x += visibleLabel.length * 6.8 + 5;
  }
  output += text(24, 753, status, { color: palette.text, size: 12 });
  output += text(1115, 753, `filter: ${filter || "-"}`, {
    color: palette.text,
    size: 12,
  });
  return output;
}

function renderSvg({
  selectedId,
  filter = "",
  sortMode = "attention",
  status = "ready | scan complete: 3/4 hosts | elapsed 287ms",
  help = false,
  prompt = null,
}) {
  const tableRows = visibleRows(filter);
  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <style>text { font-family: "SFMono-Regular", "Cascadia Mono", "Menlo", "Consolas", "Liberation Mono", monospace; }</style>
  ${rect(0, 0, width, height, { fill: palette.bg })}
  ${drawKpi(filter, sortMode)}
  ${drawTopology(tableRows, selectedId)}
  ${drawTable(tableRows, selectedId)}
  ${drawInspector(selectedId, { help })}
  ${drawFooter(status, filter, prompt)}
</svg>`;
}

async function writeSources() {
  await mkdir(workDir, { recursive: true });

  const screenshot = renderSvg({
    selectedId: "pi-agent",
  });
  await writeFile(join(workDir, "remux-tui.svg"), screenshot);

  const frames = [
    screenshot,
    renderSvg({
      selectedId: "pi-agent",
      filter: "pi",
      status: "filter | 3 rows match pi",
    }),
    renderSvg({
      selectedId: "pi-service",
      filter: "pi",
      sortMode: "state",
      status: "ready | sort: state",
    }),
    renderSvg({
      selectedId: "pi-agent",
      filter: "pi",
      status: "ready | new session",
      prompt: {
        label: "new session",
        value: "pi/scratch",
        hint: "enter <host>/<session> (Esc to cancel)",
      },
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
