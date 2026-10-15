#!/usr/bin/env node
// cook-char.mjs - manifest checker for the CT37/CT23 character intake.
// Use: node tools/cook-char.mjs --help
//      node tools/cook-char.mjs --check [--manifest content/manifest.json]
// Manifest + script only: no bytes are fetched, cooked or written here.
// Large binaries stay out: real locomotion originals live in git-ignored
// content/local/ and ship only as cooked output; combat GLBs stay under
// content/fighters/. This tool only checks that content/manifest.json is
// honest: licences gated, cook digests deduped, counts add to the 80-clip
// Tier A set, budgets hold, and no unfetched bytes are claimed.
// Licence gate: git-tracked motion sources resolve to CC0-1.0 or CC-BY-4.0
// only; Geno/LAFAN1/Mixamo/QAL-v1.0/Unreal-only stay in the refused list.
// Named refusals go to stderr with exit 2: E_MANIFEST_ABSENT,
// E_MANIFEST_EMPTY, E_PARSE. Check findings print as FAIL lines with
// exit 1; a clean manifest prints one RESULT line with exit 0.
import { execFileSync, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const DEFAULT_MANIFEST = resolve(REPO, "content", "manifest.json");

export const COMBAT_TARGET = 80;
export const ACCEPTED_SPDX = new Set(["CC0-1.0", "CC-BY-4.0"]);
export const REFUSED_MARKS = ["geno", "lafan1", "mixamo", "qal", "unreal"];
export const KEYS_ALLOWED = new Set([432, 475, 819, 862]);
export const GLB_BYTES_MIN = 53984 - 4096;
export const GLB_BYTES_MAX = 71260 + 4096;
export const DIGEST_RE = /^[0-9a-f]{16}$/;
// glb_sha16 is a display truncation copied verbatim from the receipts
// (DEDUPE-MANIFEST.md pins full SHA-256; the short column varies in
// length), so the shape check stays loose. cook_digest is the engine
// gate value and stays exactly 16 hex chars.
export const GLB_SHA_RE = /^[0-9a-f]{15,18}$/;

const fail = (code, detail) => {
  process.stderr.write(`${code} ${detail}\n`);
  process.exit(2);
};

// Check one combat clip row; returns a list of FAIL strings (empty = clean).
export function checkClip(clip, seen) {
  const out = [];
  const at = clip?.short ?? "?";
  if (typeof clip?.short !== "string" || !clip.short) out.push(`${at}: missing short`);
  if (typeof clip?.source_action !== "string" || !clip.source_action) out.push(`${at}: missing source_action`);
  if (!Number.isInteger(clip?.glb_bytes) || clip.glb_bytes < GLB_BYTES_MIN || clip.glb_bytes > GLB_BYTES_MAX) {
    out.push(`${at}: glb_bytes ${clip?.glb_bytes} outside the canonical family envelope`);
  }
  if (!DIGEST_RE.test(clip?.cook_digest ?? "")) out.push(`${at}: cook_digest is not 16 hex chars`);
  else if (seen.has(clip.cook_digest)) out.push(`${at}: duplicate cook_digest ${clip.cook_digest}`);
  else seen.add(clip.cook_digest);
  if (!GLB_SHA_RE.test(clip?.glb_sha16 ?? "")) out.push(`${at}: glb_sha16 is not a short hex truncation`);
  if (!KEYS_ALLOWED.has(clip?.keys)) out.push(`${at}: keys ${clip?.keys} not in 432/475/819/862`);
  if (typeof clip?.loop !== "boolean") out.push(`${at}: loop is not a boolean`);
  if (typeof clip?.root_motion !== "boolean") out.push(`${at}: root_motion is not a boolean`);
  return out;
}

// Check the parsed manifest object; returns a list of FAIL strings.
export function checkManifest(manifest) {
  const errors = [];
  if (!manifest || typeof manifest !== "object") return ["manifest is not an object"];
  const loco = manifest.locomotion;
  if (!loco || typeof loco !== "object") errors.push("locomotion section missing");
  else {
    const rec = loco.record_100style;
    if (!rec || rec.spdx !== "CC-BY-4.0") errors.push("locomotion.record_100style must carry CC-BY-4.0");
    else if (!ACCEPTED_SPDX.has(rec.spdx)) errors.push(`locomotion spdx ${rec.spdx} refused`);
    if (rec && rec.fetched !== false) errors.push("locomotion.record_100style must not claim fetched bytes");
    if (loco.cmu && loco.cmu.fetched !== false) errors.push("locomotion.cmu must not claim fetched bytes");
    const bake = loco.synthetic_bake;
    if (!bake || !/^0x[0-9a-f]{16}$/.test(bake.db_digest ?? "")) errors.push("locomotion.synthetic_bake.db_digest malformed");
    if (bake && (bake.clips !== 8 || bake.frames !== 360 || bake.entries !== 360)) {
      errors.push(`synthetic bake counts drift: clips=${bake.clips} frames=${bake.frames} entries=${bake.entries} (want 8/360/360)`);
    }
    if (loco.first_16 && loco.first_16.fetched !== false) errors.push("locomotion.first_16 must not claim fetched bytes");
  }
  const combat = manifest.combat;
  if (!combat || typeof combat !== "object") errors.push("combat section missing");
  else {
    if (combat.target !== COMBAT_TARGET) errors.push(`combat.target ${combat.target} is not ${COMBAT_TARGET}`);
    const canon = combat.canonical_14 ?? [];
    const batchA = combat.batch_a_16 ?? [];
    if (canon.length !== 14) errors.push(`canonical_14 holds ${canon.length} rows (want 14)`);
    if (batchA.length !== 16) errors.push(`batch_a_16 holds ${batchA.length} rows (want 16)`);
    const seen = new Set();
    for (const clip of [...canon, ...batchA]) errors.push(...checkClip(clip, seen));
    const landed = canon.length + batchA.length;
    if (combat.landed !== landed) errors.push(`combat.landed ${combat.landed} != rows ${landed}`);
    const open = combat.remaining?.open_count;
    if (open !== COMBAT_TARGET - landed) errors.push(`remaining.open_count ${open} != ${COMBAT_TARGET} - ${landed}`);
  }
  const refused = (manifest.refused ?? []).join("\n").toLowerCase();
  for (const mark of REFUSED_MARKS) {
    if (!refused.includes(mark)) errors.push(`refused list must name ${mark}`);
  }
  return errors;
}

export function loadManifest(path) {
  let raw = null;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    fail("E_MANIFEST_ABSENT", `absent: ${path}`);
  }
  if (!raw.length) fail("E_MANIFEST_EMPTY", `empty: ${path}`);
  try {
    return JSON.parse(raw);
  } catch (e) {
    fail("E_PARSE", `${path} is not valid JSON (${e.message})`);
  }
}

