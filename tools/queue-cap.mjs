// queue-cap: may a research seat add a packet, or must it verify the queue? (owner 2026-10-10)
//   node tools/queue-cap.mjs              one line: ready N of cap M -> ADD or VERIFY
//   node tools/queue-cap.mjs --mix        the seat mix from the knobs: role, count, class
//   node tools/queue-cap.mjs --verify-one <f>  cap plus check in one call: cap line then PASS/FAIL (exit 0 on ADD + PASS)
//   node tools/queue-cap.mjs --check <f>  FAIL unless the packet has its keys, a role that is a
//                                         seat, paths: that exist (or are (new) and absent),
//                                         max_diff <= 220, and a test line the seat's class allows
//                                         (TEST_FORMS): a code seat runs cargo test -p (cargo filter = a
//                                         real fn name) or node --test; a write seat runs node --test or
//                                         agentlint on a skill; a research, run or read seat names its
//                                         done-when check (report <path> exists, rg, node <script>).
//                                         No placeholder <...> in a test line. One packet per row: FAIL when an
//                                         older ready/ packet already names a row of this one, or done/ holds
//                                         a packet or run for it written in the last 2 h (the message names it).
// Cap = knob queue_per_writer x (coders + smiths) in .opencode/knobs.json. Exit 0 ADD/PASS, 1 VERIFY/FAIL, 2 usage.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { classOf, queueCap, readKnobs, seatMix } from "../.opencode/seat-guard.mjs";
import { archivedDoneRows, loadBoardText, parseBoard } from "./board-slice.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const NEEDED = ["role", "title", "row", "paths", "test", "max_diff", "source"];
export const MAX_DIFF = 220;
export const STALE_MIN = 120; // FairTS idea-only, no code copied; minutes before ready reads stale
export const staleAges = (dir, now = Date.now()) => {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((n) => n.endsWith(".md")).map((n) => Math.max(0, Math.floor((now - statSync(join(dir, n)).mtimeMs) / 60_000))).sort((a, b) => b - a);
};
// cargo test flags that take a value: the token after them is not the test name.
const VALUE_FLAGS = new Set(["-p", "--package", "--features", "-F", "--test", "--bin", "--example", "--bench", "--profile", "--target", "-j", "--jobs", "--manifest-path", "--target-dir", "--exclude", "--message-format", "--color"]);

export const frontMatter = (text) => {
  const m = String(text ?? "").replace(/\r\n/g, "\n").match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!m) return null;
  const fm = Object.fromEntries(m[1].split("\n").map((l) => l.match(/^([\w-]+):\s*(.*)$/)).filter(Boolean).map((x) => [x[1], x[2].trim()]));
  return { fm, body: m[2] };
};

// `paths:` entries: { raw, path, isNew, isDir }. A trailing "(new)" marks a file the packet creates.
// Repo paths never contain spaces, so entries split on commas and on whitespace between paths
// (a repair `paths:` or a standing-run `OWNED:` line may list two files space-separated).
export const parsePathEntries = (value) => {
  const raws = [];
  for (const seg of String(value ?? "").split(",")) {
    for (const tok of seg.trim().split(/\s+/).filter(Boolean)) {
      if (/^\(new\)$/i.test(tok) && raws.length && !/\(new\)\s*$/i.test(raws[raws.length - 1])) raws[raws.length - 1] += ` ${tok}`;
      else raws.push(tok);
    }
  }
  return raws.map((raw) => {
    const body = raw.replace(/\s*\(new\)\s*$/i, "").trim().replace(/\\/g, "/");
    return { raw, path: body.replace(/\/+$/, ""), isNew: /\(new\)\s*$/i.test(raw), isDir: body.endsWith("/") };
  });
};

