// Q1 capture checker: pins today staged + orbit stills.
// Read-only scan of target/q1 layout, writes nothing.
import { readdirSync, readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
export const Q1_ROOT_FROM_TOOLS = join(dirname(fileURLToPath(import.meta.url)), "..", "target", "q1");
export const SYNTH_REL = join("captures");
export const ORBIT_REL = join("2026-10-05", "orbit");
export const EXPECTED_Q1 = Object.freeze({ synthPng: 4, orbitPng: 30, totalPng: 34, manifests: 2, ra8Manifests: 0 });
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
if (import.meta.main) { const result = checkQ1(); console.log(JSON.stringify(result, null, 2)); if (result.gaps.length > 0) process.exitCode = 1; }
