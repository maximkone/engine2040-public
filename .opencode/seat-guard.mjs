// Seat guard (owner 2026-10-10: "fix my opencode muse setup so he wont drift"): the rules
// that keep each loop seat in its lane, as code. The loop-keeper plugin loads this file
// when it exists (a temp copy, so only node: imports here) and asks it before each tool call.
// Pure decisions plus the cargo slot files; the keeper finds the seat and its packet.
//
// Seat classes (the `class` of a knob with a `seat` in .opencode/knobs.json; defaults below):
//   lead      the sprint session: anything but Rust.
//   code      coders: Rust inside the packet's `paths:` (plus their tests) and target/.
//   write     smiths, miners, skill hunters, planners: their claimed paths and target/; never Rust; never commit.
//   read      judges: target/ only; cargo allowed.
//   run       runners, perf: target/ only; never cargo (they use the built binary).
//   research  porters, rot hunters, walkers, researchers: target/queue/ and target/reports/ only.
// Every guarded seat: no `git add -A`, reset --hard, checkout --, clean, rebase, force push,
// worktree add, `lanes.mjs create`. Owner chats (no seat) are never guarded.
// Shell writes count like edits (the lead is not checked): the file a command writes (redirect `>`, Set-Content,
// Copy-Item, `node -e` writeFileSync, git rm ...) must be one the seat may edit. Research, run and read seats: only
// target/. Write and code seats: their packet paths. A write whose path cannot be read (variable, pipeline) is blocked
// for research and run seats and allowed for the rest.
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, posix } from "node:path";

export const DEFAULT_CLASSES = {
  orchestrator: "lead", builder: "code", judge: "read", pilot: "run",
  researcher: "research", planner: "write", explore: null, general: null,
};
export const CLASSES = ["lead", "code", "write", "read", "run", "research"];
const CARGO_OK = new Set(["lead", "code", "read"]);
const COMMIT_OK = new Set(["lead"]); // write seats never commit for now (owner 2026-10-10): judges and the lead land their work
const CLAIM_MAX_MS = 3 * 3_600_000; // a claim older than this is stale
export const CARGO_SLOT_MAX_MS = 75 * 60_000; // build timeout 45 min + heavy wait 30 min

// ---- knobs ----
export const readKnobs = (root) => {
  try { return JSON.parse(readFileSync(join(root, ".opencode", "knobs.json"), "utf8"))?.knobs ?? {}; } catch { return {}; }
};
export const knob = (knobs, name, dflt) => {
  const v = Number(knobs?.[name]?.value);
  return Number.isFinite(v) ? v : dflt;
};
// role -> class: the defaults, then every seat knob (`seat` + `class`).
export const classMap = (knobs) => {
  const out = { ...DEFAULT_CLASSES };
  for (const k of Object.values(knobs ?? {})) {
    if (k && typeof k.seat === "string" && CLASSES.includes(k.class)) out[k.seat.toLowerCase()] = k.class;
  }
  return out;
};
export const classOf = (role, knobs) => {
  const r = String(role ?? "").toLowerCase().replace(/-paid$/, "");
  return r ? classMap(knobs)[r] ?? null : null;
};
// The seat mix: [{ knob, seat, cls, n }] and the total.
export const seatMix = (knobs) => {
  const seats = Object.entries(knobs ?? {}).filter(([, k]) => k && typeof k.seat === "string")
    .map(([name, k]) => ({ knob: name, seat: k.seat, cls: k.class, n: Number(k.value) || 0 }));
  return { seats, total: seats.reduce((s, x) => s + x.n, 0) };
};
// Ready packets may reach queue_per_writer x (coders + smiths).
export const queueCap = (knobs) => knob(knobs, "queue_per_writer", 3) * (knob(knobs, "coders", 1) + knob(knobs, "smiths", 0));

