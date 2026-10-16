// RA8/q1 checker: q1 top-level fighter PNG count plus RA8 manifest presence.
// Read-only scan of target/q1/*.png, writes nothing.
// Missing PNGs are reported findings, never a build.
import { readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
export const Q1_ROOT_FROM_TOOLS = join(dirname(fileURLToPath(import.meta.url)), "..", "target", "q1");
export const EXPECTED_RA8 = Object.freeze({ fighterPng: 3, ra8Min: 1 });
function listFiles(dir) {
  try {
    return readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}
function topPngFiles(dir) {
  return listFiles(dir).filter((d) => d.isFile() && d.name.toLowerCase().endsWith(".png")).map((d) => d.name).sort();
}
function topRa8Files(dir) {
  return listFiles(dir).filter((d) => d.isFile() && d.name.toLowerCase().includes("ra8")).map((d) => d.name).sort();
}
export function checkRa8(root = Q1_ROOT_FROM_TOOLS) {
  const pngFiles = topPngFiles(root);
  const pngCount = pngFiles.length;
  const ra8Files = topRa8Files(root);
  const ra8Count = ra8Files.length;
  const manifestPresent = ra8Count >= EXPECTED_RA8.ra8Min;
  const manifestStatus = manifestPresent ? "present" : "missing";
  const findings = [];
  if (pngCount !== EXPECTED_RA8.fighterPng) {
    findings.push("fighter png " + pngCount + " != " + EXPECTED_RA8.fighterPng + " (V1 hero + V2 arena-day + V3 fight)");
  }
  if (pngCount === 0) {
    findings.push("skipped: no fighter PNGs in " + root + " (today 0, need 3)");
  }
  if (!manifestPresent) {
    findings.push("skipped: RA8 manifest absent for 3 fighter captures (today " + ra8Count + " ra8 files)");
  }
  return { root, pngCount, pngFiles, expectedPng: EXPECTED_RA8.fighterPng, ra8Count, ra8Files, manifestPresent, manifestStatus, findings, expected: { ...EXPECTED_RA8 } };
}
if (import.meta.main) {
  const r = checkRa8();
  console.log("png_count: " + r.pngCount + " (expected " + r.expectedPng + ")");
  console.log("manifest: " + r.manifestStatus + " (ra8 files " + r.ra8Count + ", expected >=" + r.expected.ra8Min + ")");
  for (const f of r.findings) console.log(f);
}

