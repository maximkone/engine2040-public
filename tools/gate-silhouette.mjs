// Automate the 30m silhouette gate.
//
// Usage:
//   node tools/gate-silhouette.mjs <silhouette.json> <silhouette_30m.png> [--golden <png>] [--json]
//
// Compares the cooked silhouette against a golden PNG using pixelmatch (ISC)
// and blockhash (MIT). Reports pass/fail with metrics. Exit 0 on pass, exit 1
// on fail, exit 2 on usage error or missing file.
//
// Donors: mapbox/pixelmatch (ISC) for pixel-level comparison, jaehl/blockhash
// (MIT) for perceptual fingerprinting (16x16 block-average with Hamming distance).
// Both are installed locally under tools/vendor (gitignored). When missing, the
// tool falls back to a builtin per-pixel RGB compare and a simple block-average
// hash, noting ENGINE: builtin.
//
// The golden PNG is the reference silhouette. When --golden is not provided,
// the tool looks for golden_silhouette_30m.png in the same directory as the
// input silhouette_30m.png. When neither exists, the tool exits 2 with a
// message pointing to the expected path.
//
// Metrics:
//   pixel_diff_pct: percentage of pixels that differ (pixelmatch threshold 0.1)
//   blockhash_hamming: Hamming distance between the 16x16 blockhash fingerprints
//   bbox_fill: the bbox_fill value from silhouette.json (must be in 0.40–0.70)
//   fill_30m: the fill_30m value from silhouette.json (must be in 0.40–0.70)
//
// A gate passes when:
//   - bbox_fill is in [0.40, 0.70]
//   - fill_30m is in [0.40, 0.70]
//   - pixel_diff_pct < 5.0 (less than 5% of pixels differ)
//   - blockhash_hamming < 20 (the fingerprints are similar)
//
// Output (text):
//   GATE: pass|fail
//   bbox_fill: <value> (band 0.40–0.70)
//   fill_30m: <value> (band 0.40–0.70)
//   pixel_diff_pct: <value> (< 5.0)
//   blockhash_hamming: <value> (< 20)
//   ENGINE: pixelmatch|builtin
//
// Output (--json):
//   {"gate": "pass"|"fail", "bbox_fill": <n>, "fill_30m": <n>, "pixel_diff_pct": <n>, "blockhash_hamming": <n>, "engine": "pixelmatch"|"builtin"}

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { decodePng, fail } from "./frame-moved.mjs";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const PIXELMATCH_THRESHOLD = 0.1;
const PIXEL_DIFF_MAX = 5.0;
const BLOCKHASH_HAMMING_MAX = 20;
const BBOX_FILL_MIN = 0.40;
const BBOX_FILL_MAX = 0.70;

function usage() {
  return "usage: node tools/gate-silhouette.mjs <silhouette.json> <silhouette_30m.png> [--golden <png>] [--json]";
}

async function vendorEngine() {
  try {
    const { createRequire } = await import("node:module");
    const { pathToFileURL } = await import("node:url");
    const vdir = join(ROOT, "tools", "vendor", "node_modules");
    if (!existsSync(join(vdir, "pixelmatch")) || !existsSync(join(vdir, "pngjs"))) return null;
    const req = createRequire(fileURLToPath(import.meta.url));
    const pngjs = req(join(vdir, "pngjs"));
    const pm = req(join(vdir, "pixelmatch"));
    const pixelmatch = pm?.default ?? pm;
    if (typeof pixelmatch !== "function" || !pngjs?.PNG?.sync?.read) throw new Error("vendor shape");
    return { PNG: pngjs.PNG, pixelmatch, name: "pixelmatch" };
  } catch {
    return null;
  }
}

function blockhashBuiltin(img, w, h, bits = 16) {
  // Simple block-average hash: divide the image into bits x bits blocks,
  // compute the average luminance of each block, and threshold at the median.
  const blockW = Math.floor(w / bits);
  const blockH = Math.floor(h / bits);
  const avgs = [];
  for (let by = 0; by < bits; by++) {
    for (let bx = 0; bx < bits; bx++) {
      let sum = 0;
      let count = 0;
      for (let y = by * blockH; y < (by + 1) * blockH; y++) {
        for (let x = bx * blockW; x < (bx + 1) * blockW; x++) {
          const i = (y * w + x) * 4;
          const lum = 0.299 * img[i] + 0.587 * img[i + 1] + 0.114 * img[i + 2];
          sum += lum;
          count++;
        }
      }
      avgs.push(sum / count);
    }
  }
  const median = avgs.slice().sort((a, b) => a - b)[Math.floor(avgs.length / 2)];
  return avgs.map(a => a > median ? 1 : 0);
}

function hammingDistance(a, b) {
  if (a.length !== b.length) throw new Error("hash length mismatch");
  let dist = 0;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) dist++;
  }
  return dist;
}