// ---- paths ----
const slash = (p) => String(p ?? "").trim().replace(/^["'`]|["'`]$/g, "").replace(/\\/g, "/");
// A path relative to the repo root, lower case, `.` and `..` resolved (`target/../docs/a.md` is docs/a.md);
// null when it lies outside the root.
export const relPath = (p, root) => {
  let s = slash(p);
  if (s) s = posix.normalize(s);
  const r = slash(root).replace(/\/+$/, "");
  if (/^[a-z]:\//i.test(s) || s.startsWith("/")) {
    if (!s.toLowerCase().startsWith(`${r.toLowerCase()}/`)) return null;
    s = s.slice(r.length + 1);
  }
  s = s.replace(/^\.\//, "").toLowerCase();
  return s === ".." || s.startsWith("../") ? null : s;
};
const isTemp = (p) => {
  const s = slash(p).toLowerCase();
  const t = slash(tmpdir()).toLowerCase();
  return s.startsWith(`${t}/`) || /\/appdata\/local\/temp\//.test(s);
};
// `paths:` values: comma or space separated; a trailing / or /** is a folder; * matches in one segment.
export const parsePaths = (v) => String(v ?? "").split(/[,\s]+/).map((x) => slash(x).replace(/^\.\//, "").toLowerCase()).filter((x) => x && /[/.]/.test(x));
const globRe = (g) => new RegExp(`^${g.replace(/[.+^${}()|[\]]/g, "\\$&").replace(/\*\*/g, "\u0000").replace(/\*/g, "[^/]*").replace(/\u0000/g, ".*")}$`);
// Windows paths: case never decides (a packet's skill.md is the repo's SKILL.md).
export const pathMatch = (relIn, entry) => {
  const rel = relIn.toLowerCase();
  const e = entry.toLowerCase().replace(/\/\*\*$/, "/");
  if (e.endsWith("/")) return rel.startsWith(e);
  if (e.includes("*")) return globRe(e).test(rel);
  return rel === e;
};
// The tests that belong to an allowed path: the crate's tests/ folder, *_test.rs / tests.rs beside it,
// <stem>/tests.rs, and <stem>*.test.mjs beside a .mjs.
export const isTestOf = (rel, entry) => {
  const crate = entry.match(/^(crates\/[^/]+)\//);
  if (crate && rel.startsWith(`${crate[1]}/tests/`)) return true;
  const dir = entry.includes("/") ? entry.slice(0, entry.lastIndexOf("/") + 1) : "";
  const stem = entry.slice(dir.length).replace(/\.[^.]+$/, "");
  if (!rel.startsWith(dir)) return false;
  const rest = rel.slice(dir.length);
  if (/\.rs$/.test(entry)) return /^(?:[\w-]*_tests?|tests)\.rs$/.test(rest) || rest === `${stem}/tests.rs` || rest.startsWith(`${stem}/tests/`);
  if (/\.(?:mjs|js)$/.test(entry)) return rest.startsWith(stem) && /\.test\.mjs$/.test(rest);
  return false;
};

// ---- claims (target/queue/claims.txt: `<seat> | <UTC> | <ID> <paths>`) ----
export const claimLines = (text, now = Date.now()) => String(text ?? "").split(/\r?\n/).map((line) => {
  let cells = line.split("|").map((c) => c.trim());
  if (cells.length < 3 || !cells[0]) return null;
  // The keeper's older note wrote `<item ID> | <run> | <UTC> | <paths>`: read it as `<run> | <UTC> | <item ID> <paths>`.
  if (cells.length >= 4 && !Number.isFinite(Date.parse(cells[1])) && Number.isFinite(Date.parse(cells[2]))) cells = [cells[1], cells[2], [cells[0], ...cells.slice(3)].join(" ")];
  const at = Date.parse(cells[1]);
  if (Number.isFinite(at) && now - at > CLAIM_MAX_MS) return null;
  const rest = cells.slice(2).join(" ");
  return { seat: cells[0].toLowerCase(), raw: line.trim(), row: (rest.match(/^\s*([A-Z][A-Z0-9]*\d[\w.]*)\b/) ?? [])[1] ?? null, paths: parsePaths(rest.replace(/\([^)]*\)/g, " ")).filter((p) => p.includes("/") || /\.\w+$/.test(p)) };
}).filter(Boolean);
// The paths a seat may write: its packet's `paths:` plus claim lines named for its packet (id, or id-<suffix>).
// A claim named for a sibling run of the same standing packet (`<base>-r2` while this is r3) stays that
// run's: 2026-10-10 skill-hunter runs were bound to each other's skill dirs and refused their own
// (BLOCKERS seat-guard-scope x3).
export const allowedPaths = ({ packet, claims, now }) => {
  const out = new Set(parsePaths(packet?.fm?.paths ?? packet?.fm?.owned));
  const full = String(packet?.id ?? "").toLowerCase();
  const run = (full.match(/-r(\d+)$/) ?? [])[1] ?? null;
  const id = full.replace(/-r\d+$/, "");
  const ids = [id, String(packet?.standing ?? "").toLowerCase().replace(/\.md$/, "")].filter(Boolean);
  const siblingRun = (rest) => { const m = rest.match(/^-r(\d+)(?:-|$)/); return Boolean(m) && m[1] !== run; };
  if (ids.length) {
    for (const c of claimLines(claims, now)) {
      if (ids.some((x) => (c.seat === x || c.seat.startsWith(`${x}-`)) && !siblingRun(c.seat.slice(x.length)))) for (const p of c.paths) out.add(p);
    }
  }
  return [...out];
};
// What a write seat does about an unclaimed path: claim it under its own run name, then retry.
const claimHint = (cls, seatId, rel) => (cls === "write" && seatId
  ? `; if your item owns it, first append \`${seatId} | <UTC yyyy-mm-ddTHH:MMZ> | <ROW or item> ${rel}\` to target/queue/claims.txt, then retry`
  : "");

// ---- edits ----
const EDIT_TOOLS = new Set(["edit", "write", "multiedit", "patch", "apply_patch"]);
export const editTargets = (tool, args) => {
  if (!EDIT_TOOLS.has(tool)) return [];
  if (typeof args?.filePath === "string") return [args.filePath];
  if (typeof args?.path === "string") return [args.path];
  const text = String(args?.patchText ?? args?.patch ?? args?.input ?? "");
  return [...text.matchAll(/^\*\*\* (?:Add|Update|Delete) File:\s*(.+)$|^\*\*\* Move to:\s*(.+)$/gm)].map((m) => (m[1] ?? m[2]).trim());
};
// The board: every docs/SPRINT* shard (SPRINT-18-gaps.md was open to any claim before 2026-10-11), FOCUS, the plan of
// record and docs/EMPIRE.md (a byte mirror of center's copy).
const BOARD = ["docs/sprint", "docs/focus.md", "docs/plan/", "docs/empire.md"];
// The loop's own machinery and gates (2026-10-11): no seat edits them unless its packet's own `paths:` line names the
// file. A claim line is not enough: a seat could claim the guard that judges it.
const MACHINERY = [".opencode/plugin/", ".opencode/seat-guard.mjs", ".opencode/knobs.json", ".opencode/agents/", ".opencode/commands/",
  ".opencode/kernel.md", "opencode.json", "agents.md", "deny.toml", "tools/rust-quality.baseline.json", "tools/hooks/"];
const isRust = (rel) => rel.endsWith(".rs") || rel.startsWith("crates/") || /(^|\/)cargo\.(?:toml|lock)$/.test(rel);
const RESEARCH_OK = ["target/queue/", "target/reports/"];

// One edit target: null = allowed, else the one-line reason.
export const checkEditPath = ({ file, cls, root, allowed, seatId, own }) => {
  const rel = relPath(file, root);
  if (rel === null) return isTemp(file) || cls === "lead" ? null : `out of repo: ${slash(file)}; seats write only inside ${slash(root)} or the temp folder`;
  if (cls === "lead") return rel.endsWith(".rs") || rel.startsWith("crates/") ? `the lead never edits Rust: ${rel}; send a coder packet` : null;
  if (cls === "research") return RESEARCH_OK.some((p) => rel.startsWith(p)) ? null : `research seat: write only a packet under target/queue/ready/ or a report under target/reports/, not ${rel}`;
  if (rel.startsWith("target/sprint")) return `the handoff, lock and halt belong to the lead: ${rel}`;
  if (rel.startsWith("target/")) return null;
  if (cls === "read" || cls === "run") return `${cls === "read" ? "judge" : "runner"} seat: no edits outside target/ (${rel}); report the fix as a packet`;
  if (MACHINERY.some((m) => rel.startsWith(m)) && !(own ?? []).some((a) => pathMatch(rel, a))) return `loop machinery: ${rel} changes only through a packet whose own paths: line names it (owner or Claude); report the fix instead`;
  if (cls === "write" && isRust(rel)) return `no Rust for this seat: ${rel}; file a coder packet under target/queue/ready/`;
  if ((cls === "write" || cls === "code") && BOARD.some((b) => rel.startsWith(b)) && !(allowed ?? []).some((a) => pathMatch(rel, a))) return `the board and FOCUS belong to the lead: ${rel}`;
  const list = allowed ?? [];
  if (!list.length) return `no packet paths: ${rel} is not claimed; the packet needs a \`paths:\` line or a claim \`<packet> | <UTC> | <ID> <paths>\` in target/queue/claims.txt${claimHint(cls, seatId, rel)}`;
  if (list.some((a) => pathMatch(rel, a) || (cls === "code" && isTestOf(rel, a)))) return null;
  // The folder an allowed file lives in (mkdir skills/x/tests for skills/x/tests/pairs.md; 2026-10-10 miners were refused).
  const dir = rel.replace(/\/+$/, "");
  if (dir && list.some((a) => a.toLowerCase().startsWith(`${dir.toLowerCase()}/`))) return null;
  return `out of packet paths: ${rel}; allowed: ${list.slice(0, 6).join(", ")}${list.length > 6 ? ", ..." : ""}${claimHint(cls, seatId, rel)}`;
};

// ---- bash ----
const DENY = [
  // 2026-10-11: target/ holds the loop itself (queue, claims, halt, handoff, cargo slots) beside the builds.
  [/\bcargo(?:\.exe)?\s+(?:\+\S+\s+)?clean\b/, "cargo clean deletes target/: the loop's queue, claims, handoff and every build; delete one build dir by exact path if asked"],
  [/\b(?:rm|Remove-Item|rmdir|rd|del)\b[^;|&\n]*?(?:^|[\s"'=])(?:\.[\\/])?target(?:[\\/]+(?:queue|cargo-slots))?[\\/]*(?=["'\s;|&]|$)/i, "deleting target/ or target/queue removes the loop's queue, claims and handoff; delete exact files only"],
  [/\bgit\s+add\s+(?:[^;|&\n]*\s)?(?:-A|--all|-u|--update)\b/, "git add -A/--all/-u stages other sessions' work; stage your own paths: git add -- <path>"],
  [/\bgit\s+add\s+(?:--\s+)?\.(?=\s|$|[;|&])/, "git add . stages other sessions' work; stage your own paths: git add -- <path>"],
  [/\bgit\s+reset\s+(?:[^;|&\n]*\s)?--hard\b/, "git reset --hard destroys other sessions' work"],
  [/\bgit\s+checkout\s+(?:[^;|&\n]*\s)?--(?:\s|$)/, "git checkout -- discards edits; the lead abandons with git restore --source=HEAD -- <owned>"],
  [/\bgit\s+checkout\s+\.(?=\s|$|[;|&])/, "git checkout . discards every edit in the tree"],
  [/\bgit\s+clean\b/, "git clean deletes other sessions' files"],
  [/\bgit\s+rebase\b/, "never rebase: pull --no-rebase --no-edit"],
  [/\bgit\s+push\s+(?:[^;|&\n]*\s)?(?:--force(?:-with-lease)?|-f)\b/, "never force-push"],
  [/\bgit\s+worktree\s+add\b/, "one tree only: C:\\engine2040 on master, no worktrees"],
  [/\blanes\.mjs\s+create\b/, "lanes are retired: one tree on master"],
  [/\bgit\s+commit\s+(?:[^;|&\n]*\s)?(?:-a|--all|-am)\b/, "git commit -a takes other sessions' work; commit by path: git commit -m <msg> -- <paths>"],
  // 2026-10-10 15:43: a stash before a pull took 54 files of other sessions' edits out of the tree.
  [/\bgit\s+stash\b/, "git stash takes every session's edits out of the shared tree; pull with your own paths committed, or wait"],
  [/\bgit\s+(?:[^;|&\n]*\s)?(?:--autostash|rebase\.autoStash|merge\.autoStash)\b/i, "autostash stashes every session's edits; pull --no-rebase --no-edit, never --autostash"],
  [/\bgit\s+restore\s+(?:[^;|&\n]*\s)?--\s+\.(?=\s|$|[;|&])/, "git restore -- . discards every session's edits; restore your own paths only"],
];
export const CARGO_CMD = /(?:^|[;&|(\n]|\$\(|&\s*)\s*(?:&\s*)?cargo(?:\.exe)?(?:\s|$)/;
export const isCargo = (cmd) => CARGO_CMD.test(String(cmd ?? ""));
// Runner and perf seats may rebuild a release binary (the `rebuild:` line of
// tools/bins.mjs): one `cargo build --release -p <pkg> ... --bin <name>`, env
// assignments before it, nothing chained after it, no other target dir.
export const RUN_BUILD = /^\s*(?:\$env:\w+\s*=\s*(?:'[^']*'|"[^"]*")\s*;\s*)*cargo(?:\.exe)?\s+build\s+--release\s+-p\s+[\w-]+(?:\s+--features\s+[\w,-]+)?\s+--bin\s+[\w-]+\s*$/;
// The repo paths a `git add` / `git commit -- <paths>` names (flags and messages skipped).
const gitPathspecs = (cmd, verb) => {
  const out = [];
  for (const m of String(cmd).matchAll(new RegExp(`\\bgit\\s+${verb}\\b([^;|&\\n]*)`, "g"))) {
    let seg = m[1].replace(/(?:-m|--message|-F|--file)\s+("[^"]*"|'[^']*'|\S+)/g, " ");
    if (verb === "commit") { const i = seg.indexOf(" -- "); if (i < 0) { out.push(null); continue; } seg = seg.slice(i + 4); }
    for (const t of seg.match(/"[^"]*"|'[^']*'|\S+/g) ?? []) if (t !== "--" && !t.startsWith("-")) out.push(slash(t));
  }
  return out;
};

// ---- shell writes ----
// What a command writes, read without running it. scanShell splits it into statements (quote-aware: a `>` inside a
// string is text), each with its words (quotes removed) and its redirect targets. shellWrites reads the paths those
// write; `blind` is true when some write has a path that cannot be read (a variable, a pipeline, built code).
const SINK = /^(?:\$null|nul|con|\/dev\/(?:null|stdout|stderr))$/i;
export const scanShell = (src) => {
  const text = String(src ?? "");
  const out = [];
  let words = [];
  let redirects = [];
  let i = 0;
  const end = () => { if (words.length || redirects.length) out.push({ words, redirects }); words = []; redirects = []; };
  const word = () => {
    let w = "";
    let any = false;
    while (i < text.length) {
      const c = text[i];
      if (c === "'" || c === '"') {
        any = true;
        for (i++; i < text.length; i++) {
          const d = text[i];
          if (d === c) { if (text[i + 1] !== c) break; w += c; i++; continue; } // a doubled quote is one quote
          if (c === '"' && d === "`" && i + 1 < text.length) { w += text[++i]; continue; }
          if (c === '"' && d === "\\" && text[i + 1] === '"') { w += text[++i]; continue; }
          w += d;
        }
        i++;
        continue;
      }
      if (c === "`" && i + 1 < text.length) { w += text[i + 1]; i += 2; any = true; continue; }
      if (/[\s;|&<>]/.test(c)) break;
      w += c;
      i++;
      any = true;
    }
    return any ? w : null;
  };
  while (i < text.length) {
    const c = text[i];
    if (c === "\n" || c === "\r" || c === ";") { end(); i++; continue; }
    if (/\s/.test(c)) { i++; continue; }
    if (c === "#") { while (i < text.length && text[i] !== "\n") i++; continue; }
    const cont = /^`\r?\n/.exec(text.slice(i, i + 3)); // a backtick at the line end joins the next line
    if (cont) { i += cont[0].length; continue; }
    const redir = /^(?:[0-9*]|&)?>>?\|?(&[0-9])?/.exec(text.slice(i, i + 8));
    if (redir) {
      i += redir[0].length;
      if (redir[1]) continue; // 2>&1 joins two streams, no file
      while (i < text.length && /[ \t]/.test(text[i])) i++;
      const target = word();
      if (target !== null && !SINK.test(target)) redirects.push(target);
      continue;
    }
    if (c === "|") { end(); i += text[i + 1] === "|" ? 2 : 1; continue; }
    if (c === "&") { if (text[i + 1] === "&") { end(); i += 2; } else { i++; if (words.length) end(); } continue; } // a first `&` is pwsh's call operator
    if (c === "<") { i += text[i + 1] === "<" ? 2 : 1; word(); continue; } // input: never a write
    if (c === "{" || c === "(" || ((c === "$" || c === "@") && text[i + 1] === "(")) { end(); i += c === "{" || c === "(" ? 1 : 2; continue; } // a block or subexpression starts a statement
    const here = /^@(['"])\r?\n/.exec(text.slice(i, i + 4));
    if (here) { // a here-string is one word
      const close = text.indexOf(`\n${here[1]}@`, i);
      words.push(text.slice(i + here[0].length, close < 0 ? text.length : close));
      i = close < 0 ? text.length : close + 3;
      continue;
    }
    const w = word();
    if (w === null) { i++; continue; }
    if (/^[(){}]$/.test(w)) end();
    else words.push(w);
  }
  end();
  return out;
};

// -Path style parameters: the exact names, plus a unique prefix of one (-Lit, -Dest). Every other `-x` takes no value
// unless listed in VALUED.
const PATH_PARAMS = ["path", "literalpath", "filepath", "pspath", "destination", "destinationpath", "outfile"];
const VALUED = new Set([...PATH_PARAMS, "value", "encoding", "stream", "filter", "include", "exclude", "width", "itemtype", "type", "delimiter",
  "newname", "name", "inputobject", "credential", "errorvariable", "ev", "erroraction", "ea", "warningaction", "wa", "informationaction", "ia",
  "outvariable", "ov", "outbuffer", "ob", "pipelinevariable", "pv", "variable", "uri", "body", "method", "headers", "contenttype", "depth", "o", "output"]);
const argsOf = (a) => {
  const named = {};
  const pos = [];
  for (let k = 0; k < a.length; k++) {
    const m = /^--?([A-Za-z][\w-]*)(?:[:=](.*))?$/.exec(a[k]);
    if (!m) { if (a[k] !== "--" && !/^\/[a-z?][\w:,-]*$/i.test(a[k])) pos.push(a[k]); continue; } // `--` and cmd-style /q /MIR are no paths
    const key = m[1].toLowerCase();
    const hits = key.length > 1 ? PATH_PARAMS.filter((p) => p.startsWith(key)) : [];
    const name = VALUED.has(key) ? key : hits.length === 1 ? hits[0] : null;
    if (name === null) continue;
    const val = m[2] !== undefined ? m[2] : a[++k];
    if (val !== undefined) (named[name] ??= []).push(val);
  }
  return { named, pos };
};
const names = (xs) => xs.flatMap((x) => String(x).split(",")).map((x) => x.trim()).filter(Boolean);
const folded = (dest, src) => (/[\\/]$/.test(dest) && src ? `${dest}${String(src).replace(/^.*[\\/]/, "")}` : dest);
const none = { t: [], blind: false };
const unread = { t: [], blind: true };
// verb -> how its arguments name what it writes. "first": the first path; "all": every path; "new": New-Item.
const SHELL_VERBS = {
  "set-content": "first", sc: "first", "add-content": "first", ac: "first", "clear-content": "first", clc: "first", "out-file": "first",
  "tee-object": "tee", tee: "all", "export-csv": "first", "export-clixml": "first", "start-transcript": "first",
  "new-item": "new", ni: "new", mkdir: "all", md: "all", touch: "all",
  "remove-item": "all", rm: "all", ri: "all", del: "all", erase: "all", rd: "all", rmdir: "all",
  "copy-item": "copy", cp: "copy", cpi: "copy", copy: "copy", "move-item": "move", mv: "move", mi: "move", move: "move",
  "rename-item": "rename", ren: "rename", rni: "rename",
  "invoke-webrequest": "download", iwr: "download", "invoke-restmethod": "download", irm: "download", curl: "download", wget: "download",
  "expand-archive": "archive", "compress-archive": "archive", robocopy: "robocopy", xcopy: "robocopy",
};
const verbWrites = (kind, a) => {
  const { named, pos } = argsOf(a);
  const given = [...(named.path ?? []), ...(named.literalpath ?? []), ...(named.filepath ?? []), ...(named.pspath ?? [])];
  const dest = named.destination?.[0] ?? named.destinationpath?.[0];
  const found = (t) => (t.length ? { t, blind: false } : unread);
  switch (kind) {
    case "first": return found(names(given.length ? given : pos.slice(0, 1)));
    case "tee": return named.variable ? none : found(names(given.length ? given : pos.slice(0, 1)));
    case "all": return found(names([...given, ...pos]));
    case "new": {
      const base = given[0] ?? pos[0];
      const name = named.name?.[0];
      return found(base || name ? [base && name ? `${base}/${name}` : base ?? name] : []);
    }
    case "copy": case "move": {
      const to = dest ?? (pos.length > 1 ? pos[pos.length - 1] : undefined);
      if (to === undefined) return unread;
      const from = names(given.length ? given : dest !== undefined ? pos : pos.slice(0, -1));
      return { t: [folded(to, from[0]), ...(kind === "move" ? from : [])], blind: false };
    }
    case "rename": {
      const from = given[0] ?? pos[0];
      const to = named.newname?.[0] ?? (given.length ? pos[0] : pos[1]);
      return from === undefined ? unread : { t: [from, ...(to ? [`${from.replace(/[^\\/]*$/, "")}${to}`] : [])], blind: false };
    }
    case "download": { const t = names([...(named.outfile ?? []), ...(named.o ?? []), ...(named.output ?? [])]); return t.length ? { t, blind: false } : none; }
    case "archive": return found(names([...(dest !== undefined ? [dest] : pos.slice(1, 2))]));
    case "robocopy": return found(pos.slice(1, 2));
    default: return unread;
  }
};
// Code strings: `node -e` and `python -c`. A call with a string-literal path gives that path, any other call is blind.
const JS_WRITE = /\b(writeFileSync|appendFileSync|writeFile|appendFile|createWriteStream|copyFileSync|copyFile|cpSync|renameSync|rename|rmSync|rm|unlinkSync|unlink|rmdirSync|rmdir|mkdirSync|mkdir|truncateSync|truncate)\s*\(\s*(?:(['"`])(.*?)\2(?:\s*,\s*(['"`])(.*?)\4)?)?/g;
const jsWrites = (code) => {
  const t = [];
  let blind = false;
  for (const m of String(code).matchAll(JS_WRITE)) {
    const copy = /^(?:copy|cp)/.test(m[1]);
    const picks = copy ? [m[5]] : /^rename/.test(m[1]) ? [m[3], m[5]] : [m[3]];
    if (m[3] === undefined || (copy && m[5] === undefined)) blind = true;
    for (const p of picks) if (p !== undefined) { if (p.includes("${")) blind = true; else t.push(p); }
  }
  return { t, blind };
};
const PY_WRITE = /\bopen\(\s*(?:(['"])(.*?)\1)?[^)]*?,\s*(?:mode\s*=\s*)?(['"])[^'"]*[wax+][^'"]*\3|\b(?:shutil\.(?:copy2?|copyfile|move|rmtree|copytree)|os\.(?:remove|unlink|rename|replace|rmdir|mkdir|makedirs)|\w+\.(?:write_text|write_bytes|unlink|mkdir|rmdir|touch))\s*\(\s*(?:(['"])(.*?)\4)?/g;
const pyWrites = (code) => {
  const t = [];
  let blind = false;
  for (const m of String(code).matchAll(PY_WRITE)) { const p = m[2] ?? m[5]; if (p === undefined) blind = true; else t.push(p); }
  return { t, blind };
};
// `$env:TEMP` and friends become the temp folder; any other variable makes the path unreadable (null).
const resolveTarget = (raw) => {
  let s = String(raw).trim().replace(/^\.[\\/]+/, "");
  s = s.replace(/\$\{?env:(?:TEMP|TMP)\}?|%(?:TEMP|TMP)%/gi, slash(tmpdir())).replace(/^~(?=[\\/]|$)/, slash(homedir()));
  for (const [open, close] of ["()", "{}"]) {
    while (s.endsWith(close) && s.split(close).length > s.split(open).length) s = s.slice(0, -1);
  }
  if (!s || s === "-" || /[$%`]|^[({@]/.test(s)) return null;
  return s;
};
const baseOf = (w) => String(w).replace(/^(?:\$\(|@\(|[({])+/, "").replace(/^.*[\\/]/, "").replace(/\.(?:exe|cmd|bat|ps1)$/i, "").toLowerCase();
// Nested scripts: `pwsh -Command "..."`, `bash -c '...'`, `cmd /c ...`, `powershell -EncodedCommand <base64>`.
const SHELLS = new Set(["pwsh", "powershell", "bash", "sh", "zsh", "cmd", "wsl"]);
const nestedScript = (verb, a) => {
  const k = a.findIndex((x) => /^(?:-c|-lc|-command|\/c|\/k)$/i.test(x));
  if (k >= 0) return a.slice(k + 1).join(" ");
  const e = /^(?:pwsh|powershell)$/.test(verb) ? a.findIndex((x) => /^-(?:e|ec|enc|encodedcommand)$/i.test(x)) : -1;
  return e >= 0 && a[e + 1] ? Buffer.from(a[e + 1], "base64").toString("utf16le") : null;
};
const IO_STATIC = /\[(?:System\.)?IO\.(?:File|Directory)\]::(\w+)\s*\(\s*(?:(['"])(.*?)\2(?:\s*,\s*(['"])(.*?)\4)?)?/gi;

export const shellWrites = (command, depth = 0) => {
  const targets = [];
  let blind = false;
  const add = (r) => {
    for (const raw of r.t) { const t = resolveTarget(raw); if (t === null) blind = true; else targets.push(t); }
    if (r.blind) blind = true;
  };
  const nest = (code) => { // a script inside a string: read it the same way, three levels deep at most
    if (depth >= 3) return add(unread);
    const r = shellWrites(code, depth + 1);
    add({ t: r.targets, blind: r.blind });
  };
  const text = String(command ?? "");
  for (const st of scanShell(text)) {
    add({ t: st.redirects, blind: false });
    const w = st.words.slice();
    while (w.length && (/^(?:&|\.|call|command|sudo|time|exec|nohup)$/i.test(w[0]) || /^(?:\$\(|@\(|[({])+$/.test(w[0]) || /^[A-Za-z_]\w*=/.test(w[0]))) w.shift();
    if (!w.length) continue;
    const verb = baseOf(w[0]);
    const a = w.slice(1);
    if (Object.hasOwn(SHELL_VERBS, verb)) add(verbWrites(SHELL_VERBS[verb], a));
    else if (verb === "git") {
      const k = a.findIndex((x) => !x.startsWith("-"));
      if (["rm", "mv"].includes(a[k])) add({ t: a.slice(k + 1).filter((x) => !x.startsWith("-")), blind: false });
    } else if (verb === "sed" && a.some((x) => /^(?:-[a-z]*i|--in-place)/.test(x))) blind = true;
    else if (/^(?:node|nodejs)$/.test(verb)) {
      const k = a.findIndex((x) => /^(?:-e|--eval|-p|--print|-pe|-ep)$/.test(x));
      if (k >= 0 && a[k + 1] !== undefined) add(jsWrites(a[k + 1]));
    } else if (/^(?:python3?|py)$/.test(verb)) {
      const k = a.indexOf("-c");
      if (k >= 0 && a[k + 1] !== undefined) add(pyWrites(a[k + 1]));
    } else if (SHELLS.has(verb)) {
      const code = nestedScript(verb, a);
      if (code !== null) nest(code);
    } else if (verb === "iex" || verb === "invoke-expression" || verb === "icm" || verb === "invoke-command") {
      if (a.length === 1 && !/[$(]/.test(a[0])) nest(a[0]);
      else blind = true;
    }
  }
  for (const m of text.matchAll(IO_STATIC)) { // [System.IO.File]::WriteAllText('path', ...)
    if (!/^(?:Write|Append|Create|OpenWrite|Delete|Copy|Move|Replace)/i.test(m[1])) continue;
    const picks = /^copy/i.test(m[1]) ? [m[5]] : /^(?:move|replace)/i.test(m[1]) ? [m[3], m[5]] : [m[3]];
    if (picks.some((p) => p === undefined)) blind = true;
    add({ t: picks.filter((p) => p !== undefined), blind: false });
  }
  return { targets: [...new Set(targets)], blind };
};

const SEAT_NAME = { read: "judge", run: "runner", research: "research" };
const TARGET_ONLY = new Set(["read", "run", "research"]);
const BLIND_BLOCKED = new Set(["run", "research"]);
// null = the command's file writes are inside the seat's lane, else the one-line reason.
export const checkShellWrites = ({ command, cls, root, allowed }) => {
  if (!["code", "write", ...TARGET_ONLY].includes(cls)) return null;
  const { targets, blind } = shellWrites(command);
  for (const file of targets) {
    if (!TARGET_ONLY.has(cls)) { const why = checkEditPath({ file, cls, root, allowed }); if (why) return why; continue; }
    const rel = relPath(file, root);
    if (rel === null) { if (!isTemp(file)) return `out of repo: ${slash(file)}; seats write only inside ${slash(root)} or the temp folder`; continue; }
    if (rel.startsWith("target/sprint")) return `the handoff, lock and halt belong to the lead: ${rel}`;
    if (!rel.startsWith("target/")) return `${SEAT_NAME[cls]} seat: no shell writes outside target/ (${rel}); report the fix as a packet`;
  }
  if (blind && BLIND_BLOCKED.has(cls)) return `${SEAT_NAME[cls]} seat: this command writes to a path I cannot read; name a literal path under target/ or use the edit tool`;
  return null;
};

// A coder may undo its own edit: git restore --source=HEAD -- <paths inside its packet>. Null = fine.
const RESTORE_FLAGS = /^(?:--worktree|-W|-q|--quiet|--no-overlay|--ignore-skip-worktree-bits)$/;
const restoreWhy = (args, { cls, root, allowed }) => {
  const words = scanShell(args)[0]?.words ?? [];
  const sep = words.indexOf("--");
  const flags = sep < 0 ? words : words.slice(0, sep);
  const paths = sep < 0 ? [] : words.slice(sep + 1);
  let fromHead = false;
  const odd = flags.some((f, k) => {
    if (/^--source=HEAD$/i.test(f)) { fromHead = true; return false; }
    if (/^(?:--source|-s)$/.test(f)) { fromHead = /^HEAD$/i.test(flags[k + 1] ?? ""); return !fromHead; }
    return !(RESTORE_FLAGS.test(f) || (/^HEAD$/i.test(f) && /^(?:--source|-s)$/.test(flags[k - 1] ?? "")));
  });
  if (odd || !fromHead || !paths.length) return "coder seats may only run git restore --source=HEAD -- <your packet paths>; the lead decides the rest";
  for (const file of paths) { const why = checkEditPath({ file, cls, root, allowed }); if (why) return why; }
  return null;
};

export const checkBash = ({ command, cls, root, allowed }) => {
  // `-c rebase.autoStash=true` would vanish with the -c strip below.
  if (/\bgit\b[^;|&\n]*\b(?:rebase|merge)\.autoStash\b/i.test(String(command ?? ""))) return "blocked: autostash stashes every session's edits; pull --no-rebase --no-edit, never --autostash";
  // `git -C <dir> add` and `git -c k=v push` are the same verbs.
  const cmd = String(command ?? "").replace(/\bgit(?:\.exe)?((?:\s+-[Cc]\s+(?:"[^"]*"|'[^']*'|\S+))+)(?=\s)/g, "git");
  for (const [re, why] of DENY) if (re.test(cmd)) return `blocked: ${why}`;
  if (isCargo(cmd) && !CARGO_OK.has(cls) && !(cls === "run" && RUN_BUILD.test(cmd))) return cls === "run"
    ? "runner seats call cargo only to rebuild a binary: run the `rebuild:` line that `node tools/bins.mjs` prints (cargo build --release -p <pkg> --bin <name>), never test, run or another target dir"
    : "this seat never calls cargo: only coders, judges and the lead build; file a packet instead";
  if (cls === "lead") return null;
  // 2026-10-11: a coder's crate-wide format/fix rewrote 10 endpoint files outside its packet, under other coders' work.
  if (/\bcargo(?:\.exe)?\s+(?:\+\S+\s+)?fmt\b/.test(cmd) && !/\s--check\b/.test(cmd)) return "blocked: cargo fmt rewrites every file of the crate; format only your own files: rustfmt --edition 2021 <your packet paths> (cargo fmt only with --check)";
  if (/\bcargo(?:\.exe)?\s+(?:\+\S+\s+)?(?:clippy\b[^;|&\n]*\s--fix\b|fix\b)/.test(cmd)) return "blocked: cargo clippy --fix / cargo fix edits files across the crate; make the fix by hand in your packet paths";
  // Seats build one crate at a time on 3 shared cargo slots; these run the whole workspace or spawn cargo outside the slots.
  if (isCargo(cmd) && /\s--(?:workspace|all)(?=\s|$)/.test(cmd)) return "blocked: workspace-wide cargo; seats run one crate: cargo test -p <crate> --lib <filter>";
  if (/\btools[\\/]proof\.mjs\b/.test(cmd) && !/\s--skip-build\b/.test(cmd)) return "blocked: tools/proof.mjs builds and tests the whole workspace; golden: cargo run -p engine2040 --release (final_hash=d57b5c8ae3096614)";
  if (/\btools[\\/]mutants\.mjs\b/.test(cmd)) return "blocked: mutants.mjs runs cargo-mutants outside the cargo slots; the lead runs it";
  if (/\bgit\s+(?:add|commit)\b/.test(cmd)) {
    if (!COMMIT_OK.has(cls)) return "this seat never stages or commits: the lead lands your work after a judge; end with RESULT and the paths";
    for (const verb of ["add", "commit"]) {
      for (const p of gitPathspecs(cmd, verb)) {
        if (p === null) return "commit by path only: git commit -m <msg> -- <your claimed paths>";
        const why = checkEditPath({ file: p, cls, root, allowed });
        if (why) return why;
      }
    }
  }
  const restores = [...cmd.matchAll(/\bgit\s+restore\b([^;|&\n]*)/g)];
  if (/\bgit\s+(?:apply|am|cherry-pick|stash|revert|tag|branch\s+-D|checkout|switch)\b/.test(cmd) || (restores.length && cls !== "code")) {
    return "this seat never applies, stashes, restores or reverts: report it; the lead decides";
  }
  for (const m of restores) { const why = restoreWhy(m[1], { cls, root, allowed }); if (why) return why; }
  return checkShellWrites({ command: cmd, cls, root, allowed });
};

// ---- who refused ----
// Every refusal ends with a bracket naming the rule that refused it and, when a claim line is the cause, that line
// (2026-10-12: 487 refusals in target/loop-keeper.log; seats retried the same write blind). The rule is read from the
// reason's own opening words, so the plain reasons above stay the one place the wording lives.
const RULES = [
  [/^held by Claude coder/, "claude-claim"],
  [/^out of repo:/, "repo-only"],
  [/^the lead never edits Rust/, "lead-no-rust"],
  [/^research seat:/, "research-lane"],
  [/^the handoff, lock and halt/, "lead-owns-sprint-files"],
  [/^(?:judge|runner) seat:/, "target-only"],
  [/^loop machinery:/, "machinery"],
  [/^no Rust for this seat/, "no-rust"],
  [/^the board and FOCUS/, "board-lead-only"],
  [/^no packet paths:/, "no-packet-paths"],
  [/^out of packet paths:/, "packet-paths"],
  [/^blocked: /, "bash-deny-list"],
  [/^(?:this seat never calls cargo|runner seats call cargo)/, "cargo-lane"],
  [/^coder seats may only run git restore/, "coder-restore-only"],
  [/^(?:this seat never stages or commits|this seat never applies|commit by path only)/, "git-verbs"],
];
// Rules a claim line can decide: a path the seat's packet or claims do not list, a board file, a Claude coder's file.
const CLAIM_RULES = new Set(["claude-claim", "no-packet-paths", "packet-paths", "board-lead-only"]);
const clipLine = (s) => (s.length > 140 ? `${s.slice(0, 137)}...` : s);

export const refusalRule = (why) => RULES.find(([re]) => re.test(String(why ?? "")))?.[1] ?? "seat-guard";

// why = a reason from checkEditPath/checkBash/checkCall. seatId = the seat's packet id (claim lines are named for it).
export const explainRefusal = ({ why, claims, seatId = "", now = Date.now() }) => {
  if (!why) return why;
  const rule = refusalRule(why);
  const parts = [`rule: ${rule}`];
  const rel = CLAIM_RULES.has(rule) ? (String(why).match(/^[^:]+(?:\([^)]*\))?: (\S+?)(?=;| is not claimed| changes only|$)/) ?? [])[1] : null;
  if (rel) {
    // The seat's own claims, as allowedPaths reads them: its packet id, or id-<suffix>; a sibling run (-r2 for -r3) is another seat.
    const full = String(seatId).toLowerCase();
    const id = full.replace(/-r\d+$/, "");
    const run = (full.match(/-r(\d+)$/) ?? [])[1] ?? null;
    const sibling = (rest) => { const m = rest.match(/^-r(\d+)(?:-|$)/); return Boolean(m) && m[1] !== run; };
    const mine = (c) => Boolean(id) && (c.seat === full || (c.seat.startsWith(`${id}-`) && !sibling(c.seat.slice(id.length))));
    const fresh = new Set(claimLines(claims, now).map((c) => c.raw));
    const covering = claimLines(claims, 0).filter((c) => c.paths.some((p) => pathMatch(rel, p)));
    const lines = [];
    for (const c of covering) {
      if (rule === "claude-claim") { if (fresh.has(c.raw) && c.seat.startsWith("claude-")) lines.push(`claim line "${clipLine(c.raw)}"`); }
      else if (mine(c) && !fresh.has(c.raw)) lines.push(`your claim line "${clipLine(c.raw)}" is older than 3 h: append a fresh one`);
      else if (!mine(c) && fresh.has(c.raw)) lines.push(`claim line "${clipLine(c.raw)}" is another seat's (${c.seat}), not yours`);
    }
    if (lines.length) parts.push(...lines.slice(0, 2));
    else if (rule !== "claude-claim") parts.push("no claim line names it");
  }
  return `${why} [${parts.join("; ")}]`;
};

// ---- one call ----
// seat = { role, cls, lead, packet: { id, standing, fm } }. Returns null (go) or the one-line reason, with its rule.
export const checkCall = ({ tool, args, seat, root, claims, now = Date.now() }) => {
  const why = checkCallPlain({ tool, args, seat, root, claims, now });
  return why ? explainRefusal({ why, claims, seatId: String(seat?.packet?.id ?? ""), now }) : null;
};
const checkCallPlain = ({ tool, args, seat, root, claims, now }) => {
  if (!seat?.cls) return null;
  const allowed = seat.cls === "lead" ? [] : allowedPaths({ packet: seat.packet, claims, now });
  if (tool === "bash") return checkBash({ command: args?.command, cls: seat.cls, root, allowed });
  for (const file of editTargets(tool, args)) {
    const held = claudeHeld({ file, root, claims, now });
    if (held) return held;
    const why = checkEditPath({ file, cls: seat.cls, root, allowed, seatId: String(seat.packet?.id ?? "").toLowerCase(), own: parsePaths(seat.packet?.fm?.paths ?? seat.packet?.fm?.owned) });
    if (why) return why;
  }
  return null;
};

// A file a Claude Code coder claimed (`claude-<name> | <UTC> | <ID> <paths>` in claims.txt) is
// off limits to every loop seat until the claim is gone or stale. 2026-10-10: VS4 and RA14 claimed
// game/lib.rs and render/present.rs minutes after Claude coders took them for EG1 and EG2.
export const claudeHeld = ({ file, root, claims, now = Date.now() }) => {
  const rel = relPath(file, root);
  if (rel === null) return null;
  for (const c of claimLines(claims, now)) {
    if (!c.seat.startsWith("claude-")) continue;
    if (c.paths.some((p) => pathMatch(rel, p))) return `held by Claude coder ${c.seat}${c.row ? ` (row ${c.row})` : ""}: ${rel}; take another row or file`;
  }
  return null;
};

// ---- dispatch: coder seats and disjoint paths ----
// packet = { role, fm, id } about to start; running = [{ role, fm, id }]. null = go.
export const checkDispatch = ({ packet, running, knobs, claims, now = Date.now() }) => {
  // A packet naming a file a Claude coder holds would only be refused edit by edit: it waits in ready/.
  // 2026-10-11: cut packets on ai/depth.rs and render/twin_shadow.rs sat behind EG33 and EG9.
  const lines = claimLines(claims, now).filter((c) => c.seat.startsWith("claude-"));
  const own = parsePaths(packet?.fm?.paths ?? packet?.fm?.owned);
  for (const p of own) {
    const c = lines.find((l) => l.paths.some((h) => pathMatch(p, h) || pathMatch(h, p)));
    if (c) return `${p} is held by Claude coder ${c.seat}${c.row ? ` (row ${c.row})` : ""}; the packet waits in ready/ until that claim lands or goes stale`;
  }
  // smith.md denies the board, so a smith packet on FOCUS or a SPRINT shard can only fail (BLOCKERS smith-FOCUS-deny x2).
  const board = String(packet?.role ?? "").toLowerCase() === "smith" ? own.find((p) => /^docs\/(?:focus|sprint)/.test(p)) : null;
  if (board) return `a smith seat cannot edit ${board} (smith.md denies the board); change the packet's role to planner`;
  if (classOf(packet?.role, knobs) !== "code") return null;
  const coders = running.filter((j) => classOf(j.role, knobs) === "code");
  const max = knob(knobs, "coders", 1);
  if (coders.length >= max) return `coder seats full: ${coders.length} of ${max} running (${coders.map((j) => j.id).join(", ")}); send smiths or research seats instead (knob coders)`;
  const mine = allowedPaths({ packet, claims, now });
  for (const j of coders) {
    const theirs = allowedPaths({ packet: j, claims, now });
    const hit = mine.find((a) => theirs.some((b) => pathMatch(a, b) || pathMatch(b, a) || crateOf(a) && crateOf(a) === crateOf(b)));
    if (hit) return `paths overlap with running coder ${j.id}: ${hit}; give each coder its own crate`;
  }
  return null;
};
const crateOf = (p) => (p.match(/^crates\/([^/]+)\//) ?? [])[1] ?? null;

// ---- cargo slots: one file per slot under target/cargo-slots/, created with `wx` (atomic) ----
export const listSlots = (dir) => {
  let names = [];
  try { names = readdirSync(dir).filter((n) => /^slot-\d+\.json$/.test(n)); } catch { return []; }
  return names.map((n) => {
    try { return { file: join(dir, n), ...JSON.parse(readFileSync(join(dir, n), "utf8")) }; } catch {
      try { return { file: join(dir, n), at: statSync(join(dir, n)).mtimeMs }; } catch { return null; }
    }
  }).filter(Boolean);
};
// A slot's `pid` (tools that hold slots themselves, like quiet.mjs) is checked too: a killed tool never
// runs its release, and its slots once blocked every cargo for 75 min (2026-10-11 perf run).
export const pidAlive = (pid) => {
  try { process.kill(Number(pid), 0); return true; } catch (e) { return e?.code === "EPERM"; }
};
// Cargo and rustc processes on this PC now; -1 when unknown. A keeper-held slot (no pid) stands for one running cargo
// command, so with none running a slot past SLOT_ORPHAN_MS is an orphan: its after-hook went to a keeper body that
// hot-reloaded away (2026-10-11: all 3 slots held 25 min with no cargo running, every coder in cargo wait).
const SLOT_ORPHAN_MS = 3 * 60_000;
export const cargoProcs = () => {
  try {
    const out = process.platform === "win32"
      ? execFileSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8", windowsHide: true })
      : execFileSync("ps", ["-A", "-o", "comm="], { encoding: "utf8" });
    return out.split("\n").filter((l) => /^"?(?:cargo|rustc)(?:\.exe)?"?(?:,|\s*$)/i.test(l.trim())).length;
  } catch { return -1; }
};
// alive(slot) false frees it (its session ended without the after-hook). Returns { ok, slot } or { ok: false, busy }.
export const acquireSlot = ({ dir, max, id, info = {}, now = Date.now(), alive = () => true, pidLive = pidAlive, procs = cargoProcs }) => {
  mkdirSync(dir, { recursive: true });
  let running = null;
  const orphan = (s) => !s.pid && now - (Number(s.at) || 0) > SLOT_ORPHAN_MS && (running ??= procs()) === 0;
  for (const s of listSlots(dir)) {
    if (now - (Number(s.at) || 0) > CARGO_SLOT_MAX_MS || !alive(s) || (s.pid && !pidLive(s.pid)) || orphan(s)) rmSync(s.file, { force: true });
  }
  for (let i = 0; i < max; i++) {
    const file = join(dir, `slot-${i}.json`);
    try {
      writeFileSync(file, JSON.stringify({ id, at: now, ...info }), { flag: "wx" });
      return { ok: true, slot: file };
    } catch { /* taken */ }
  }
  return { ok: false, busy: listSlots(dir) };
};
export const releaseSlot = ({ dir, id }) => {
  for (const s of listSlots(dir)) if (s.id === id) rmSync(s.file, { force: true });
};
export const busyLine = (busy, now = Date.now()) => busy.map((s) => `${s.seat ?? "?"} \`${String(s.cmd ?? "").slice(0, 40)}\` ${Math.round((now - (Number(s.at) || now)) / 60_000)}m`).join("; ");
// pwsh: every cargo command builds with 10 jobs (Maxim 2026-10-16 "use full cpu"). The one shared target dir lets only one
// compile run at a time per profile, so 2 jobs left 10 of 12 cores idle (measured 1.2 cores busy, 15 cargo calls queued).
export const CARGO_JOBS = "10";
export const withCargoJobs = (cmd) => (isCargo(cmd) && !/CARGO_BUILD_JOBS/.test(cmd) ? `$env:CARGO_BUILD_JOBS='${CARGO_JOBS}'; ${cmd}` : cmd);
