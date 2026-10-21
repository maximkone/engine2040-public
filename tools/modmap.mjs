// modmap: one line per tracked Rust file under crates/, grouped by crate, written to
// .opencode/modmap.md (AGENTS.md "Files inside a crate": Grep it, never read it whole).
//   node tools/modmap.mjs                print the map to stdout
//   node tools/modmap.mjs --write        rewrite .opencode/modmap.md
//   node tools/modmap.mjs --check        print PASS .opencode/modmap.md, exit 1 when stale
//   node tools/modmap.mjs --pack <crate> print one crate's packet: the map slice, the
//                                      docs/CRATE-MAP.md row, the plan-index line and
//                                      the open board rows, under 6,000 tokens
//   node tools/modmap.mjs --packet <crate> the same packet (--packet is an alias)
// Exit codes: 0 ok, 1 stale map (--check) or packet over budget (--pack), 2 usage.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadBoardText, ownedOf, parseBoard, rowReady } from "./board-slice.mjs";
import { parsePrefixTable, prefixOf } from "./plan-atlas.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MAP = join(ROOT, ".opencode", "modmap.md");

// The house token rule (docs/compaction-baseline.md 1.9): tokens = bytes / 4.
// The packet budget is the number that rule buys: 6,000 tokens.
export const PACK_BUDGET_TOKENS = 6000;
// Bytes kept free for the measure block and the row-id tail of the packet.
const PACK_RESERVE_BYTES = 1024;

// Package name from a crate's Cargo.toml text ("" when absent).
export function packageName(cargoToml) {
  const m = /^\s*name\s*=\s*"([^"]+)"/m.exec(cargoToml);
  return m ? m[1] : "";
}

// The file's own first `//!` line, trimmed; "(no //! line)" when it has none.
export function leadingDoc(text) {
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim();
    if (t.startsWith("//!")) {
      const doc = t.slice(3).trim();
      if (doc) return doc;
      continue;
    }
    if (t === "" || t.startsWith("//") || t.startsWith("#![")) continue;
    break;
  }
  return "(no //! line)";
}

// lib.rs and main.rs first, then plain path order.
export function fileOrder(a, b) {
  const rank = (p) => (/^src\/(lib|main)\.rs$/.test(p) ? 0 : 1);
  return rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0);
}

// crates: [{ dir, pkg, files: [{ path (crate-relative), lines, doc }] }] -> markdown.
export function render(crates) {
  const fileCount = crates.reduce((n, c) => n + c.files.length, 0);
  const out = [
    "# modmap: engine2040",
    `_${crates.length} crates, ${fileCount} Rust files. One line per file: \`crate/path\` · lines · the file's own leading \`//!\` line. Read it with Grep (a crate name or a word), never whole. Generated from tracked files by \`node tools/modmap.mjs --write\`; do not edit by hand._`,
    "",
  ];
  for (const c of crates) {
    const lines = c.files.reduce((n, f) => n + f.lines, 0);
    out.push(`## ${c.dir} (${c.pkg}, ${c.files.length} files, ${lines} lines)`);
    for (const f of [...c.files].sort((a, b) => fileOrder(a.path, b.path))) {
      out.push(`- \`${c.dir}/${f.path}\` · ${f.lines} · ${f.doc}`);
    }
    out.push("");
  }
  return out.join("\n");
}