// A new path's folder exists, or the packet creates it (another listed path lies under the first missing folder).
const folderReady = (entry, entries, root) => {
  let top = posix.dirname(entry.path);
  if (top === "." || existsSync(join(root, top))) return true;
  while (posix.dirname(top) !== "." && !existsSync(join(root, posix.dirname(top)))) top = posix.dirname(top);
  return entries.some((o) => o !== entry && o.path.startsWith(`${top}/`));
};

export const pathProblems = (entries, root = ROOT) => {
  const out = [];
  for (const e of entries) {
    if (!e.path || e.path.startsWith("/") || /^[a-z]:/i.test(e.path) || e.path.split("/").includes("..")) { out.push(`bad path ${e.raw}`); continue; }
    const there = existsSync(join(root, e.path));
    if (e.isNew) {
      if (there) out.push(`${e.path}: (new) but it already exists`);
      else if (!folderReady(e, entries, root)) out.push(`${e.path}: folder ${posix.dirname(e.path)} does not exist and no other listed path creates it`);
    } else if (!there) out.push(`${e.path}: does not exist (mark it (new) if the packet creates it)`);
  }
  return out;
};

// Packages under crates/: package name -> crates/<dir>.
export const packageDirs = (root = ROOT) => {
  const map = new Map();
  const crates = join(root, "crates");
  if (!existsSync(crates)) return map;
  for (const d of readdirSync(crates)) {
    const file = join(crates, d, "Cargo.toml");
    if (!existsSync(file)) continue;
    const name = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(file, "utf8"))?.[1];
    if (name) map.set(name, `crates/${d}`);
  }
  return map;
};

// Every fn name under a folder. A libtest filter matches a substring of a test name.
export const testNames = (dir) => {
  const names = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === "target") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".rs")) for (const m of readFileSync(p, "utf8").matchAll(/\bfn\s+(\w+)/g)) names.push(m[1]);
    }
  };
  if (existsSync(dir)) walk(dir);
  return names;
};

// `cargo test` arguments after the word `test`: the package and the test-name filter.
export const cargoTestArgs = (args) => {
  let pkg = "";
  let filter = "";
  for (let i = 0; i < args.length; i++) {
    const t = args[i];
    if (t === "--") continue;
    if (t.startsWith("-")) {
      if (t === "-p" || t === "--package") pkg = args[++i] ?? "";
      else if (VALUE_FLAGS.has(t)) i++;
      continue;
    }
    if (!filter) filter = t;
  }
  return { pkg, filter };
};

