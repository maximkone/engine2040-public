// Lane worktrees are RETIRED (owner 2026-10-10: one worktree). Agents edit their
// owned paths in the single tree on master; `create` refuses (exit 2). The other
// commands stay only to clean up a stray lane: .lanes/<name> on branch
// lane/<name>, built in a shared pool folder target/pool-<k> (--target-dir).
// Pool-contention discipline (SL22, ADV1 finding 4): two lanes racing one
// pool serialize on the build-dir lock with green builds — queue time, never
// corruption. Evidence: wave-32 ADV1 race, both lanes green with
// "Blocking waiting for file lock on build directory" in one log.
// target/ stays out of git (see .gitignore).
//   node tools/lanes.mjs create <name>            refuses: lanes retired
//   node tools/lanes.mjs list
//   node tools/lanes.mjs flags <name>             cargo flags for the lane
//   node tools/lanes.mjs diff <name>              target/lanes/<name>.patch vs base
//   node tools/lanes.mjs prune [--apply|--dry-run] [--min-age-h N] [--skip a,b]
//                                           merged+clean+old lanes only (dry-run by default);
//                                           --min-age-h N: commit and every file older than N h too
//   node tools/lanes.mjs remove <name> [--force]  refuses uncommitted work unless --force
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { availableParallelism } from "node:os";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.cwd();
const ADMIN = join(ROOT, "target", "lanes-worktrees.json");
const CPUS = availableParallelism();
// One pool per 4 logical cores, at most 4: more pools only multiply the
// dependency cache on disk without adding CPU.
const POOLS = Math.max(1, Math.min(4, Math.floor(CPUS / 4)));
const JOBS = Math.max(1, Math.floor(CPUS / POOLS));