export function collect(root = ROOT) {
  const tracked = execFileSync("git", ["ls-files", "-z", "--", "crates/*.rs"], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
    .split("\0")
    .filter(Boolean);
  const byCrate = new Map();
  for (const rel of tracked) {
    const parts = rel.split("/");
    if (parts.length < 3) continue;
    const dir = parts[1];
    const full = join(root, rel);
    if (!existsSync(full)) continue; // deleted in the working tree, not yet committed
    if (!byCrate.has(dir)) byCrate.set(dir, []);
    const text = readFileSync(full, "utf8");
    byCrate.get(dir).push({ path: parts.slice(2).join("/"), lines: text.split(/\r?\n/).length - (text.endsWith("\n") ? 1 : 0), doc: leadingDoc(text) });
  }
  return [...byCrate.keys()].sort().flatMap((dir) => {
    const toml = join(root, "crates", dir, "Cargo.toml");
    if (!existsSync(toml)) return [];
    return [{ dir, pkg: packageName(readFileSync(toml, "utf8")), files: byCrate.get(dir) }];
  });
}

// The house token rule: bytes / 4, rounded (docs/compaction-baseline.md 1.9
// prints the same numbers: 144,615 bytes -> 36,154 tokens).
export function tokenCount(bytes) {
  return Math.round(bytes / 4);
}

// Bytes of a text as the packet counts them.
export function byteCount(text) {
  return Buffer.byteLength(text, "utf8");
}

// "render" or "crates/render" -> "render"; anything else -> null.
export function crateDirOf(arg) {
  const s = String(arg ?? "").trim().replace(/[\\/]+$/, "");
  if (!s) return null;
  const dir = s.startsWith("crates/") ? s.slice("crates/".length) : s;
  if (!/^[a-z0-9_-]+$/.test(dir)) return null;
  return dir;
}

// The crate's section of the map text: the heading plus its file lines.
export function mapSectionOf(mapText, dir) {
  const lines = String(mapText).split(/\r?\n/);
  const start = lines.findIndex((l) => l.startsWith(`## ${dir} (`));
  if (start < 0) return [];
  const out = [lines[start]];
  for (let i = start + 1; i < lines.length; i += 1) {
    if (lines[i].startsWith("## ")) break;
    out.push(lines[i]);
  }
  while (out.length && out[out.length - 1].trim() === "") out.pop();
  return out;
}

// The docs/CRATE-MAP.md table row of one crate ("" when the map has no row).
export function crateMapRowOf(crateMapText, dir) {
  const want = `\`crates/${dir}\``;
  for (const line of String(crateMapText).split(/\r?\n/)) {
    if (!line.startsWith("|")) continue;
    const cells = line.split("|").map((c) => c.trim());
    if (cells[2] === want) return line;
  }
  return "";
}

// The docs/plan/README.md index line of one plan file ("" when it has none).
export function planIndexLineOf(planReadmeText, file) {
  const want = String(file ?? "").trim();
  if (!want) return "";
  for (const line of String(planReadmeText).split(/\r?\n/)) {
    const m = /^-\s+(.*)$/.exec(line);
    if (!m) continue;
    const head = m[1].split(/\s+—\s+/)[0];
    const names = [...head.matchAll(/`([^`]+)`/g)].map((x) => x[1].trim());
    if (names.includes(want)) return line;
  }
  return "";
}

// Does one owned path point into the crate? ("crates/render" and
// "crates/render/src/x.rs" do; "crates/renderx" does not.)
export function ownsCrate(path, dir) {
  const p = String(path ?? "");
  return p === `crates/${dir}` || p.startsWith(`crates/${dir}/`);
}

// The open board rows that own a path of this crate, ready first, then board
// order. Each row keeps its first physical line and its section plan.
export function boardRowsForCrate(dir, root = ROOT) {
  const { tasks, order } = parseBoard(loadBoardText(root));
  const rows = [];
  for (const id of order) {
    const t = tasks.get(id);
    if (!t || t.done) continue;
    if (!ownedOf(t).some((p) => ownsCrate(p, dir))) continue;
    rows.push({ id, line: t.lines[0], ready: rowReady(tasks, t), sectionPlan: t.sectionPlan ?? "" });
  }
  return rows.sort((a, b) => Number(b.ready) - Number(a.ready));
}

// The plan that serves this crate: the plan file whose row prefix covers most
// of the crate's open rows. Tie: the file name. No prefix match: the plan of
// the rows' board section, else "".
export function planForCrate(rows, prefixTable) {
  const score = new Map();
  for (const r of rows) {
    const p = prefixOf(r.id);
    if (!p) continue;
    for (const file of prefixTable.prefixToFiles.get(p) ?? []) {
      score.set(file, (score.get(file) ?? 0) + 1);
    }
  }
  const ranked = [...score].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  if (ranked.length) return ranked[0][0];
  const section = rows.find((r) => r.sectionPlan)?.sectionPlan ?? "";
  return section ? section.replace(/^docs\/plan\//, "") : "";
}

// How many lines the fresh map and the committed map add and drop.
export function diffSummary(oldText, newText) {
  const count = (text) => {
    const m = new Map();
    for (const line of String(text).split(/\r?\n/)) m.set(line, (m.get(line) ?? 0) + 1);
    return m;
  };
  const a = count(oldText);
  const b = count(newText);
  let added = 0;
  let removed = 0;
  for (const [line, n] of b) {
    const have = a.get(line) ?? 0;
    if (n > have) added += n - have;
  }
  for (const [line, n] of a) {
    const have = b.get(line) ?? 0;
    if (n > have) removed += n - have;
  }
  return { added, removed };
}

// One crate's packet: the map slice, the crate row, the plan line and the open
// board rows, kept under the token budget. The board rows fill what the other
// parts leave; the rest of their ids ride along on one line.
export function buildPacket(dir, { root = ROOT, crates = null, budget = PACK_BUDGET_TOKENS } = {}) {
  const mapText = render(crates ?? collect(root));
  const slice = mapSectionOf(mapText, dir);
  if (!slice.length) return { ok: false, reason: `no map section for crates/${dir}`, text: "", bytes: 0, tokens: 0 };
  const crateRow = crateMapRowOf(readFileSync(join(root, "docs", "CRATE-MAP.md"), "utf8"), dir);
  const planReadme = readFileSync(join(root, "docs", "plan", "README.md"), "utf8");
  const rows = boardRowsForCrate(dir, root);
  const planFile = planForCrate(rows, parsePrefixTable(planReadme));
  const planLine = planIndexLineOf(planReadme, planFile);

  const head = [
    `# packet: crates/${dir}`,
    `_One crate for one agent: the map slice, the crate row, the plan line and the open board rows. Tokens = bytes / 4, rounded (docs/compaction-baseline.md 1.9)._`,
    "",
    "## Map slice (.opencode/modmap.md)",
    ...slice,
    "",
    "## Crate row (docs/CRATE-MAP.md)",
    crateRow || "(no row in docs/CRATE-MAP.md)",
    "",
    "## Plan line (docs/plan/README.md)",
    planLine || "(no plan line)",
    "",
  ];
  const boardHead = (n, total) => `## Board rows (${n} of ${total} open rows own crates/${dir}; ready first)`;
  const notShown = (ids) => (ids.length ? [`not shown (slice one): ${ids.join(", ")}`] : []);

  // The board rows: ready first, then board order, while the packet stays
  // under the budget. The id tail is reserved for every candidate row, so
  // the packet never grows past the budget when the rows are added.
  const reserved = byteCount([...head, boardHead(rows.length, rows.length), ...notShown(rows.map((r) => r.id)), ""].join("\n")) + PACK_RESERVE_BYTES;
  let room = budget * 4 - reserved;
  const shown = [];
  for (const r of rows) {
    const cost = byteCount(r.line) + 1;
    if (cost > room) continue;
    room -= cost;
    shown.push(r);
  }
  const rest = rows.filter((r) => !shown.includes(r)).map((r) => r.id);
  const body = [
    ...head,
    boardHead(shown.length, rows.length),
    ...shown.map((r) => r.line),
    ...notShown(rest),
    "",
  ];
  const measure = (bytes, tokens) => [
    "## Measure",
    `- bytes: ${bytes}`,
    `- tokens: ${tokens} (bytes / 4, rounded)`,
    `- budget: ${budget} tokens`,
    "",
  ];
  // The packet measures itself: the byte count is part of the text, so the
  // number is found by measuring the text until it stops moving.
  let bytes = 0;
  let tokens = 0;
  let out = "";
  for (let i = 0; i < 5; i += 1) {
    const ok = tokens <= budget;
    out = [
      ...body,
      ...measure(bytes, tokens),
      ok
        ? `PASS crates/${dir} packet: ${tokens} tokens under the ${budget} budget`
        : `FAIL crates/${dir} packet: ${tokens} tokens over the ${budget} budget`,
      "",
    ].join("\n");
    const measured = byteCount(out);
    if (measured === bytes) break;
    bytes = measured;
    tokens = tokenCount(bytes);
  }
  const ok = tokens <= budget;
  return { ok, dir, text: out, bytes, tokens, budget, rows: { shown: shown.length, total: rows.length, ready: rows.filter((r) => r.ready).length }, planFile, planLine, crateRow, sliceLines: slice.length };
}

function main(argv) {
  const mode = argv[0] ?? "--print";
  if (mode === "--pack" || mode === "--packet") {
    if (argv.length !== 2) {
      process.stderr.write("usage: node tools/modmap.mjs --pack <crate>\n");
      return 2;
    }
    const dir = crateDirOf(argv[1]);
    if (!dir) {
      process.stderr.write(`modmap: error: unknown crate ${argv[1]}\n`);
      return 2;
    }
    const packet = buildPacket(dir);
    if (!packet.ok) {
      process.stderr.write(`modmap: error: ${packet.reason}\n`);
      return 2;
    }
    process.stdout.write(packet.text);
    return packet.ok ? 0 : 1;
  }
  if (!["--print", "--write", "--check", "--pack", "--packet", "--help", "-h"].includes(mode) || argv.length > 1) {
    process.stderr.write("usage: node tools/modmap.mjs [--write | --check | --pack <crate>]\n");
    return 2;
  }
  if (mode === "--help" || mode === "-h") {
    process.stdout.write("usage: node tools/modmap.mjs [--write | --check | --pack <crate>]\n");
    return 0;
  }
  const text = render(collect());
  if (mode === "--print") process.stdout.write(text);
  if (mode === "--write") {
    writeFileSync(MAP, text);
    process.stdout.write(`modmap: wrote ${MAP}\n`);
  }
  if (mode === "--check") {
    const old = existsSync(MAP) ? readFileSync(MAP, "utf8").replace(/\r\n/g, "\n") : "";
    if (old !== text) {
      const d = diffSummary(old, text);
      process.stdout.write(`FAIL .opencode/modmap.md: +${d.added} / -${d.removed} lines; run node tools/modmap.mjs --write\n`);
      return 1;
    }
    process.stdout.write("PASS .opencode/modmap.md\n");
  }
  return 0;
}

if (import.meta.main) process.exitCode = main(process.argv.slice(2));