// The test line each seat class may run (owner 2026-10-10; classes from .opencode/knobs.json seat/class, see seat-guard.mjs).
// code: builder, proves its change with cargo test -p or node --test. write: smith, miner, skill-hunter, planner: node --test,
// the agentlint skill check, or a done-when line. research, run, read: porter, rot-hunter, walker, runner, perf, judge: a
// done-when line (report, rg, node script), and cargo or node --test where they already had one.
export const TEST_FORMS = {
  code: ["cargo", "node-test", "exe"],
  write: ["node-test", "agentlint", "report", "rg", "node-script"],
  read: ["cargo", "node-test", "exe", "report", "rg", "node-script"],
  run: ["exe", "report", "rg", "node-script"],
  research: ["cargo", "node-test", "exe", "report", "rg", "node-script"],
};
const FORM_TEXT = {
  cargo: "cargo test -p", "node-test": "node --test", exe: "an existing .exe", agentlint: "node <agentlint.mjs> <repo> (skill packets)",
  report: "report <path> exists", rg: "rg <pattern> <files>", "node-script": "node <existing script>",
};
const words = (line) => String(line).match(/"[^"]*"|'[^']*'|\S+/g) ?? [];
const unquote = (w) => w.replace(/^["']|["']$/g, "");
const underPaths = (rel, entries) => entries.some((e) => rel === e.path || rel.startsWith(`${e.path}/`));
// A repo-relative file, or an absolute one (agentlint.mjs and lead2.mjs live outside the repo).
const hasFile = (p, root) => (/^([a-z]:|\/)/i.test(p) ? existsSync(p) : existsSync(join(root, p)));

// The form of a test line, from its first command; null when it is none of the forms.
export const testForm = (line, { root = ROOT } = {}) => {
  const exe = line.split(/\s+/)[0];
  if (/^cargo test -p\s/.test(line)) return "cargo";
  if (/^node --test(\s|$)/.test(line)) return "node-test";
  if (/\.exe$/i.test(exe) && existsSync(join(root, exe))) return "exe";
  if (/^node\s+\S*agentlint\.mjs\s/.test(line)) return "agentlint";
  if (/^report\s+\S+\s+exists$/.test(line)) return "report";
  if (/^!?\s*rg\s/.test(line)) return "rg";
  if (/^node\s+\S+\.m?js(\s|$)/.test(line)) return "node-script";
  return null;
};

export const testProblems = (test, { root = ROOT, entries = [], cls = "code" } = {}) => {
  const out = [];
  const line = String(test ?? "").trim();
  const seat = TEST_FORMS[cls] ? cls : "code";
  const form = testForm(line, { root });
  if (!form || !TEST_FORMS[seat].includes(form)) out.push(`test line must start with ${TEST_FORMS[seat].map((f) => FORM_TEXT[f]).join(", ")} for a ${seat} seat`);
  if (/<[\w .,-]+>/.test(line)) out.push("test line has a <placeholder>: write the real value");
  if (form === "report") {
    const rel = line.split(/\s+/)[1].replace(/\\/g, "/");
    if (!underPaths(rel, entries)) out.push(`report ${rel} is not in paths:`);
  }
  if (form === "agentlint") {
    const script = line.split(/\s+/)[1];
    if (!hasFile(script, root)) out.push(`agentlint script ${script} does not exist`);
    if (!entries.some((e) => /^(\.opencode\/)?skills\//.test(e.path))) out.push("agentlint is the skill check: paths: must name a skill folder");
  }
  if (form === "node-script") {
    const script = words(line).slice(1).find((w) => !w.startsWith("-"));
    if (!script || !hasFile(script, root)) out.push(`node script ${script ?? "(none)"} does not exist`);
  }
  if (form === "rg") {
    const w = words(line);
    if (w[0] === "!") w.shift();
    w.shift(); // rg, or !rg
    const operands = w.filter((x) => !x.startsWith("-")).map(unquote);
    const files = operands.slice(1);
    if (!files.length) out.push("rg names no file");
    for (const f of files) if (!existsSync(join(root, f)) && !underPaths(f.replace(/\\/g, "/"), entries)) out.push(`rg file ${f} is neither in the repo nor in paths:`);
  }
  const packages = packageDirs(root);
  for (const seg of line.split(";").map((s) => s.trim()).filter(Boolean)) {
    const tok = seg.split(/\s+/);
    if (tok[0] === "cargo" && tok[1] === "test") {
      const { pkg, filter } = cargoTestArgs(tok.slice(2));
      const dir = packages.get(pkg);
      if (!pkg) out.push("cargo test has no -p package");
      else if (!dir) out.push(`unknown package ${pkg}`);
      else if (filter && !testNames(join(root, dir)).some((n) => n.includes(filter))) out.push(`test filter ${filter} matches no test name in ${dir}`);
    } else if (tok[0] === "node" && tok.includes("--test")) {
      for (const f of tok.slice(tok.indexOf("--test") + 1)) {
        if (f.startsWith("-") || !/\.(?:m|c)?js$/.test(f)) continue;
        if (!existsSync(join(root, f)) && !entries.some((e) => e.isNew && e.path === f)) out.push(`test file ${f} does not exist`);
      }
    }
  }
  return out;
};

// The board row ids a `row:` value names (EG12, MP6, MP17). Category tags (rot:unwired-testonly, bug:x, new) name none.
export const rowKeys = (value) => String(value ?? "").match(/\b[A-Z][A-Z0-9]*\d[a-z]?\b/g) ?? [];
const recordRow = (text) => frontMatter(text)?.fm?.row ?? [...String(text ?? "").matchAll(/\bROW:\s*([^|\n`]+)/g)].at(-1)?.[1] ?? "";
export const DONE_WAIT_MS = 2 * 3_600_000;

// The ids of every row on the board (index, shards, archived done rows); null when the tree has no board to read.
export const boardRowIds = (root = ROOT) => {
  try { return new Set(parseBoard(loadBoardText(root), archivedDoneRows(root)).tasks.keys()); } catch { return null; }
};

// One packet per row (2026-10-12 flow audit: 5 rows had 2 ready packets at once, EG8 was planned twice). Another packet in
// ready/ that names a row id of this one stands if it is older (ties: the smaller name); done/ holding a packet or a standing
// run for the row, written in the last 2 h, means its review and landing are still in flight. `self` = this packet's own
// file name, which is never its own twin. A packet that is not in ready/ yet meets every queued one. Only ids that are
// rows on the board count: `row: LEAD2` or `row: J5` tag a whole class of packets, not one row.
export const rowProblems = (fm, { root = ROOT, self = "", now = Date.now() } = {}) => {
  const keys = rowKeys(fm?.row);
  if (!keys.length) return [];
  const out = [];
  let ids; // the board is read once, and only when two packets might share a row
  const isRow = (k) => (ids === undefined ? (ids = boardRowIds(root)) : ids) === null || ids.has(k);
  const queue = join(root, "target", "queue");
  const names = (dir) => (existsSync(join(queue, dir)) ? readdirSync(join(queue, dir)).filter((n) => n.endsWith(".md") && n !== self) : []);
  const own = self && existsSync(join(queue, "ready", self)) ? statSync(join(queue, "ready", self)).mtimeMs : Infinity;
  for (const n of names("ready")) {
    const file = join(queue, "ready", n);
    const theirs = rowKeys(frontMatter(readFileSync(file, "utf8"))?.fm?.row);
    const hit = keys.find((k) => theirs.includes(k) && isRow(k));
    if (!hit) continue;
    const at = statSync(file).mtimeMs;
    if (at < own || (at === own && n < self)) out.push(`row ${hit} is already named by ready/${n}: one packet per row (fix that packet, or park it in target/queue/parked/)`);
  }
  for (const n of names("done").filter((x) => !/-review(?:-\d+)?\.md$/.test(x))) {
    const file = join(queue, "done", n);
    const age = now - statSync(file).mtimeMs;
    if (age >= DONE_WAIT_MS) continue;
    const theirs = rowKeys(recordRow(readFileSync(file, "utf8")));
    const hit = keys.find((k) => theirs.includes(k) && isRow(k));
    if (hit) out.push(`row ${hit} has done/${n}, written ${Math.max(0, Math.round(age / 60_000))} min ago (under 2 h): let its review and landing finish before planning the row again`);
  }
  return out;
};

export const checkPacket = (text, { root = ROOT, knobs = readKnobs(ROOT), self = "", now = Date.now() } = {}) => {
  const p = frontMatter(text);
  if (!p) return ["no front matter (--- block)"];
  const miss = NEEDED.filter((k) => !p.fm[k]).map((k) => `no ${k}:`);
  for (const w of ["Goal", "Scope", "Proof", "Stop"]) if (!new RegExp(`\\b${w}\\b`).test(p.body)) miss.push(`no ${w} line`);
  if (p.fm.max_diff && !(Number(p.fm.max_diff) > 0)) miss.push("max_diff is not a number");
  else if (p.fm.max_diff && Number(p.fm.max_diff) > MAX_DIFF) miss.push(`max_diff ${p.fm.max_diff} is over ${MAX_DIFF}`);
  if (p.fm.role && !classOf(p.fm.role, knobs)) miss.push(`role ${p.fm.role} is not a seat`);
  const entries = parsePathEntries(p.fm.paths);
  if (p.fm.paths) miss.push(...pathProblems(entries, root));
  if (p.fm.test) miss.push(...testProblems(p.fm.test, { root, entries, cls: classOf(p.fm.role, knobs) }));
  miss.push(...rowProblems(p.fm, { root, self, now }));
  return miss;
};

// Ready packets a coder or smith takes (role builder or a write/code seat).
export const countReady = (dir, knobs) => {
  if (!existsSync(dir)) return 0;
  return readdirSync(dir).filter((n) => n.endsWith(".md")).filter((n) => {
    const role = frontMatter(readFileSync(join(dir, n), "utf8"))?.fm?.role;
    return ["code", "write"].includes(classOf(role, knobs));
  }).length;
};

// One-line cap verdict shared by the bare call and --verify-one (one owner, no dup body).
export const capStatus = (root = ROOT, knobs = readKnobs(root)) => {
  const cap = queueCap(knobs);
  const readyDir = join(root, "target", "queue", "ready");
  const n = countReady(readyDir, knobs);
  const ages = staleAges(readyDir);
  const stale = ages.filter((a) => a >= STALE_MIN).length;
  const oldest = ages.length ? ages[0] : 0;
  const suffix = `; stale ${stale} oldest ${oldest}min`;
  const line = n >= cap
    ? `ready ${n} of cap ${cap} -> VERIFY: do not add; repro a queued packet, tighten its paths:, move a stale one to target/queue/parked/ (a bug you saw in the built product still goes in: row: bug:<slug>, the keeper runs it next)${suffix}`
    : `ready ${n} of cap ${cap} -> ADD: you may write ${cap - n} more packet(s) to target/queue/ready/${suffix}`;
  return { n, cap, add: n < cap, line };
};

// Research ran cap then check per packet (3 calls for cap + 2 checks). verifyOne chains
// both in one call: cap line plus the packet check. Exit 0 only on ADD + PASS.
export const verifyOne = (packetPath, { root = ROOT, knobs = readKnobs(root), now = Date.now() } = {}) => {
  const cap = capStatus(root, knobs);
  const text = existsSync(packetPath) ? readFileSync(packetPath, "utf8") : "";
  const miss = checkPacket(text, { root, knobs, self: basename(packetPath), now });
  return { capLine: cap.line, miss, ok: cap.add && miss.length === 0, n: cap.n, cap: cap.cap };
};

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args[0] === "--check") {
    if (!args[1]) { console.log("usage: node tools/queue-cap.mjs --check <packet.md>"); process.exit(2); }
    const miss = checkPacket(existsSync(args[1]) ? readFileSync(args[1], "utf8") : "", { self: basename(args[1]) });
    console.log(miss.length ? `FAIL ${args[1]}: ${miss.join(", ")}` : `PASS ${args[1]}`);
    process.exit(miss.length ? 1 : 0);
  }
  if (args[0] === "--mix") {
    const { seats, total } = seatMix(readKnobs(ROOT));
    console.log(seats.map((s) => `${s.seat} ${s.n} (${s.cls})`).join(" | ") + ` = ${total} seats`);
    process.exit(0);
  }
  if (args[0] === "--verify-one") {
    if (!args[1]) { console.log("usage: node tools/queue-cap.mjs --verify-one <packet.md>"); process.exit(2); }
    const r = verifyOne(args[1], { root: ROOT });
    console.log(r.capLine);
    console.log(r.miss.length ? `FAIL ${args[1]}: ${r.miss.join(", ")}` : `PASS ${args[1]}`);
    process.exit(r.ok ? 0 : 1);
  }
  if (args.length) { console.log("usage: node tools/queue-cap.mjs [--mix | --check <packet.md> | --verify-one <packet.md>]"); process.exit(2); }
  const cap = capStatus(ROOT);
  console.log(cap.line);
  process.exit(cap.add ? 0 : 1);
}
