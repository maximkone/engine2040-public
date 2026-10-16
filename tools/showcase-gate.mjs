// showcase-gate.mjs — freshness gate for target/showcase sets.
// Fails loudly when the newest set is older than 5 waves or holds 0 fighters.
// 5 waves is 24 hours (same duty as drift-check section 9).
// Read-only on captures: never writes into target/, never runs cargo.
// Usage:
//   node tools/showcase-gate.mjs [--dir target/showcase] [--max-age-hours 24] [--fighters <n>] [--json]
// Exit: 0 pass, 1 gate FAIL (old or 0 fighters, or no set yet), 2 usage or dir error.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fail, usageFail } from "./argv.mjs";

// 5 waves of duty equals one day. Same limit as drift-check.
export const MAX_WAVES = 5;
export const MAX_AGE_HOURS = 24;

const HELP = `showcase-gate.mjs — newest showcase set must be fresh and hold fighters

Usage:
  node tools/showcase-gate.mjs [--dir target/showcase] [--max-age-hours 24] [--fighters <n>] [--json]

Checks:
  newest set age is at most --max-age-hours (default 24 h, which is 5 waves)
  newest set fighter count is above 0 (NOTES.md first, manifest.json fallback)

Fighters:
  NOTES.md with "N staged fighters" gives N.
  NOTES.md with "look_stage fighters" (and no "0" or "no" fighters line) gives 2.
  Otherwise the gate falls back to manifest.json fighters keys.
  No NOTES.md and no manifest fighters means 0 fighters and the gate FAILs.

Exit: 0 pass, 1 FAIL (old, 0 fighters, or no set), 2 usage.
Every run prints newest-set age hours and fighter count.
`;

// Fighters come from NOTES.md first. This matches the S27 board contract.
export function parseFightersFromNotes(text) {
  const t = String(text ?? "");
  const staged = t.match(/(\d+)\s+staged fighters/i);
  if (staged) return Math.max(0, Math.floor(Number(staged[1])));
  if (/look_stage fighters/i.test(t) && !/(no|0)[^.\n]*fighters/i.test(t)) return 2;
  return 0;
}

// Manifest fallback keys.
export function fightersFromManifest(manifest) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) return null;
  for (const key of ["fighters", "fighter_count", "fighters_count", "num_fighters"]) {
    const v = manifest[key];
    if (v !== undefined) {
      if (typeof v !== "number" || !Number.isFinite(v)) return null;
      return Math.max(0, Math.floor(v));
    }
  }
  return null;
}

// Newest set is the subdir with the latest mtime. Same rule as drift-check.
export function findNewestSet(dirAbs) {
  let entries;
  try {
    entries = readdirSync(dirAbs, { withFileTypes: true });
  } catch {
    return { error: `showcase dir not found: ${dirAbs}` };
  }
  const sets = [];
  for (const e of entries) {
    if (!e.isDirectory()) continue;
    const full = join(dirAbs, e.name);
    let st;
    try {
      st = statSync(full);
    } catch {
      continue;
    }
    if (!st.isDirectory()) continue;
    sets.push({ name: e.name, path: full, mtimeMs: st.mtimeMs });
  }
  sets.sort((a, b) => a.mtimeMs - b.mtimeMs);
  if (sets.length === 0) return { newest: null };
  return { newest: sets[sets.length - 1] };
}

// Count *.png files under a set dir. File names only, bytes are never read.
export function countPngs(setDir) {
  let count = 0;
  let entries;
  try {
    entries = readdirSync(setDir, { withFileTypes: true });
  } catch {
    return 0;
  }
  for (const e of entries) {
    const full = join(setDir, e.name);
    if (e.isDirectory()) count += countPngs(full);
    else if (e.isFile() && e.name.toLowerCase().endsWith(".png")) count += 1;
  }
  return count;
}

// Walk a set dir for manifest.json files. Keep the max fighter count found.
function maxManifestFighters(setDir) {
  let best = null;
  let found = false;
  let entries;
  try {
    entries = readdirSync(setDir, { withFileTypes: true });
  } catch {
    return { fighters: null, found: false };
  }
  for (const e of entries) {
    const full = join(setDir, e.name);
    if (e.isDirectory()) {
      const sub = maxManifestFighters(full);
      if (sub.found) {
        found = true;
        best = best === null ? sub.fighters : Math.max(best, sub.fighters);
      }
    } else if (e.isFile() && e.name.toLowerCase() === "manifest.json") {
      try {
        const raw = readFileSync(full, "utf8");
        const n = fightersFromManifest(JSON.parse(raw));
        if (n !== null) {
          found = true;
          best = best === null ? n : Math.max(best, n);
        }
      } catch {
        // A bad manifest is no fighters evidence. Skip it.
      }
    }
  }
  return { fighters: best, found };
}