const run = (cmd, args, cwd = ROOT) => {
  const r = spawnSync(cmd, args, { encoding: "utf8", cwd, maxBuffer: 64 * 1024 * 1024 });
  return { ok: !r.error && r.status === 0, out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
};

const loadAdmin = () => {
  // Missing file means no lanes yet: empty is correct. A present but
  // unreadable or unparsable file must FAIL LOUD, never silently act on an
  // empty list (that silently hid live lanes).
  if (!existsSync(ADMIN)) return { worktrees: {} };
  let raw;
  try {
    raw = readFileSync(ADMIN, "utf8");
  } catch (err) {
    console.error(`lanes admin unreadable at ${ADMIN}: ${err?.message ?? err}`);
    process.exit(1);
  }
  try {
    const admin = JSON.parse(raw);
    if (!admin || typeof admin !== "object" || !admin.worktrees || typeof admin.worktrees !== "object") {
      throw new Error("expected { worktrees: {...} }");
    }
    return admin;
  } catch (err) {
    console.error(`corrupt lanes admin at ${ADMIN}: ${err?.message ?? err} (refusing empty list; restore backup or delete the file)`);
    process.exit(1);
  }
};

const saveAdmin = (admin) => {
  mkdirSync(join(ROOT, "target"), { recursive: true });
  writeFileSync(ADMIN, `${JSON.stringify(admin, null, 2)}\n`);
};

// Pending-review guard (2026-10-03: lane-GC pruned live lanes, judges
// BLOCKED on absent .lanes/<id>): packet text still queued in
// target/queue/ready or target/queue/running that names a lane keeps it,
// however merged/clean/old. A fresh collected patch
// (target/lanes/<id>.patch under 24 h old) keeps it too: the review may be
// between packets while the diff is fresh.
const QUEUE_SUBS = ["ready", "running"];
const RECENT_PATCH_MS = 24 * 3600 * 1000;
const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const queueText = () => {
  const parts = [];
  for (const sub of QUEUE_SUBS) {
    let files = [];
    try {
      files = readdirSync(join(ROOT, "target", "queue", sub)).filter((f) => f.endsWith(".md"));
    } catch { continue; }
    for (const f of files) {
      try { parts.push(readFileSync(join(ROOT, "target", "queue", sub, f), "utf8")); }
      catch { /* a packet vanishing mid-scan is fine */ }
    }
  }
  return parts.join("\n");
};
// Live-claim guard (2026-10-03 nav-rot review: 23 "cannot find the path" and
// reads of .lanes/win6-top, .lanes/bx8 while a helper still built there): a
// running helper holds a line `<item ID> | <run> | <UTC> | <files>` in
// target/queue/claims.txt (the keeper deletes it when the run ends; a line
// older than 2 h is a run the keeper never saw end). A lane named by that
// item ID (`win6` holds `win6` and `win6-top`, never `win10`) is in use.
const CLAIM_MS = 2 * 3600 * 1000;
const liveClaim = (key, now) => {
  let text = "";
  try { text = readFileSync(join(ROOT, "target", "queue", "claims.txt"), "utf8"); } catch { return false; }
  const lane = key.toLowerCase();
  for (const line of text.split(/\r?\n/)) {
    const parts = line.split("|").map((s) => s.trim());
    let item = parts[0];
    let stamp = parts[2];
    if (parts.length < 2) {
      // Pipeless claims (`<id> <UTC> <files>`, still written 2026-10-08):
      // first token is the item, the UTC inside is its clock (millis and a
      // missing Z normalized: claims are always UTC).
      const m = line.match(/^\s*(\S+)\s+(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?)(?:\.\d+)?Z?\b/);
      if (!m) continue;
      item = m[1];
      stamp = `${m[2]}Z`;
    }
    if (!item) continue;
    const id = item.toLowerCase();
    if (lane !== id && !lane.startsWith(`${id}-`)) continue;
    const at = Date.parse(stamp ?? "");
    if (Number.isFinite(at) && now - at < CLAIM_MS) return true;
  }
  return false;
};
const laneHeld = (key, queued, now) => {
  if (liveClaim(key, now)) return "live-claim";
  const k = escRe(key);
  // `.lanes/<id>`, `lanes/<id>.patch`, `lane/<id>`, `Lane: <id>` — each
  // bounded so `cq1` never matches `cq10-nav`.
  const bound = "[^a-z0-9-]";
  const ref = new RegExp(`(\\.lanes/${k}${bound}|lanes/${k}\\.patch|lane/${k}${bound}|lane:\\s*${k}${bound})`, "i");
  if (ref.test(`\n${queued}\n`)) return "pending-review";
  let mtime = 0;
  try { mtime = statSync(join(ROOT, "target", "lanes", `${key}.patch`)).mtimeMs; }
  catch { return null; }
  if (now - mtime < RECENT_PATCH_MS) return "recent-patch";
  return null;
};

// One line, exit 2: `create` is retired (owner 2026-10-10: one worktree on master).
const RETIRED_MSG ="lanes retired 2026-10-10 (owner: one worktree): edit owned paths in C:\\engine2040; see .opencode/commands/sprint.md";

const usage = () => {
  console.error("Usage: node tools/lanes.mjs list | flags <name> | diff <name> | remove <name> [--force]");
  console.error("       node tools/lanes.mjs prune [--apply|--dry-run] [--min-age-h N] [--skip a,b]");
  process.exit(2);
};

// Old entries (outside the project, no pool) still list and remove.
export const flagsFor = (info) => {
  const manifest = relative(ROOT, join(info.dir, "Cargo.toml")).split("\\").join("/");
  const pool = `target/pool-${info.pool ?? 1}`;
  return `--manifest-path ${manifest} --target-dir ${pool} -j ${JOBS} --locked`;
};

// Cold-lane hint (speed-pack): a lane whose pool target has no incremental
// cache, or whose worktree is newer than the pool's last build, starts
// cold. `flags` keeps stdout to the single cargo-flags line (the engine-land
// step 5 substitution contract) and prints the opt-in sccache hint on stderr
// in pwsh `$env:` form, so the builder can run that one command under sccache
// by pasting it in front. Warm lanes print nothing extra: sccache needs
// CARGO_INCREMENTAL=0, which slows warm rebuilds, so it stays opt-in per
// run and never lands in .cargo/config.toml.
export const isWarmLane = (info) => {
  const incr = join(ROOT, `target/pool-${info.pool ?? 1}`, "debug", "incremental");
  let entries = null;
  try {
    entries = readdirSync(incr);
  } catch {
    return false;
  }
  if (entries.length === 0) return false;
  try {
    if (typeof info.createdAt === "number" && info.createdAt > statSync(incr).mtimeMs) return false;
  } catch { return false; }
  return true;
};

// Pwsh-only hint text (AGENTS.md): paste in front of the one command.
export const COLD_HINT = `$env:RUSTC_WRAPPER="sccache"; $env:CARGO_INCREMENTAL="0"`;
const printColdHint = (info) => {
  if (!isWarmLane(info)) console.error(`cold lane (opt-in for this command only): ${COLD_HINT}`);
};

function main() {
const [command, name, ...rest] = process.argv.slice(2);
if (command === '-h' || command === '--help') {
  console.log('Usage: node tools/lanes.mjs list | flags <name> | diff <name> | remove <name> [--force]');
  console.log('       node tools/lanes.mjs prune [--apply|--dry-run] [--min-age-h N] [--skip a,b]');
  process.exit(0);
}
if (!command) usage();
const flag = (key) => {
  const index = rest.indexOf(key);
  return index === -1 ? null : rest[index + 1] ?? null;
};
const lane = (admin) => {
  let info = name && admin.worktrees[name];
  if (!info && name) {
    // Adopt-on-read (2026-10-08: 140 of 143 prod lanes were git worktrees the
    // admin file never tracked, so diff/remove said "unknown lane"): a lane
    // git lists under .lanes still diffs and removes. Prune adopts it for
    // write; here it stays read-only.
    const lanesDir = resolve(ROOT, ".lanes").split("\\").join("/").toLowerCase();
    const listed = run("git", ["worktree", "list", "--porcelain"]);
    if (listed.ok) {
      for (const block of listed.out.split(/\r?\n\r?\n/)) {
        let dir = null;
        let branch = "";
        for (const line of block.split(/\r?\n/)) {
          if (line.startsWith("worktree ")) dir = line.slice(9).trim();
          else if (line.startsWith("branch ")) branch = line.slice(7).trim().replace(/^refs\/heads\//, "");
        }
        if (!dir) continue;
        const full = resolve(dir).split("\\").join("/");
        if (full.toLowerCase() !== `${lanesDir}/${name.toLowerCase()}`) continue;
        info = { dir: resolve(dir), branch };
      }
    }
  }
  if (!info) {
    console.error(`unknown lane ${name ?? "(missing)"}`);
    process.exit(1);
  }
  return info;
};

if (command === "list") {
  const admin = loadAdmin();
  for (const [key, info] of Object.entries(admin.worktrees)) {
    const dirty = existsSync(info.dir) && run("git", ["-C", info.dir, "status", "--porcelain"]).out !== "";
    console.log(`${key} ${info.dir} ${info.branch} pool-${info.pool ?? 1}${dirty ? " DIRTY" : ""}`);
  }
  console.log(`RESULT ${Object.keys(admin.worktrees).length} lane(s), ${POOLS} pool(s), -j ${JOBS}`);
} else if (command === "create") {
  console.error(RETIRED_MSG);
  process.exit(2);
} else if (command === "flags") {
  const info = lane(loadAdmin());
  console.log(flagsFor(info));
  printColdHint(info);
} else if (command === "diff") {
  const info = lane(loadAdmin());
  // Orphan lanes (adopted on read) carry no base: diff against their fork
  // point, else the worktree diff refuses like the lane never existed.
  let base = info.base;
  if (!base) {
    const mb = run("git", ["merge-base", "master", info.branch || "HEAD"]);
    base = mb.ok && mb.out ? mb.out.split("\n")[0].trim() : null;
  }
  if (!base) {
    console.error(`no base for lane ${name}: unknown branch ${info.branch || "(detached)"}`);
    process.exit(1);
  }
  // Intent-to-add makes new files visible to diff without staging content.
  run("git", ["-C", info.dir, "add", "--intent-to-add", "--all"]);
  const patch = run("git", ["-C", info.dir, "diff", "--binary", base]);
  const stat = run("git", ["-C", info.dir, "diff", "--stat", base]);
  if (!patch.ok) {
    console.error(`git diff failed: ${patch.out}`);
    process.exit(1);
  }
  mkdirSync(join(ROOT, "target", "lanes"), { recursive: true });
  const out = join(ROOT, "target", "lanes", `${name}.patch`);
  writeFileSync(out, patch.out ? `${patch.out}\n` : "");
  console.log(stat.out || "(no changes)");
  console.log(`PATCH ${relative(ROOT, out)} (apply with: git apply --3way ${relative(ROOT, out)})`);
} else if (command === "prune") {
  // Self-cleaning lanes (W2-04): drop a lane worktree + branch ONLY when all
  // three hold — branch fully merged into master, lane status --porcelain
  // empty, lane older than 2 h — AND no review is outstanding: a lane named
  // by any packet in target/queue/ready or target/queue/running, or with a
  // collected target/lanes/<id>.patch under 24 h old, is always kept.
  // Dry-run by default; --apply acts. Never -D, never --force for ordinary
  // dirt: anything unknown (merge list unreadable, status
  // failure, age unknown) keeps the lane. No sprint-loop hook exists, so the
  // reaper runs this (see center's report), not the wave itself.
  // Scope: every worktree under .lanes that git lists, not only the ones in
  // target/lanes-worktrees.json (2026-10-04: 38 of 92 were untracked there and
  // filled the disk). --min-age-h <n> is the strict disk-pressure mode used by
  // center's auto-clean.ps1: the lane's creation, its last commit AND every
  // file in its tree must be older than n hours (default mode: creation 2 h).
  // --skip a,b keeps named lanes (a live process names them).
  // Frozen lanes count as clean: intent-to-add-only dirt with a fresh (<7 d)
  // collected patch and no recently touched file prunes like a clean lane —
  // the patch, not the worktree, carries the work to its judge.
  // No lane name here, so flags arrive as `name`: read both.
  const pargs = [name, ...rest].filter((a) => a !== undefined);
  const apply = pargs.includes("--apply") && !pargs.includes("--dry-run"); // --dry-run always wins
  const valueOf = (key) => {
    const index = pargs.indexOf(key);
    return index === -1 ? null : pargs[index + 1] ?? "";
  };
  const minAgeRaw = valueOf("--min-age-h");
  const minAgeH = minAgeRaw === null ? null : Number(minAgeRaw);
  if (minAgeH !== null && !(Number.isFinite(minAgeH) && minAgeH > 0)) {
    console.error("--min-age-h needs a positive number of hours");
    process.exit(2);
  }
  const strict = minAgeH !== null;
  const skip = new Set((valueOf("--skip") ?? "").split(",").map((s) => s.trim()).filter(Boolean));
  const admin = loadAdmin();
  const AGE_MS = (strict ? minAgeH : 2) * 3600 * 1000;
  const now = Date.now();
  // True when any file under dir (not .git, not through links) changed after cutoff.
  const anyFileNewerThan = (dir, cutoff) => {
    let entries = [];
    try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return true; } // unreadable: assume busy
    for (const entry of entries) {
      if (entry.name === ".git") continue;
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (anyFileNewerThan(full, cutoff)) return true;
      } else {
        try { if (statSync(full).mtimeMs > cutoff) return true; } catch { return true; }
      }
    }
    return false;
  };
  // Lanes git knows under .lanes, keyed by folder name; admin info wins when it has the key.
  const lanesDir = resolve(ROOT, ".lanes").split("\\").join("/");
  const laneMap = new Map(Object.entries(admin.worktrees));
  const listed = run("git", ["worktree", "list", "--porcelain"]);
  if (listed.ok) {
    for (const block of listed.out.split(/\r?\n\r?\n/)) {
      let dir = null;
      let branch = "";
      for (const line of block.split(/\r?\n/)) {
        if (line.startsWith("worktree ")) dir = line.slice(9).trim();
        else if (line.startsWith("branch ")) branch = line.slice(7).trim().replace(/^refs\/heads\//, "");
      }
      if (!dir) continue;
      const full = resolve(dir).split("\\").join("/");
      if (!full.toLowerCase().startsWith(`${lanesDir.toLowerCase()}/`)) continue;
      const key = full.slice(lanesDir.length + 1);
      if (!laneMap.has(key)) laneMap.set(key, { dir: resolve(dir), branch });
    }
  }
  const mergedRun = run("git", ["branch", "--merged", "master", "--format=%(refname:short)"]);
  const merged = new Set(mergedRun.ok ? mergedRun.out.split("\n").map((s) => s.trim()).filter(Boolean) : []);
  // Branch HEAD commit time, ms; null when unknown (fail honest, keep lane).
  const commitTime = (branch) => {
    const r = run("git", ["log", "-1", "--format=%ct", branch]);
    const secs = Number(r.ok ? r.out.trim() : NaN);
    return Number.isFinite(secs) && secs > 0 ? secs * 1000 : null;
  };
  const queued = queueText();
  let pruned = 0;
  let kept = 0;
  for (const [key, info] of laneMap) {
    const reasons = [];
    if (skip.has(key)) reasons.push("live-process");
    const held = laneHeld(key, queued, now);
    if (held) reasons.push(held);
    if (!mergedRun.ok || !merged.has(info.branch)) reasons.push("unmerged");
    let dirty = true;
    let frozen = false;
    if (existsSync(info.dir)) {
      // Untrimmed stdout: run() trims, which eats the leading space of an
      // intent-to-add line (" A file") and blinds the frozen check below.
      const stRaw = spawnSync("git", ["-C", info.dir, "status", "--porcelain"],
        { encoding: "utf8", cwd: ROOT, maxBuffer: 64 * 1024 * 1024 });
      if (stRaw.error || stRaw.status !== 0) reasons.push("status-unknown");
      else {
        // Porcelain-shaped lines only: stderr chatter (CRLF warnings) is
        // concatenated here and must not read as dirt.
        const entries = `${stRaw.stdout ?? ""}${stRaw.stderr ?? ""}`.split("\n")
          .map((l) => l.replace(/\r$/, ""))
          .filter((l) => /^[\s?!MADRCU][\s?!MADRCUA] /.test(l));
        if (entries.length === 0) dirty = false;
        else {
        // Frozen work (2026-10-08: diff marks new files intent-to-add, which
        // reads as dirty forever, so collected lanes never prune): when every
        // dirty line is intent-to-add, a fresh collected patch preserves the
        // work, and no file was touched recently, the lane counts as clean.
        // A stale patch (>7 d, past the salvage window) never qualifies: the
        // work was collected but no judge will read it.
        const itaOnly = entries.every((l) => l[0] === " " && l[1] === "A");
        let patchMtime = 0;
        try { patchMtime = statSync(join(ROOT, "target", "lanes", `${key}.patch`)).mtimeMs; } catch { /* no patch */ }
        if (itaOnly && patchMtime > 0 && now - patchMtime < 7 * 86_400_000
          && !anyFileNewerThan(info.dir, now - AGE_MS)) {
          dirty = false;
          frozen = true;
        } else dirty = true;
        }
      }
    } else {
      dirty = false;
    }
    if (dirty) reasons.push("dirty");
    const birth = typeof info.createdAt === "number" ? info.createdAt : commitTime(info.branch);
    if (birth === null) reasons.push("age-unknown");
    else if (now - birth < AGE_MS) reasons.push("young");
    if (strict && !reasons.includes("age-unknown") && !reasons.includes("young")) {
      // Strict mode: last commit and every file must be old too, so a lane someone is
      // editing right now (files touched, nothing committed or merged yet) is never taken.
      const lastCommit = commitTime(info.branch);
      if (lastCommit === null) reasons.push("age-unknown");
      else if (now - lastCommit < AGE_MS) reasons.push("young-commit");
      if (existsSync(info.dir) && anyFileNewerThan(info.dir, now - AGE_MS)) reasons.push("young-files");
    }
    if (reasons.length > 0) {
      kept += 1;
      console.log(`KEEP ${key} (${reasons.join(", ")})`);
      continue;
    }
    if (!apply) {
      console.log(`PRUNE ${key} (dry-run: ${info.branch} merged, ${frozen ? "frozen" : "clean"}, >${strict ? `${minAgeH}h` : "2h"})`);
      continue;
    }
    if (existsSync(info.dir)) {
      // Frozen worktrees are dirty by design (the patch carries the work),
      // and plain remove refuses dirty trees: --force only here, never for
      // ordinary dirt, never -D for the branch below.
      const remove = run("git", ["worktree", "remove", ...(frozen ? ["--force"] : []), info.dir]);
      if (!remove.ok) {
        kept += 1;
        console.error(`KEEP ${key} (worktree remove refused: ${remove.out.split("\n")[0]})`);
        continue;
      }
    } else {
      run("git", ["worktree", "prune"]);
    }
    // Safe delete only: a race that unmerged the branch keeps it, never -D.
    const branch = run("git", ["branch", "-d", info.branch]);
    if (!branch.ok) {
      kept += 1;
      console.error(`KEEP ${key} (branch ${info.branch} kept: ${branch.out.split("\n")[0]})`);
      continue;
    }
    delete admin.worktrees[key];
    pruned += 1;
    console.log(`PRUNED ${key} (${info.branch} removed)`);
  }
  if (apply) saveAdmin(admin);
  console.log(`RESULT ${apply ? `pruned ${pruned}` : "dry-run, nothing removed"}, kept ${kept}`);
} else if (command === "remove") {
  const admin = loadAdmin();
  const info = lane(admin);
  const force = rest.includes("--force");
  // Pending-review guard (2026-10-03: the lead's `remove` + `create`
  // re-created the lane-GC BLOCKEDs on cq10-combat): a lane named by any
  // packet in target/queue/ready or target/queue/running, or with a
  // collected target/lanes/<id>.patch under 24 h old, is refused without
  // --force, like uncommitted work.
  if (!force) {
    const held = laneHeld(name, queueText(), Date.now());
    if (held) {
      console.error(`refusing: lane ${name} is in use (${held}): a live claim in target/queue/claims.txt or an outstanding review; collect target/lanes/${name}.patch and judge it first, or pass --force to discard it`);
      process.exit(1);
    }
  }
  if (existsSync(info.dir)) {
    const remove = run("git", ["worktree", "remove", ...(force ? ["--force"] : []), info.dir]);
    if (!remove.ok) {
      console.error(`git worktree remove refused: ${remove.out}`);
      console.error("the lane has uncommitted work: run diff first, or pass --force to discard it");
      process.exit(1);
    }
  } else {
    run("git", ["worktree", "prune"]);
  }
  // Safe delete keeps a branch that holds unmerged commits unless --force.
  const branch = run("git", ["branch", force ? "-D" : "-d", info.branch]);
  delete admin.worktrees[name];
  saveAdmin(admin);
  console.log(`REMOVED ${name}${branch.ok ? "" : ` (branch ${info.branch} kept: ${branch.out.split("\n")[0]})`}`);
} else {
  usage();
}
} // main

if (import.meta.main) {
  main();
}
