// A Mixamo pack folder to baked combat packs: one command per pack.
//
//   node tools/motion/mixamo.mjs --pack-dir C:\forge-media\mixamo\sword_and_shield \
//     [--out target/motion/mixamo] [--only slash,block] [--prefix ss] [--install]
//
// Each FBX goes: tools/motion/mixamo_npz.py (Blender: the clip in Kimodo's NPZ
// layout on the SOMA-77 rig, see that file for why the transfer is exact) ->
// tools/motion_bake (the same bake the Kimodo clips take) -> the receipt.
// --install copies each pack to content/fighters/packs/clips/<name>.am5p and its
// receipt beside it, and prints the PROVENANCE rows to paste.
// An FBX with no animation (the pack's character file) is skipped, not failed.
// Env: BLENDER (blender.exe), MOTION_BAKE_BIN (a built motion_bake, else cargo run),
// SOMA_OFFSETS (default C:\forge-media\kimodo\soma77_offsets.npy).
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseBakeLine, sha256File } from "./clip.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const INSTALL_DIR = path.join(ROOT, "content", "fighters", "packs", "clips");

// "sword and shield slash (2).fbx" -> "ss_slash_2" (prefix "ss").
export function clipName(file, prefix = "mx") {
  const stem = path.basename(file, path.extname(file)).toLowerCase();
  const m = /^(.*?)(?:\s*\((\d+)\))?$/.exec(stem);
  const words = m[1]
    .replace(/^sword and shield\s+/, "")
    .replace(/^great sword\s+/, "")
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return [prefix, words, m[2]].filter(Boolean).join("_");
}

export function parseArgs(argv) {
  const out = { packDir: null, out: path.join(ROOT, "target", "motion", "mixamo"), only: [], prefix: "mx", install: false, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      i += 1;
      return argv[i];
    };
    if (a === "--help" || a === "-h") out.help = true;
    else if (a === "--pack-dir") out.packDir = next();
    else if (a === "--out") out.out = next();
    else if (a === "--only") out.only = next().split(",").map((s) => s.trim()).filter(Boolean);
    else if (a === "--prefix") out.prefix = next();
    else if (a === "--install") out.install = true;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!out.help && !out.packDir) throw new Error("--pack-dir is required");
  return out;
}

export function wanted(file, only) {
  if (!only.length) return true;
  const stem = path.basename(file).toLowerCase();
  return only.some((word) => stem.includes(word.toLowerCase()));
}

export function provenanceRow({ name, source, ticks, slip }) {
  return `| \`${name}.am5p\` | Mixamo \`${source}\` | ${ticks} | ${slip[0].toFixed(6)} / ${slip[1].toFixed(6)} |`;
}

function blender() {
  const candidates = [process.env.BLENDER, "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe",
    "C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe"].filter(Boolean);
  const hit = candidates.find((c) => existsSync(c));
  if (!hit) throw new Error("no Blender found: set BLENDER to blender.exe");
  return hit;
}

function bake(npz, name, outDir) {
  const pack = path.join(outDir, `${name}.am5p`);
  const receipt = path.join(outDir, `${name}.receipt.json`);
  const args = ["bake", "--source", npz, "--name", name, "--out", pack, "--receipt", receipt];
  const r = process.env.MOTION_BAKE_BIN
    ? spawnSync(process.env.MOTION_BAKE_BIN, args, { encoding: "utf8" })
    : spawnSync("cargo", ["run", "--quiet", "--manifest-path", path.join(ROOT, "tools", "motion_bake", "Cargo.toml"), "--", ...args], { encoding: "utf8" });
  const line = parseBakeLine(r.stdout || "");
  if (r.status !== 0 || !line) return { ok: false, why: (r.stderr || r.stdout || "").trim().split("\n").slice(-2).join(" ") };
  return { ok: true, pack, receipt, line };
}

function main(argv) {
  let opts;
  try {
    opts = parseArgs(argv);
  } catch (err) {
    console.error(`mixamo: ${err.message}`);
    return 1;
  }
  if (opts.help) {
    console.log("usage: node tools/motion/mixamo.mjs --pack-dir <folder of Mixamo FBX> [--out <dir>] [--only a,b] [--prefix ss] [--install]");
    return 0;
  }
  const files = readdirSync(opts.packDir).filter((f) => f.toLowerCase().endsWith(".fbx")).filter((f) => wanted(f, opts.only)).sort();
  mkdirSync(opts.out, { recursive: true });
  const offsets = process.env.SOMA_OFFSETS || "C:\\forge-media\\kimodo\\soma77_offsets.npy";
  const exe = blender();
  const rows = [];
  let failed = 0;
  for (const file of files) {
    const name = clipName(file, opts.prefix);
    const npz = path.join(opts.out, `${name}.npz`);
    const conv = spawnSync(exe, ["-b", "--factory-startup", "--python-exit-code", "1", "-P",
      path.join(ROOT, "tools", "motion", "mixamo_npz.py"), "--", path.join(opts.packDir, file), npz, "--offsets", offsets], { encoding: "utf8" });
    if (conv.status !== 0) {
      const why = ((conv.stderr || "").match(/E_MIXAMO .*/) || [`exit ${conv.status}`])[0];
      if (/no animation/.test(why)) {
        console.log(`skip ${file}: ${why}`);
        continue;
      }
      console.log(`FAIL ${file}: ${why}`);
      failed += 1;
      continue;
    }
    const baked = bake(npz, name, opts.out);
    if (!baked.ok) {
      console.log(`FAIL ${file}: bake: ${baked.why}`);
      failed += 1;
      continue;
    }
    const { ticks, avgSlip: slip } = baked.line;
    console.log(`ok ${name} ticks=${ticks} slip=${slip[0].toFixed(3)}/${slip[1].toFixed(3)} <- ${file}`);
    rows.push({ name, source: file, ticks: Number(ticks), slip, sha256: sha256File(baked.pack) });
    if (opts.install) {
      mkdirSync(INSTALL_DIR, { recursive: true });
      copyFileSync(baked.pack, path.join(INSTALL_DIR, `${name}.am5p`));
    }
  }
  writeFileSync(path.join(opts.out, "index.json"), `${JSON.stringify({ packDir: opts.packDir, clips: rows }, null, 2)}\n`);
  if (opts.install && rows.length) {
    console.log("\nPROVENANCE rows (content/fighters/packs/PROVENANCE.md):");
    for (const row of rows) console.log(provenanceRow(row));
  }
  console.log(`\nmixamo: ${rows.length} baked, ${failed} failed, index ${path.join(opts.out, "index.json")}`);
  return failed ? 1 : 0;
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  process.exit(main(process.argv.slice(2)));
}