// Fighter count for one set. NOTES.md wins, manifest.json is the fallback.
export function readSetFighters(setDir) {
  const notesPath = join(setDir, "NOTES.md");
  if (existsSync(notesPath)) {
    try {
      const text = readFileSync(notesPath, "utf8");
      return { fighters: parseFightersFromNotes(text), source: "NOTES.md" };
    } catch {
      return { fighters: 0, source: "NOTES.md unreadable" };
    }
  }
  const m = maxManifestFighters(setDir);
  if (m.found) return { fighters: m.fighters, source: "manifest.json" };
  return { fighters: 0, source: "none" };
}

// Pure check. Same inputs give the same result. Only reads, never writes.
export function checkShowcase(dirAbs, opts = {}) {
  const maxAgeHours = opts.maxAgeHours ?? MAX_AGE_HOURS;
  const found = findNewestSet(dirAbs);
  if (found.error) return { ok: false, error: found.error, newest: null };
  if (!found.newest) {
    return { ok: false, newest: null, ageHours: 0, fighters: 0, pngs: 0, source: "none", errors: [`no capture set yet in ${dirAbs}`] };
  }
  const newest = found.newest;
  const ageHours = (Date.now() - newest.mtimeMs) / 3_600_000;
  const read = opts.fighters !== undefined
    ? { fighters: opts.fighters, source: "--fighters" }
    : readSetFighters(newest.path);
  const pngs = countPngs(newest.path);
  const errors = [];
  if (ageHours > maxAgeHours) {
    errors.push(`newest set ${newest.name} is ${ageHours.toFixed(1)} h old (limit ${maxAgeHours} h = ${MAX_WAVES} waves)`);
  }
  if (read.fighters <= 0) {
    errors.push(`newest set ${newest.name} holds 0 fighters (source ${read.source}; want NOTES.md with staged fighters or a manifest fighters count)`);
  }
  return { ok: errors.length === 0, newest, ageHours, fighters: read.fighters, pngs, source: read.source, errors };
}

function parseArgs(argv) {
  const opts = { dir: "target/showcase", maxAgeHours: MAX_AGE_HOURS, fighters: undefined, json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) usageFail("showcase-gate", `flag ${a} needs a value`, HELP);
      i += 1;
      return v;
    };
    if (a === "--dir") opts.dir = next();
    else if (a === "--max-age-hours") {
      const v = Number(next());
      if (!Number.isFinite(v) || v <= 0) usageFail("showcase-gate", "--max-age-hours must be a number above 0", HELP);
      opts.maxAgeHours = v;
    } else if (a === "--fighters") {
      const v = Number(next());
      if (!Number.isInteger(v) || v < 0) usageFail("showcase-gate", "--fighters must be an integer at or above 0", HELP);
      opts.fighters = v;
    } else if (a === "--json") opts.json = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else usageFail("showcase-gate", `unknown flag ${a}`, HELP);
  }
  return opts;
}

function printResult(r, asJson) {
  if (asJson) {
    const out = r.newest
      ? { ok: r.ok, set: r.newest.name, ageHours: Number(r.ageHours.toFixed(2)), fighters: r.fighters, pngs: r.pngs, source: r.source, errors: r.errors ?? [] }
      : { ok: false, set: null, ageHours: 0, fighters: 0, pngs: 0, source: "none", errors: r.error ? [r.error] : (r.errors ?? ["no capture set"]) };
    process.stdout.write(`${JSON.stringify(out)}\n`);
    return;
  }
  if (!r.newest) {
    const msg = r.error ?? (r.errors ?? []).join("; ") ?? "no capture set";
    process.stderr.write(`showcase-gate FAIL: ${msg} | age 0.0 h fighters=0\n`);
    return;
  }
  const line = `newest set ${r.newest.name} age ${r.ageHours.toFixed(1)} h fighters=${r.fighters} pngs=${r.pngs}`;
  if (r.ok) process.stdout.write(`showcase-gate PASS: ${line}\n`);
  else process.stderr.write(`showcase-gate FAIL: ${line} — ${(r.errors ?? []).join("; ")}\n`);
}

if (import.meta.main) {
  const opts = parseArgs(process.argv.slice(2));
  const dirAbs = resolve(opts.dir);
  if (!existsSync(dirAbs)) {
    if (opts.json) process.stdout.write(`${JSON.stringify({ ok: false, set: null, ageHours: 0, fighters: 0, pngs: 0, source: "none", errors: [`showcase dir not found: ${dirAbs}`] })}\n`);
    else process.stderr.write(`showcase-gate FAIL: showcase dir not found: ${dirAbs} | age 0.0 h fighters=0\n`);
    fail("showcase-gate", `showcase dir not found: ${dirAbs}`, 2);
  }
  const r = checkShowcase(dirAbs, opts);
  printResult(r, opts.json);
  process.exit(r.ok ? 0 : 1);
}

