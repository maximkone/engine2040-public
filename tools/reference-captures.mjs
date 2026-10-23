// reference-captures: verify one tier's real reference captures and merge
// them into content/reference/index.json.
//
// The native and potato tiers are shot by the arena binary (`arena
// --reference-set --reference-tier <tier>`), which writes ten PNGs plus a
// `fragment.json` beside them: the canvas, the seed, the camera, the adapter,
// the command line and every shot's tick. The web tier is shot by
// `tools/reference-web.mjs` from the real page. This tool is the verifier and
// the manifest writer: it reads each PNG back, checks its canvas against the
// row, decodes the pixels and files the FNV-1a digest over them - the same
// definition `crates/playtest/src/look/reference.rs` checks on load. Two
// passes over two captures must print the same pixel_fnv per shot.
//
// Usage from the repo root:
//   node tools/reference-captures.mjs                     verify every tier on disk
//   node tools/reference-captures.mjs --tier native       one tier
//   node tools/reference-captures.mjs --capture           run the native bin, then verify
//   node tools/reference-captures.mjs --capture --tier potato
//   node tools/reference-captures.mjs --capture --bin target/debug/arena.exe
//   node tools/reference-captures.mjs --help
//
// Exit codes: 0 verified and merged - 1 a capture or the manifest failed the
// check - 2 usage or the capture run failed.
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { inflateSync } from "node:zlib";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const INDEX_NAME = "index.json";
const FRAGMENT_NAME = "fragment.json";
/// Rows a complete tier holds. A tier is either absent or complete.
const ROWS_PER_TIER = 10;
/// The tier order the manifest files: the delivery legs.
const TIER_ORDER = ["native", "web", "potato"];
/// The canvas each delivery tier declares, pixels.
const TIER_CANVAS = { native: [1280, 720], web: [640, 360], potato: [320, 180] };
/// The one camera path every tier samples, in shot order.
const REFERENCE_TICKS = [0, 60, 120, 180, 240, 300, 360, 420, 480, 540];
/// The fixed reference seed.
const REFERENCE_SEED = 2040;
const FNV64_OFFSET = 0xcbf29ce484222325n;
const FNV64_PRIME = 0x100000001b3n;
const MASK64 = 0xffffffffffffffffn;

/// FNV-1a 64 over bytes, 16 lowercase hex chars (mirrors `engine_core::hash_bytes`).
export function fnvHex(bytes) {
  let hash = FNV64_OFFSET;
  for (const byte of bytes) hash = ((hash ^ BigInt(byte)) * FNV64_PRIME) & MASK64;
  return hash.toString(16).padStart(16, "0");
}

/// Decode an 8-bit RGBA non-interlaced PNG into `{ width, height, rgba }`.
/// The renderer's encoder writes exactly that shape; anything else refuses by
/// name instead of guessing.
export function decodePng(bytes) {
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  if (bytes.length < 33) throw new Error("not a PNG: too short");
  for (let at = 0; at < signature.length; at += 1) {
    if (bytes[at] !== signature[at]) throw new Error("not a PNG: bad signature");
  }
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  let interlace = 0;
  const parts = [];
  let pos = 8;
  while (pos + 12 <= bytes.length) {
    const len = bytes.readUInt32BE(pos);
    const type = bytes.toString("latin1", pos + 4, pos + 8);
    const data = bytes.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      interlace = data[12];
    } else if (type === "IDAT") {
      parts.push(data);
    } else if (type === "IEND") {
      break;
    }
    pos += 12 + len;
  }
  if (bitDepth !== 8 || colorType !== 6 || interlace !== 0) {
    throw new Error(
      `unsupported PNG shape: bit_depth=${bitDepth} color_type=${colorType} interlace=${interlace}`,
    );
  }
  if (width === 0 || height === 0) throw new Error("PNG holds no pixels");
  const raw = inflateSync(Buffer.concat(parts));
  const stride = width * 4;
  if (raw.length < (stride + 1) * height) throw new Error("PNG pixel stream is short");
  const rgba = Buffer.alloc(width * height * 4);
  let read = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = raw[read];
    read += 1;
    const line = raw.subarray(read, read + stride);
    read += stride;
    const out = rgba.subarray(y * stride, (y + 1) * stride);
    const prior = y === 0 ? null : rgba.subarray((y - 1) * stride, y * stride);
    for (let x = 0; x < stride; x += 1) {
      const left = x >= 4 ? out[x - 4] : 0;
      const up = prior ? prior[x] : 0;
      const upLeft = prior && x >= 4 ? prior[x - 4] : 0;
      let value = line[x];
      if (filter === 1) value += left;
      else if (filter === 2) value += up;
      else if (filter === 3) value += (left + up) >> 1;
      else if (filter === 4) value += paeth(left, up, upLeft);
      else if (filter !== 0) throw new Error(`bad PNG filter ${filter} on row ${y}`);
      out[x] = value & 0xff;
    }
  }
  return { width, height, rgba };
}

