// Q1 capture checker: pins today staged + orbit stills, then compares each
// still against its filed golden under `target/q1/golden`.
// Tier 1, bit-exact, lives here: on the software adapter the same draw order
// gives the same bytes, so a sha256 over the PNG is the gate. The perceptual
// tier — FLIP scored per still, pooled p95 and p99 held to a per-adapter bar —
// is the Rust bless flow's in crates/playtest/src/look/golden.rs, because node
// decodes no PNG here.
// A changed still writes `{still}.new.png` (the candidate), `{still}.diff.json`
// (the byte receipt: both sizes, both sha256, the first differing byte) and
// keeps `{still}.old.png` (the golden it is about to replace). Nothing is
// edited in place: only `--bless` promotes the candidate into `{still}.png`,
// and the `.old` copy survives that promotion.
// A still with no golden is a gap, unless `--bless` seeds one. A still listed
// in `golden/known-failures.json` that no longer differs is a stale row and a
// gap: the ratchet that keeps the table short.
import { readdirSync, readFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
export const Q1_ROOT_FROM_TOOLS = join(dirname(fileURLToPath(import.meta.url)), "..", "target", "q1");
export const SYNTH_REL = join("captures");
export const ORBIT_REL = join("2026-10-05", "orbit");
export const EXPECTED_Q1 = Object.freeze({ synthPng: 4, orbitPng: 30, totalPng: 34, manifests: 2, ra8Manifests: 0 });
export const GOLDEN_REL = "golden";
export const KNOWN_FAILURES_FILE = "known-failures.json";
export const NEW_SUFFIX = ".new.png";
export const DIFF_SUFFIX = ".diff.json";
export const OLD_SUFFIX = ".old.png";
function listFiles(dir) { try { return readdirSync(dir, { withFileTypes: true }); } catch (err) { console.warn("q1-check: cannot list " + dir + ": " + (err && err.code ? err.code : err)); return []; } }
function countPng(dir) { return listFiles(dir).filter((d) => d.isFile() && d.name.toLowerCase().endsWith(".png")).length; }
function manifestPath(dir) { const p = join(dir, "manifest.json"); return existsSync(p) ? p : null; }
function isRa8Manifest(p) { try { return readFileSync(p, "utf8").toLowerCase().includes("ra8"); } catch (err) { console.warn("q1-check: cannot read " + p + ": " + (err && err.code ? err.code : err)); return false; } }
export function checkQ1(root = Q1_ROOT_FROM_TOOLS) {
  const synthDir = join(root, SYNTH_REL);
  const orbitDir = join(root, ORBIT_REL);
  const synthPng = countPng(synthDir);
  const orbitPng = countPng(orbitDir);
  const manifests = [manifestPath(synthDir), manifestPath(orbitDir)].filter(Boolean);
  const ra8Manifests = manifests.filter(isRa8Manifest).length;
  const gaps = [];
  if (synthPng !== EXPECTED_Q1.synthPng) gaps.push("synth png " + synthPng + " != " + EXPECTED_Q1.synthPng);
  if (orbitPng !== EXPECTED_Q1.orbitPng) gaps.push("orbit png " + orbitPng + " != " + EXPECTED_Q1.orbitPng);
  if (manifests.length !== EXPECTED_Q1.manifests) gaps.push("manifests " + manifests.length + " != " + EXPECTED_Q1.manifests);
  if (ra8Manifests !== EXPECTED_Q1.ra8Manifests) gaps.push("ra8 " + ra8Manifests + " != " + EXPECTED_Q1.ra8Manifests);
  if (ra8Manifests === 0) gaps.push("skipped: RA8 reference-GPU look manifest absent");
  return { root, synthPng, orbitPng, totalPng: synthPng + orbitPng, manifests: manifests.length, manifestPaths: manifests, ra8Manifests, gaps, expected: { ...EXPECTED_Q1 } };
}

// The still names under one directory, sorted, bless artifacts skipped: a
// receipt records a change, it is never itself a still.
function pngNames(dir) {
  return listFiles(dir)
    .filter((d) => d.isFile() && d.name.toLowerCase().endsWith(".png"))
    .map((d) => d.name)
    .filter((name) => !name.endsWith(NEW_SUFFIX) && !name.endsWith(DIFF_SUFFIX) && !name.endsWith(OLD_SUFFIX))
    .sort();
}

export function sha256Hex(buf) { return createHash("sha256").update(buf).digest("hex"); }

// Every still the run owns: `{ still, capture }`, `still` relative to both the
// capture root and the golden root, so one name pairs the two trees.
export function stillsOf(root = Q1_ROOT_FROM_TOOLS) {
  const out = [];
  for (const rel of [SYNTH_REL, ORBIT_REL]) {
    for (const name of pngNames(join(root, rel))) out.push({ still: join(rel, name), capture: join(root, rel, name) });
  }
  return out;
}

// The known-failure table: still names a run forgives. Absent or unreadable
// file is an empty table; anything but a list of names is a gap the caller
// reports, never a silent ignore. Names compare with `/` separators, so a table
// filed on one host still reads on another.
export function knownFailures(root = Q1_ROOT_FROM_TOOLS) {
  const path = join(root, GOLDEN_REL, KNOWN_FAILURES_FILE);
  if (!existsSync(path)) return { names: [], gaps: [] };
  let parsed;
  try { parsed = JSON.parse(readFileSync(path, "utf8")); } catch (err) { return { names: [], gaps: ["known failures: " + path + " is not readable JSON: " + err.message] }; }
  if (!Array.isArray(parsed) || parsed.some((name) => typeof name !== "string")) return { names: [], gaps: ["known failures: " + path + " must hold a list of still names"] };
  return { names: parsed, gaps: [] };
}

function norm(p) { return String(p).split("\\").join("/"); }

// The byte receipt of one changed still: both sizes, both digests and the
// first offset where the two streams part.
function diffReceipt(still, capture, golden) {
  let firstDiff = -1;
  for (let i = 0; i < Math.min(capture.length, golden.length); i += 1) { if (capture[i] !== golden[i]) { firstDiff = i; break; } }
  return JSON.stringify({
    still,
    capture: { bytes: capture.length, sha256: sha256Hex(capture) },
    golden: { bytes: golden.length, sha256: sha256Hex(golden) },
    firstDiffByte: firstDiff,
    lengthsEqual: capture.length === golden.length,
  }, null, 2) + "\n";
}

// Compare every still against its filed golden, write the bless artifacts for
// the ones that changed and seed or promote only under `bless`.
export function blessQ1({ root = Q1_ROOT_FROM_TOOLS, bless = false } = {}) {
  const goldRoot = join(root, GOLDEN_REL);
  const table = knownFailures(root);
  const report = {
    root, goldens: goldRoot, bless, stills: 0, compared: 0, matched: 0,
    changed: [], missing: [], seeded: [], blessed: [], known: [], stale: [], wrote: [],
    gaps: [...table.gaps],
  };
  const knownSet = new Set(table.names.map(norm));
  const forgiven = new Set();
  for (const { still, capture } of stillsOf(root)) {
    report.stills += 1;
    const goldenPath = join(goldRoot, still);
    const stem = still.replace(/\.png$/i, "");
    if (!existsSync(goldenPath)) {
      if (!bless) {
        report.missing.push(still);
        report.gaps.push("missing golden: " + still);
        continue;
      }
      mkdirSync(dirname(goldenPath), { recursive: true });
      writeFileSync(goldenPath, readFileSync(capture));
      report.seeded.push(still);
      report.wrote.push(still);
      continue;
    }
    const live = readFileSync(capture);
    const filed = readFileSync(goldenPath);
    report.compared += 1;
    if (sha256Hex(live) === sha256Hex(filed)) {
      report.matched += 1;
      continue;
    }
    const receipt = [stem + NEW_SUFFIX, stem + OLD_SUFFIX, stem + DIFF_SUFFIX];
    writeFileSync(join(goldRoot, receipt[0]), live);
    writeFileSync(join(goldRoot, receipt[1]), filed);
    writeFileSync(join(goldRoot, receipt[2]), diffReceipt(still, live, filed));
    report.wrote.push(...receipt);
    if (knownSet.has(norm(still))) { report.known.push(still); forgiven.add(norm(still)); continue; }
    report.changed.push(still);
    if (bless) {
      writeFileSync(goldenPath, live);
      report.blessed.push(still);
    } else {
      report.gaps.push("golden differs: " + still);
    }
  }
  for (const name of table.names) {
    if (!forgiven.has(norm(name))) {
      report.stale.push(name);
      report.gaps.push("stale known failure: " + name);
    }
  }
  return report;
}

if (import.meta.main) {
  const bless = process.argv.includes("--bless");
  const counts = checkQ1();
  const golden = blessQ1({ bless });
  const gaps = [...counts.gaps, ...golden.gaps];
  console.log(JSON.stringify({ ...counts, golden, gaps }, null, 2));
  if (gaps.length > 0) process.exitCode = 1;
}
