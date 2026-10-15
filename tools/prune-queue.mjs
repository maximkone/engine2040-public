// prune-queue: keep done/ to newest 50 and park stale ready packets.
// Usage: node tools/prune-queue.mjs [--apply] [--keep <n>] [--age <min>]
// Default dry-run prints counts and moves nothing; --apply moves files.
// Exit 0 ok, 1 IO failure, 2 usage.
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const KEEP_DONE = 50;
export const STALE_MIN = 720;

/// File-list entry: { name, mtime } where mtime is ms since epoch.
export const mtimeOf = (e) => {
  const v = e?.mtimeMs ?? e?.mtime ?? 0;
  return v instanceof Date ? v.getTime() : Number(v) || 0;
};

const byName = (a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);

/// Oldest entries beyond the newest `keep` (newest-first keep, overflow oldest-first).
export function pickDoneOverflow(files, keep = KEEP_DONE) {
  const k = Math.max(0, Math.trunc(keep) || 0);
  const sorted = [...(files ?? [])].sort((a, b) => mtimeOf(b) - mtimeOf(a) || byName(a, b));
  return sorted.slice(k).sort((a, b) => mtimeOf(a) - mtimeOf(b) || byName(a, b));
}

/// Ready entries older than `ageMin` minutes with no running twin, oldest-first.
/// `running` is a list of twin names (array, Set, or entries); `now` is ms epoch.
export function pickStaleReady(files, ageMin = STALE_MIN, running = [], now = Date.now()) {
  let twins = running;
  let at = now;
  if (typeof twins === "number") {
    at = twins;
    twins = [];
  }
  const limit = Math.max(0, Number(ageMin) || 0) * 60000;
  const set = new Set();
  if (twins instanceof Set) {
    for (const n of twins) set.add(typeof n === "string" ? n : n?.name);
  } else {
    for (const n of twins ?? []) set.add(typeof n === "string" ? n : n?.name);
  }
  return [...(files ?? [])]
    .filter((f) => !set.has(f.name) && !f.running && at - mtimeOf(f) > limit)
    .sort((a, b) => mtimeOf(a) - mtimeOf(b) || byName(a, b));
}

const listMd = (dir) => {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((n) => n.endsWith(".md"))
    .map((n) => ({ name: n, mtime: statSync(join(dir, n)).mtimeMs }));
};

const runningTwins = () => {
  const dir = join("target", "queue", "running");
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .map((n) => (n.endsWith(".md.json") ? n.slice(0, -5) : n))
    .filter((n) => n.endsWith(".md"));
};

function main(argv) {
  let apply = false;
  let keep = KEEP_DONE;
  let age = STALE_MIN;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--apply") apply = true;
    else if (a === "--dry-run") apply = false;
    else if (a === "--keep") {
      keep = Math.trunc(Number(argv[++i]));
      if (!(keep >= 0)) {
        console.error("usage: node tools/prune-queue.mjs [--apply] [--keep <n>] [--age <min>]");
        process.exitCode = 2;
        return;
      }
    } else if (a === "--age") {
      age = Number(argv[++i]);
      if (!(age >= 0)) {
        console.error("usage: node tools/prune-queue.mjs [--apply] [--keep <n>] [--age <min>]");
        process.exitCode = 2;
        return;
      }
    } else if (a === "-h" || a === "--help") {
      console.log("usage: node tools/prune-queue.mjs [--apply] [--keep <n>] [--age <min>]");
      return;
    } else {
      console.error(`unknown flag ${a}: usage: node tools/prune-queue.mjs [--apply] [--keep <n>] [--age <min>]`);
      process.exitCode = 2;
      return;
    }
  }
  let done;
  let ready;
  try {
    done = listMd(join("target", "queue", "done"));
    ready = listMd(join("target", "queue", "ready"));
  } catch (err) {
    console.error(`prune-queue: cannot list queue: ${err?.message ?? err}`);
    process.exitCode = 1;
    return;
  }
  const overflow = pickDoneOverflow(done, keep);
  const stale = pickStaleReady(ready, age, runningTwins());
  console.log(`done ${done.length} keep ${keep} archive ${overflow.length}`);
  console.log(`ready ${ready.length} stale ${stale.length} parked ${apply ? stale.length : 0}`);
  if (!apply) {
    console.log("dry-run: moved nothing (use --apply to move)");
    return;
  }
  try {
    mkdirSync(join("target", "queue", "archive"), { recursive: true });
    mkdirSync(join("target", "queue", "parked"), { recursive: true });
    for (const f of overflow) renameSync(join("target", "queue", "done", f.name), join("target", "queue", "archive", f.name));
    const stamp = new Date().toISOString();
    for (const f of stale) {
      const from = join("target", "queue", "ready", f.name);
      const to = join("target", "queue", "parked", f.name);
      const ageMin = Math.max(0, Math.floor((Date.now() - mtimeOf(f)) / 60000));
      const body = readFileSync(from, "utf8");
      writeFileSync(to, `<!-- prune-queue: stale ready age ${ageMin}min over ${age}min, no running twin, parked ${stamp} -->\n${body}`);
      rmSync(from);
    }
    console.log(`moved archive ${overflow.length} parked ${stale.length}`);
  } catch (err) {
    console.error(`prune-queue: move failed: ${err?.message ?? err}`);
    process.exitCode = 1;
  }
}

if ((process.argv[1] ?? "").replace(/\\/g, "/").endsWith("tools/prune-queue.mjs")) main(process.argv.slice(2));