function paeth(left, up, upLeft) {
  const guess = left + up - upLeft;
  const toLeft = Math.abs(guess - left);
  const toUp = Math.abs(guess - up);
  const toUpLeft = Math.abs(guess - upLeft);
  if (toLeft <= toUp && toLeft <= toUpLeft) return left;
  return toUp <= toUpLeft ? up : upLeft;
}

/// Parse the tool's flags.
export function parseArgs(argv) {
  const args = { tiers: [], dir: "content/reference", capture: false, bin: null, help: false };
  for (let at = 0; at < argv.length; at += 1) {
    const flag = argv[at];
    if (flag === "--help" || flag === "-h") args.help = true;
    else if (flag === "--capture") args.capture = true;
    else if (flag === "--tier") {
      const value = argv[at + 1];
      if (!value || value.startsWith("--")) throw new Error("--tier needs a name");
      args.tiers.push(value);
      at += 1;
    } else if (flag === "--dir") {
      const value = argv[at + 1];
      if (!value || value.startsWith("--")) throw new Error("--dir needs a path");
      args.dir = value;
      at += 1;
    } else if (flag === "--bin") {
      const value = argv[at + 1];
      if (!value || value.startsWith("--")) throw new Error("--bin needs a path");
      args.bin = value;
      at += 1;
    } else {
      throw new Error(`unknown flag ${flag}`);
    }
  }
  return args;
}

/// Read one tier's fragment and verify every shot against its PNG.
/// Returns `{ fragment, rows }` where each row is the manifest row this tier
/// files.
export function verifyTier(setDir, tier) {
  const tierDir = join(setDir, tier);
  const fragmentPath = join(tierDir, FRAGMENT_NAME);
  if (!existsSync(fragmentPath)) throw new Error(`${tier}: no ${FRAGMENT_NAME} at ${fragmentPath}`);
  const fragment = JSON.parse(readFileSync(fragmentPath, "utf8"));
  if (fragment.tier !== tier) throw new Error(`${tier}: fragment names tier ${fragment.tier}`);
  if (fragment.schema !== 1) throw new Error(`${tier}: fragment schema ${fragment.schema}`);
  const canvas = TIER_CANVAS[tier];
  if (!canvas) throw new Error(`${tier}: unknown tier`);
  if (fragment.width !== canvas[0] || fragment.height !== canvas[1]) {
    throw new Error(
      `${tier}: fragment canvas is ${fragment.width}x${fragment.height}, ` +
        `the tier declares ${canvas[0]}x${canvas[1]}`,
    );
  }
  if (fragment.seed !== REFERENCE_SEED) {
    throw new Error(`${tier}: fragment seed ${fragment.seed}, the set declares ${REFERENCE_SEED}`);
  }
  for (const field of ["camera", "adapter", "driver", "command"]) {
    if (typeof fragment[field] !== "string" || fragment[field].length === 0) {
      throw new Error(`${tier}: fragment names no ${field}`);
    }
  }
  if (!Array.isArray(fragment.shots) || fragment.shots.length !== ROWS_PER_TIER) {
    throw new Error(
      `${tier}: fragment holds ${fragment.shots?.length ?? 0} shots, a tier files ${ROWS_PER_TIER}`,
    );
  }
  const source =
    typeof fragment.source === "string" && fragment.source.trim().length > 0
      ? fragment.source.trim()
      : `arena (engine-game bin) adapter=${fragment.adapter.trim()} ` +
        `driver=${fragment.driver.trim()} cmd=${fragment.command.trim()}`;
  const rows = [];
  const ticks = new Set();
  for (const shot of fragment.shots) {
    const path = join(setDir, shot.png);
    if (!existsSync(path)) throw new Error(`${tier}/${shot.shot}: ${shot.png} is not on disk`);
    const bytes = readFileSync(path);
    const decoded = decodePng(bytes);
    if (decoded.width !== shot.width || decoded.height !== shot.height) {
      throw new Error(
        `${tier}/${shot.shot}: PNG is ${decoded.width}x${decoded.height}, ` +
          `the fragment declares ${shot.width}x${shot.height}`,
      );
    }
    if (decoded.width !== fragment.width || decoded.height !== fragment.height) {
      throw new Error(
        `${tier}/${shot.shot}: PNG is ${decoded.width}x${decoded.height}, ` +
          `the tier canvas is ${fragment.width}x${fragment.height}`,
      );
    }
    const byteFnv = fnvHex(bytes);
    if (byteFnv !== shot.png_fnv) {
      throw new Error(
        `${tier}/${shot.shot}: PNG bytes hash ${byteFnv}, the fragment filed ${shot.png_fnv}`,
      );
    }
    if (bytes.length !== shot.bytes) {
      throw new Error(
        `${tier}/${shot.shot}: ${bytes.length} bytes, the fragment filed ${shot.bytes}`,
      );
    }
    if (ticks.has(shot.tick)) throw new Error(`${tier}/${shot.shot}: tick ${shot.tick} repeats`);
    ticks.add(shot.tick);
    const pixelFnv = fnvHex(decoded.rgba);
    if (pixelFnv === "0000000000000000") throw new Error(`${tier}/${shot.shot}: zero digest`);
    rows.push({
      tier,
      shot: shot.shot,
      png: shot.png,
      pixel_fnv: pixelFnv,
      width: shot.width,
      height: shot.height,
      seed: fragment.seed,
      camera: fragment.camera,
      tick: shot.tick,
      source,
      bytes: bytes.length,
    });
  }
  const ordered = [...ticks].sort((left, right) => left - right);
  if (
    ordered.length !== REFERENCE_TICKS.length ||
    ordered.some((tick, at) => tick !== REFERENCE_TICKS[at])
  ) {
    throw new Error(
      `${tier}: shot ticks are ${ordered.join(",")}, the camera path samples ` +
        REFERENCE_TICKS.join(","),
    );
  }
  rows.sort((left, right) => left.tick - right.tick);
  return { fragment, rows };
}