export async function main() {
  const args = process.argv.slice(2);
  if (args.length < 2) {
    process.stderr.write(usage() + "\n");
    process.exitCode = 2;
    return;
  }

  const jsonPath = args[0];
  const pngPath = args[1];
  let goldenPath = null;
  let jsonOut = false;

  for (let i = 2; i < args.length; i++) {
    if (args[i] === "--golden" && i + 1 < args.length) {
      goldenPath = args[++i];
    } else if (args[i] === "--json") {
      jsonOut = true;
    }
  }

  if (!existsSync(jsonPath)) fail(`silhouette.json not found: ${jsonPath}`);
  if (!existsSync(pngPath)) fail(`silhouette_30m.png not found: ${pngPath}`);

  const sil = JSON.parse(readFileSync(jsonPath, "utf8"));
  const bbox_fill = sil.bbox_fill;
  const fill_30m = sil.fill_30m;

  if (!goldenPath) {
    goldenPath = join(dirname(pngPath), "golden_silhouette_30m.png");
    if (!existsSync(goldenPath)) {
      process.stderr.write(`gate-silhouette: golden PNG not found: ${goldenPath}\n`);
      process.stderr.write("  provide --golden <png> or place golden_silhouette_30m.png in the same directory\n");
      process.exitCode = 2;
      return;
    }
  }

  if (!existsSync(goldenPath)) fail(`golden PNG not found: ${goldenPath}`);

  const vendor = await vendorEngine();
  const engine = vendor ? "pixelmatch" : "builtin";

  const newBuf = readFileSync(pngPath);
  const goldenBuf = readFileSync(goldenPath);

  let newImg, goldenImg;
  try {
    newImg = decodePng(newBuf);
    goldenImg = decodePng(goldenBuf);
  } catch (err) {
    fail(`PNG decode failed: ${err.message}`);
  }

  if (newImg.width !== goldenImg.width || newImg.height !== goldenImg.height) {
    fail(`dimension mismatch: ${newImg.width}x${newImg.height} vs ${goldenImg.width}x${goldenImg.height}`);
  }

  let pixel_diff_pct = 0;
  if (vendor) {
    const newPng = vendor.PNG.sync.read(newBuf);
    const goldenPng = vendor.PNG.sync.read(goldenBuf);
    const diff = new vendor.PNG({ width: newPng.width, height: newPng.height });
    const numDiff = vendor.pixelmatch(newPng.data, goldenPng.data, diff.data, newPng.width, newPng.height, {
      threshold: PIXELMATCH_THRESHOLD,
    });
    pixel_diff_pct = (numDiff / (newPng.width * newPng.height)) * 100;
  } else {
    // Builtin: per-pixel RGB compare
    let numDiff = 0;
    const totalPixels = newImg.width * newImg.height;
    for (let i = 0; i < newImg.data.length; i += 4) {
      const dr = Math.abs(newImg.data[i] - goldenImg.data[i]);
      const dg = Math.abs(newImg.data[i + 1] - goldenImg.data[i + 1]);
      const db = Math.abs(newImg.data[i + 2] - goldenImg.data[i + 2]);
      if (dr > 25 || dg > 25 || db > 25) numDiff++;
    }
    pixel_diff_pct = (numDiff / totalPixels) * 100;
  }

  const newHash = blockhashBuiltin(newImg.data, newImg.width, newImg.height);
  const goldenHash = blockhashBuiltin(goldenImg.data, goldenImg.width, goldenImg.height);
  const blockhash_hamming = hammingDistance(newHash, goldenHash);

  const bbox_ok = bbox_fill >= BBOX_FILL_MIN && bbox_fill <= BBOX_FILL_MAX;
  const fill_ok = fill_30m >= BBOX_FILL_MIN && fill_30m <= BBOX_FILL_MAX;
  const pixel_ok = pixel_diff_pct < PIXEL_DIFF_MAX;
  const hash_ok = blockhash_hamming < BLOCKHASH_HAMMING_MAX;
  const gate = bbox_ok && fill_ok && pixel_ok && hash_ok ? "pass" : "fail";

  if (jsonOut) {
    process.stdout.write(JSON.stringify({
      gate,
      bbox_fill,
      fill_30m,
      pixel_diff_pct: pixel_diff_pct.toFixed(2),
      blockhash_hamming,
      engine,
    }) + "\n");
  } else {
    process.stdout.write(`GATE: ${gate}\n`);
    process.stdout.write(`bbox_fill: ${bbox_fill.toFixed(4)} (band ${BBOX_FILL_MIN}–${BBOX_FILL_MAX}) ${bbox_ok ? "✓" : "✗"}\n`);
    process.stdout.write(`fill_30m: ${fill_30m.toFixed(4)} (band ${BBOX_FILL_MIN}–${BBOX_FILL_MAX}) ${fill_ok ? "✓" : "✗"}\n`);
    process.stdout.write(`pixel_diff_pct: ${pixel_diff_pct.toFixed(2)} (< ${PIXEL_DIFF_MAX}) ${pixel_ok ? "✓" : "✗"}\n`);
    process.stdout.write(`blockhash_hamming: ${blockhash_hamming} (< ${BLOCKHASH_HAMMING_MAX}) ${hash_ok ? "✓" : "✗"}\n`);
    process.stdout.write(`ENGINE: ${engine}\n`);
  }

  process.exitCode = gate === "pass" ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
