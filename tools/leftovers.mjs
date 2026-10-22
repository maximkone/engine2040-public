// leftovers: list dirty paths in the shared tree that nobody has touched for
// a while, grouped so one free seat can adopt one group.
//
// Agents die mid-work when the model quota runs out; their uncommitted files
// stay in the tree. A path counts as left over when its newest write (the
// file, or the newest file under an untracked folder) is older than --hours.
// The tool never commits, deletes or reverts anything. --save writes each
// idle group's diff under target/leftovers/ so a fresh seat applies the work
// instead of re-reading it; a patch names the board row that mentions its
// paths, and `git apply` takes it on a clean tree.
//
// Usage from the repo root:
//   node tools/leftovers.mjs [--hours 3] [--json] [--save]
//   node tools/leftovers.mjs --help
//
// Exit codes: 0 ok (with or without leftovers) - 2 usage or git failed.
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const DEFAULT_HOURS = 3;
// Files under a folder walked for its newest write; deeper trees stop here.
const WALK_CAP = 2000;

/// Parse `git status --porcelain=v1 -z` output into `{ status, path }` rows.
/// Renames carry the new path (the old one follows as its own NUL field).
export function parseStatus(raw) {
  const fields = raw.split("\0");
  const rows = [];
  for (let i = 0; i < fields.length; i += 1) {
    const field = fields[i];
    if (field.length < 4) continue;
    const status = field.slice(0, 2);
    rows.push({ status: status.trim() || status, path: field.slice(3) });
    if (status.startsWith("R") || status.startsWith("C")) i += 1;
  }
  return rows;
}

/// The group a path belongs to: its crate (`crates/<c>`), its tool folder,
/// or its first two folders (`content/generated`), so one seat takes one.
export function groupOf(path) {
  const parts = path.replace(/\/$/, "").split("/");
  if (parts[0] === "crates" && parts.length > 1) return `crates/${parts[1]}`;
  if (parts.length <= 2) return parts.length === 1 ? "(root)" : parts[0];
  return `${parts[0]}/${parts[1]}`;
}

/// Newest write under `abs` in ms since the epoch: the file's own mtime, or
/// the newest file in a folder (walked up to WALK_CAP entries). Missing
/// paths (a deletion) give null.
export function newestWrite(abs) {
  let stat;
  try {
    stat = statSync(abs);
  } catch {
    return null;
  }
  if (!stat.isDirectory()) return stat.mtimeMs;
  let newest = stat.mtimeMs;
  let seen = 0;
  const stack = [abs];
  while (stack.length > 0 && seen < WALK_CAP) {
    const dir = stack.pop();
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      seen += 1;
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else {
        try {
          newest = Math.max(newest, statSync(full).mtimeMs);
        } catch {
          // A file gone mid-walk is not a write.
        }
      }
    }
  }
  return newest;
}

/// Leftover rows older than `hours`, grouped: `[{ group, idleHours, paths }]`,
/// oldest group first. `now` and `stat` are injectable for tests.
export function findLeftovers(rows, { hours, now = Date.now(), stat = (p) => newestWrite(join(ROOT, p)) }) {
  const groups = new Map();
  for (const { status, path } of rows) {
    const written = stat(path);
    if (written === null) continue;
    const idle = (now - written) / 3_600_000;
    if (idle < hours) continue;
    const group = groupOf(path);
    const entry = groups.get(group) ?? { group, idleHours: idle, paths: [] };
    entry.idleHours = Math.min(entry.idleHours, idle);
    entry.paths.push({ status, path, idleHours: Math.round(idle * 10) / 10 });
    groups.set(group, entry);
  }
  return [...groups.values()]
    .map((g) => ({ ...g, idleHours: Math.round(g.idleHours * 10) / 10 }))
    .sort((a, b) => b.idleHours - a.idleHours);
}

/// Parse board docs into `{ id, file, text }` rows. A row starts at
/// `- [ ] ID ` and runs to the next list item or heading: its continuation
/// lines hold Accept, Owns and Progress, which is where a path is named.
export function parseRows(docs) {
  const rows = [];
  for (const { file, body } of docs) {
    let cur = null;
    for (const line of body.split(/\r?\n/)) {
      const start = /^- \[[ xX]\] ([A-Za-z0-9]+) /.exec(line);
      if (start) {
        if (cur) rows.push(cur);
        cur = { id: start[1], file, text: line };
        continue;
      }
      if (cur === null) continue;
      if (line.startsWith("- ") || line.startsWith("#") || line.startsWith("|")) {
        rows.push(cur);
        cur = null;
        continue;
      }
      cur.text += "\n" + line;
    }
    if (cur) rows.push(cur);
  }
  return rows;
}

/// Paths named by a row block, at a path boundary, so `a.rs` never matches
/// `a.rs.bak`. Used to put the board row in a saved patch's header.
export function namesPath(text, path) {
  let at = text.indexOf(path);
  while (at !== -1) {
    const next = text[at + path.length] ?? "";
    if (!/[A-Za-z0-9_.\-/]/.test(next)) return true;
    at = text.indexOf(path, at + 1);
  }
  return false;
}