/// Row key order the manifest files, so a rewrite is byte-stable.
function rowJson(row) {
  return {
    tier: row.tier,
    shot: row.shot,
    png: row.png,
    pixel_fnv: row.pixel_fnv,
    width: row.width,
    height: row.height,
    seed: row.seed,
    camera: row.camera,
    tick: row.tick,
    source: row.source,
  };
}

function tierRank(tier) {
  const at = TIER_ORDER.indexOf(tier);
  return at === -1 ? TIER_ORDER.length : at;
}

/// Merge this run's tiers into the index text, preserving every other tier's
/// rows and the recipe block. Rows a legacy index still files under `rows`
/// with a recipe source move to `recipe` the first time (the second source
/// keeps its three painted rows).
export function mergeIndex(existingText, verified) {
  const existing = existingText === null ? null : JSON.parse(existingText);
  const index = existing ?? { schema: 1, scene: "arena-duel", seed: 2040 };
  const oldRows = Array.isArray(index.rows) ? index.rows : [];
  const ran = new Set(verified.keys());
  let recipe = Array.isArray(index.recipe) ? index.recipe.slice() : [];
  const kept = [];
  for (const row of oldRows) {
    const isRecipe = typeof row.source === "string" && row.source.startsWith("recipe ");
    if (isRecipe) {
      if (!recipe.some((filed) => filed.tier === row.tier && filed.shot === row.shot)) {
        recipe.push(row);
      }
      continue;
    }
    if (!ran.has(row.tier)) kept.push(row);
  }
  for (const rows of verified.values()) kept.push(...rows.map(rowJson));
  kept.sort((left, right) => tierRank(left.tier) - tierRank(right.tier) || left.tick - right.tick);
  const out = {
    schema: index.schema ?? 1,
    scene: index.scene ?? "arena-duel",
    camera: index.camera ?? verified.values().next().value?.[0]?.camera ?? "arena600",
    seed: index.seed ?? verified.values().next().value?.[0]?.seed ?? 2040,
    rows: kept,
  };
  if (recipe.length > 0) out.recipe = recipe;
  return `${JSON.stringify(out, null, 2)}\n`;
}