export function check(argv) {
  const args = argv ?? [];
  const opt = (name) => {
    const i = args.findIndex((a) => a === name || a.startsWith(`${name}=`));
    if (i < 0) return "";
    return args[i].includes("=") ? args[i].slice(name.length + 1) : (args[i + 1] ?? "");
  };
  const manifestPath = opt("--manifest") || DEFAULT_MANIFEST;
  const manifest = loadManifest(manifestPath);
  const errors = checkManifest(manifest);
  for (const line of errors) process.stderr.write(`FAIL ${line}\n`);
  if (errors.length) {
    process.exitCode = 1;
    return { ok: false, errors };
  }
  const landed = manifest.combat.landed;
  process.stdout.write(
    `RESULT cook-char ok landed=${landed}/${COMBAT_TARGET} locomotion=planned combat=30+50 manifest=${manifestPath}\n`,
  );
  return { ok: true, errors: [] };
}

export const COOK_SCRIPT = resolve(REPO, "tools", "blender", "cook_char.py");
export const BLENDER_BIN = "blender"; export const BLENDER_HINT = "install Blender 4.x and put blender on PATH (https://www.blender.org/download/)";
export function buildCookArgs(s, i, o) { return ["-b", "-P", String(s), "--", String(i), String(o)]; }
export function formatCookLine(p) { const n = Number(p); return `cook: peak_ws_mb=${p === null || p === undefined || !Number.isFinite(n) ? "UNKNOWN" : Math.round(n)}`; }
// No Blender on PATH: hint plus exit 2. Peak MB for one pid; null when unsampled.
export function missingBlender({ stderr = process.stderr, exitFn = process.exit } = {}) {
  stderr.write(`E_BLENDER_MISSING ${BLENDER_HINT}\n`);
  exitFn(2);
  return { exit: 2 };
}
// Peak working set in MB for one pid; null when unsampled. Never throws.
export function sampleCookPeakMb(pid, execFn = execFileSync) {
  try {
    if (!Number.isFinite(Number(pid))) return null;
    if (process.platform === "win32") {
      const out = execFn("tasklist", ["/FO", "CSV", "/NH", "/FI", `PID eq ${pid}`], { encoding: "utf8" });
      let mKb = -1;
      for (const m of String(out).matchAll(/"([\d,]+)\s*K"/g)) {
        const kb = Number(m[1].replace(/,/g, ""));
        if (Number.isFinite(kb) && kb > mKb) mKb = kb;
      }
      return mKb < 0 ? null : Math.round(mKb / 1024);
    }
    const kb = Number(String(execFn("ps", ["-o", "rss=", "-p", String(pid)], { encoding: "utf8" })).trim());
    return Number.isFinite(kb) ? Math.round(kb / 1024) : null;
  } catch {
    return null;
  }
}
// One blender process per clip; heavy RAM stays out of the loop.
export async function runCook({ input, output, script = COOK_SCRIPT, blender = BLENDER_BIN, spawnFn = spawn, sampleFn = sampleCookPeakMb, stdout = process.stdout, stderr = process.stderr } = {}) {
  const args = buildCookArgs(script, input, output);
  let child;
  try {
    child = spawnFn(blender, args, { stdio: ["ignore", "inherit", "pipe"], shell: process.platform === "win32" });
  } catch {
    missingBlender({ stderr });
    return { exit: 2, peakWsMb: null, args };
  }
  if (!child || typeof child.on !== "function") {
    missingBlender({ stderr });
    return { exit: 2, peakWsMb: null, args };
  }
  let peak = null;
  const note = (v) => {
    const n = Number(v);
    if (Number.isFinite(n)) peak = peak === null ? Math.round(n) : Math.max(peak, Math.round(n));
  };
  try { note(sampleFn(child.pid)); } catch {}
  const timer = setInterval(() => { try { note(sampleFn(child.pid)); } catch {} }, 50);
  let errBuf = "";
  child.stderr?.on("data", (d) => { const s = String(d); errBuf += s; stderr.write(s); });
  const exit = await new Promise((resolve) => {
    child.on("error", (err) => {
      if (err?.code === "ENOENT") missingBlender({ stderr });
      else stderr.write(`E_BLENDER_SPAWN ${err?.message ?? String(err)}\n`);
      resolve(err?.code === "ENOENT" ? 2 : 1);
    });
    child.on("close", (code) => resolve(code ?? 0));
  });
  clearInterval(timer);
  try { note(sampleFn(child.pid)); } catch {}
  if (exit === 2 || (exit !== 0 && /not recognized|not found/i.test(errBuf))) {
    if (!/E_BLENDER_MISSING/.test(errBuf)) missingBlender({ stderr });
    return { exit: 2, peakWsMb: peak, args };
  }
  if (exit === 0) {
    stdout.write(`${formatCookLine(peak)}\n`);
    return { exit: 0, peakWsMb: peak, args };
  }
  stderr.write(`E_COOK_EXIT ${exit}\n`);
  return { exit, peakWsMb: peak, args };
}
const isMain = import.meta.main;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(
      "Usage: node tools/cook-char.mjs --check [--manifest content/manifest.json]\n      node tools/cook-char.mjs --cook <in> <out.glb> [--script <path>]\n\nChecks the CT37/CT23 intake manifest (licences, digest dedupe, counts, budgets, refused list).\nHeadless cook runs blender -b -P <script> -- <in> <out.glb> in one process per clip and prints cook: peak_ws_mb=.. .\n",
    );
  } else if (args.includes("--check")) {
    check(args);
  } else if (args.includes("--cook")) {
    const i = args.indexOf("--cook"), input = args[i + 1], output = args[i + 2];
    const j = args.findIndex((a) => a === "--script" || a.startsWith("--script="));
    const script = (j < 0 ? "" : args[j].includes("=") ? args[j].slice(9) : (args[j + 1] ?? "")) || COOK_SCRIPT;
    if (!input || !output || input.startsWith("--") || output.startsWith("--")) {
      process.stderr.write("E_USAGE need --cook <in> <out.glb> [--script <path>]\n");
      process.exit(2);
    } else runCook({ input, output, script }).then(({ exit }) => { process.exitCode = exit; });
  } else {
    process.stderr.write("E_USAGE need --check|--cook or --help\n");
    process.exit(2);
  }
}