/// Rows that name any of `paths`, at most `limit`, so one patch header stays
/// one short line.
export function rowsTouching(rows, paths, limit = 3) {
  const found = [];
  for (const row of rows) {
    if (found.includes(row.id)) continue;
    if (paths.some((p) => namesPath(row.text, p.replace(/\/$/, "")))) found.push(row.id);
    if (found.length === limit) break;
  }
  return found;
}

function gitText(args, { cwd, maxBuffer = 256 << 20 }) {
  return execFileSync("git", args, { cwd, encoding: "utf8", maxBuffer });
}

/// `git diff --no-index` exits 1 when the files differ: that stdout is the
/// patch. A new file has no base, so `/dev/null` is its left side.
function untrackedDiff(path, cwd) {
  try {
    return gitText(["diff", "--no-index", "--binary", "--no-color", "--", "/dev/null", path], { cwd });
  } catch (err) {
    if (err.status === 1 && typeof err.stdout === "string") return err.stdout;
    throw err;
  }
}

/// One patch for a group: tracked hunks against HEAD plus a new-file hunk for
/// every untracked file under the group's untracked paths.
export function buildPatch(group, { root = ROOT } = {}) {
  const tracked = group.paths.filter((p) => p.status !== "??").map((p) => p.path);
  const untracked = group.paths.filter((p) => p.status === "??").map((p) => p.path);
  const parts = [];
  if (tracked.length > 0) {
    parts.push(gitText(["diff", "--binary", "--no-color", "HEAD", "--", ...tracked], { cwd: root }));
  }
  if (untracked.length > 0) {
    const listed = gitText(["ls-files", "--others", "--exclude-standard", "-z", "--", ...untracked], { cwd: root });
    for (const file of listed.split("\0").filter(Boolean)) parts.push(untrackedDiff(file, root));
  }
  return parts.filter((p) => p.length > 0).join("");
}

/// The header names what the patch is and which board rows mention its paths.
/// `git apply` reads past it, the same way it reads a mail patch's preamble.
export function patchHeader(group, ids) {
  const row = ids.length > 0 ? ids.join(",") : "no row names these paths";
  return `leftovers ${group.group} idle ${group.idleHours} h paths ${group.paths.length} row ${row}\n\n`;
}

/// Write one patch per non-empty group under `root/target/leftovers/`.
export function savePatches(groups, { root = ROOT, rowsOf = () => [] } = {}) {
  const dir = join(root, "target", "leftovers");
  mkdirSync(dir, { recursive: true });
  const written = [];
  for (const group of groups) {
    const body = buildPatch(group, { root });
    if (body.length === 0) continue;
    const ids = rowsOf(group);
    const file = join(dir, `${group.group.replace(/[\\/]/g, "__")}.patch`);
    writeFileSync(file, patchHeader(group, ids) + body);
    written.push({ group: group.group, file, rows: ids });
  }
  return written;
}

/// The board docs a patch header reads: every docs/SPRINT*.md shard.
export function readBoardDocs(root = ROOT) {
  const dir = join(root, "docs");
  return readdirSync(dir)
    .filter((name) => /^SPRINT.*\.md$/.test(name))
    .map((name) => ({ file: name, body: readFileSync(join(dir, name), "utf8") }));
}

function usage(code) {
  console.log("usage: node tools/leftovers.mjs [--hours N] [--json] [--save]");
  process.exit(code);
}

function main() {
  const args = process.argv.slice(2);
  let hours = DEFAULT_HOURS;
  let json = false;
  let save = false;
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === "--help") usage(0);
    else if (args[i] === "--json") json = true;
    else if (args[i] === "--save") save = true;
    else if (args[i] === "--hours") {
      hours = Number(args[i + 1]);
      i += 1;
      if (!Number.isFinite(hours) || hours < 0) usage(2);
    } else usage(2);
  }
  let raw;
  try {
    raw = execFileSync("git", ["status", "--porcelain=v1", "-z"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 << 20 });
  } catch (err) {
    console.error(`leftovers: git status failed: ${err.message}`);
    process.exit(2);
  }
  const groups = findLeftovers(parseStatus(raw), { hours });
  if (json) {
    console.log(JSON.stringify(groups, null, 2));
    return;
  }
  if (groups.length === 0) {
    console.log(`leftovers: none idle for ${hours} h`);
    return;
  }
  for (const g of groups) {
    console.log(`${g.group}  (${g.paths.length} paths, idle ${g.idleHours} h)`);
    for (const p of g.paths.slice(0, 8)) console.log(`  ${p.status.padEnd(2)} ${p.path}  ${p.idleHours} h`);
    if (g.paths.length > 8) console.log(`  ... ${g.paths.length - 8} more`);
  }
  if (save) {
    const rows = parseRows(readBoardDocs());
    const written = savePatches(groups, { rowsOf: (g) => rowsTouching(rows, g.paths.map((p) => p.path)) });
    for (const w of written) {
      console.log(`saved ${w.file}  (${w.rows.length > 0 ? `row ${w.rows.join(",")}` : "no row"})`);
    }
    if (written.length === 0) console.log("leftovers: nothing to save");
    return;
  }
  console.log("Adopt one group: finish, prove and commit it, or name what is missing. Never delete or revert it.");
}

const isMain = process.argv[1] !== undefined
  && resolve(process.argv[1]).toLowerCase() === fileURLToPath(import.meta.url).toLowerCase();
if (isMain) main();