/// Every tier on disk that holds a fragment, in manifest order.
function tiersOnDisk(setDir) {
  const found = [];
  for (const tier of TIER_ORDER) {
    if (existsSync(join(setDir, tier, FRAGMENT_NAME))) found.push(tier);
  }
  return found;
}

function usage() {
  return [
    "node tools/reference-captures.mjs [--tier <tier>]... [--dir <set dir>]",
    "     [--capture] [--bin <arena binary>]",
    "",
    "Verifies each tier's PNGs against its fragment.json, prints one line per",
    "shot (tier/shot/pixel_fnv/bytes) and merges content/reference/index.json.",
    "--capture first runs `arena --reference-set --reference-tier <tier>`",
    "--reference-dir <set dir>; native (the default) and potato only. The web",
    "tier comes from tools/reference-web.mjs.",
  ].join("\n");
}

function main(argv) {
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    console.error(`reference-captures: ${error.message}`);
    console.error(usage());
    return 2;
  }
  if (args.help) {
    console.log(usage());
    return 0;
  }
  const setDir = resolve(ROOT, args.dir);
  if (args.capture) {
    const bin = resolve(ROOT, args.bin ?? join("target", "debug", "arena.exe"));
    if (!existsSync(bin)) {
      console.error(`reference-captures: no arena binary at ${bin}; build it or pass --bin`);
      return 2;
    }
    if (args.tiers.length > 1) {
      console.error("reference-captures: --capture runs one tier per command");
      return 2;
    }
    const captureTier = args.tiers.length === 0 ? "native" : args.tiers[0];
    if (captureTier === "web") {
      console.error("reference-captures: the web tier is shot by tools/reference-web.mjs");
      return 2;
    }
    if (!(captureTier in TIER_CANVAS)) {
      console.error(`reference-captures: unknown tier ${captureTier}`);
      return 2;
    }
    const command = [
      bin,
      "--reference-set",
      "--reference-tier",
      captureTier,
      "--reference-dir",
      args.dir,
    ];
    console.log(`reference-captures: run ${command.join(" ")}`);
    const run = spawnSync(command[0], command.slice(1), { cwd: ROOT, encoding: "utf8" });
    for (const line of `${run.stdout ?? ""}`.split(/\r?\n/)) {
      if (line.startsWith("arena reference:")) console.log(`  ${line}`);
    }
    if (run.status !== 0) {
      console.error(`reference-captures: the capture run exited ${run.status}`);
      const stderr = `${run.stderr ?? ""}`.trim();
      if (stderr.length > 0) console.error(stderr);
      return 2;
    }
  }
  const tiers = args.tiers.length > 0 ? args.tiers : tiersOnDisk(setDir);
  if (tiers.length === 0) {
    console.error(`reference-captures: no tier holds a ${FRAGMENT_NAME} under ${setDir}`);
    return 2;
  }
  const verified = new Map();
  for (const tier of tiers) {
    let checked;
    try {
      checked = verifyTier(setDir, tier);
    } catch (error) {
      console.error(`reference-captures: ${error.message}`);
      return 1;
    }
    verified.set(tier, checked.rows);
    for (const row of checked.rows) {
      console.log(
        `reference ${row.tier}/${row.shot} pixel_fnv=${row.pixel_fnv} ` +
          `bytes=${row.bytes} tick=${row.tick}`,
      );
    }
  }
  const indexPath = join(setDir, INDEX_NAME);
  const existing = existsSync(indexPath) ? readFileSync(indexPath, "utf8") : null;
  const merged = mergeIndex(existing, verified);
  const parsed = JSON.parse(merged);
  for (const tier of TIER_ORDER) {
    const rows = parsed.rows.filter((row) => row.tier === tier);
    if (rows.length === 0) continue;
    if (rows.length !== ROWS_PER_TIER) {
      console.error(
        `reference-captures: ${tier} files ${rows.length} rows, a tier files ${ROWS_PER_TIER}`,
      );
      return 1;
    }
  }
  mkdirSync(setDir, { recursive: true });
  writeFileSync(indexPath, merged);
  const recipeRows = parsed.recipe?.length ?? 0;
  console.log(
    `reference set: tiers=${parsed.rows.map((row) => row.tier).filter((t, i, all) => all.indexOf(t) === i).join(",")} ` +
      `rows=${parsed.rows.length} recipe_rows=${recipeRows} index=${args.dir}/${INDEX_NAME}`,
  );
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
