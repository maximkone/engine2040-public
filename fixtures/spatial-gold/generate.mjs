// Deterministic generator for fixtures/spatial-gold/tasks.tsv.
// Byte-identical to the checked-in file. Fix repeats are code, not copies:
// 16 unique bases plus a 30-entry order. Gold strings stay verbatim.
// Vision capture_hex is recomputed per id with FNV-1a, same as Rust.
// Node stdlib only. No wall clock. LF only.
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const DIR = dirname(fileURLToPath(import.meta.url));
const TSV = join(DIR, "tasks.tsv");
const HEADER = "# spatial-gold v1\nid\tkind\tscene\ttool\tparams\tgold\tvision";

// FNV-1a 64, same owner as engine-core hash.rs.
const OFF = 0xcbf29ce484222325n;
const PRIME = 0x100000001b3n;
const MASK = 0xffffffffffffffffn;

function mixByte(h, b) {
  return ((h ^ BigInt(b)) * PRIME) & MASK;
}

function writeU64(h, v) {
  let x = BigInt(v);
  for (let i = 0; i < 8; i++) {
    h = mixByte(h, Number((x >> BigInt(8 * i)) & 0xffn));
  }
  return h;
}

// Same as spatial_eval.rs capture_hex: tag plus length-prefixed id.
export function captureHex(id) {
  let h = OFF;
  for (const b of Buffer.from("e2sp12:1", "utf8")) h = mixByte(h, b);
  h = writeU64(h, Buffer.byteLength(id, "utf8"));
  for (const b of Buffer.from(id, "utf8")) h = mixByte(h, b);
  return h.toString(16).padStart(16, "0");
}

const ROOMS = ["room_pillar", "room_open", "room_tight"];

// 30 place golds in id order. Params come from the sweep below.
const PLACE_GOLDS = [
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=0;conflicts=41",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=0;conflicts=31",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=0;conflicts=25",
  "clean=1;conflicts=",
  "clean=1;conflicts=",
  "clean=0;conflicts=20",
  "clean=1;conflicts=",
  "clean=0;conflicts=44",
];

// 40 query rows: scene, tool, params, gold, note. Capture is per id.
const QUERIES = [
  ["room_pillar", "scene.query near", "anchor=21;radius_cm=150", "ids=", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=20;radius_cm=250", "ids=", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=22;radius_cm=450", "ids=10,21,25", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=21;radius_cm=500", "ids=10,20,22,23,25", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=20;radius_cm=2000", "ids=10,21,22,23,24,25,26", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=22;radius_cm=300", "ids=10,25", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=21;radius_cm=450", "ids=10,20,22,23,25", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=20;radius_cm=450", "ids=21", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=21;radius_cm=120", "ids=", "advisory:agree"],
  ["room_pillar", "scene.query near", "anchor=22;radius_cm=900", "ids=10,20,21,23,25,26", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=20;b=22", "ids=10,21", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=21;b=22", "ids=10", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=24;b=22", "ids=10,20,21", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=23;b=22", "ids=25", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=24;b=21", "ids=20", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=22;b=23", "ids=25", "advisory:agree"],
  ["room_pillar", "scene.query between", "a=20;b=22", "ids=10,21", "advisory:agree"],
  ["room_open", "scene.query between", "a=30;b=32", "ids=31", "advisory:agree"],
  ["room_pillar", "scene.query visible", "ex=-300;ez=0;oct=0", "ids=21,25", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=-300;ez=0;oct=4", "ids=20", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=0;ez=-300;oct=2", "ids=10,21,25", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=0;ez=300;oct=6", "ids=10,21", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=200;ez=-300;oct=2", "ids=10,21,22,23", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=-300;ez=200;oct=0", "ids=10,21,22,23,25", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=400;ez=400;oct=5", "ids=10,21,22,25", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query visible", "ex=0;ez=0;oct=0", "ids=21", "advisory:confirm-occluder-in-capture"],
  ["room_pillar", "scene.query reachable", "anchor=20", "ids=10,21,23,24,25,26", "advisory:agree"],
  ["room_pillar", "scene.query reachable", "anchor=22", "ids=", "advisory:agree"],
  ["room_pillar", "scene.query reachable", "anchor=27", "ids=", "advisory:agree"],
  ["room_open", "scene.query reachable", "anchor=30", "ids=31,32", "advisory:agree"],
  ["room_open", "scene.query reachable", "anchor=33", "ids=", "advisory:agree"],
  ["room_tight", "scene.query reachable", "anchor=40", "ids=41,42,43,44", "advisory:agree"],
  ["room_pillar", "scene.query cover_near", "anchor=20;tx=1000;tz=0;crouch=0;radius_cm=3000", "ids=10,21,22,23,24,25,26", "advisory:confirm-threat-sightline"],
  ["room_pillar", "scene.query cover_near", "anchor=20;tx=1000;tz=0;crouch=1;radius_cm=3000", "ids=10,21,22,23,24,25,26", "advisory:confirm-threat-sightline"],
  ["room_open", "scene.query cover_near", "anchor=30;tx=1000;tz=1000;crouch=0;radius_cm=3000", "ids=31,32,33", "advisory:confirm-threat-sightline"],
  ["room_pillar", "scene.query cover_near", "anchor=21;tx=-1000;tz=0;crouch=1;radius_cm=3000", "ids=10,20,22,23,25,26", "advisory:confirm-threat-sightline"],
  ["room_pillar", "scene.query largest_open", "capsule_cm=30", "empty=0;min_clear=30", "advisory:confirm-rest-contact"],
  ["room_pillar", "scene.query largest_open", "capsule_cm=9000", "empty=1;min_clear=0", "advisory:confirm-rest-contact"],
  ["room_pillar", "scene.query free_space", "capsule_cm=30;gap_cm=100;max_n=8;seed=7", "empty=0;min_clear=30", "advisory:confirm-rest-contact"],
  ["room_tight", "scene.query free_space", "capsule_cm=9500;gap_cm=100;max_n=8;seed=7", "empty=1;min_clear=0", "advisory:confirm-rest-contact"],
];

// 16 unique fix bases: scene, x, z, gold. Repeats share gold.
const FIX_BASES = [
  ["room_pillar", 20, 390, "repair=60,0;at=80,390"],
  ["room_open", 30, -10, "repair=50,0;at=80,-10"],
  ["room_tight", -60, -10, "repair=40,0;at=-20,-10"],
  ["room_pillar", 220, -10, "repair=60,0;at=280,-10"],
  ["room_open", -270, -310, "repair=50,0;at=-220,-310"],
  ["room_tight", 40, -310, "repair=40,0;at=80,-310"],
  ["room_pillar", 420, -10, "repair=0,-70;at=420,-80"],
  ["room_open", 430, -410, "repair=0,90;at=430,-320"],
  ["room_tight", 140, -10, "repair=40,0;at=180,-10"],
  ["room_pillar", 320, 390, "repair=60,0;at=380,390"],
  ["room_open", 330, 290, "repair=50,0;at=380,290"],
  ["room_tight", -260, -10, "repair=40,0;at=-220,-10"],
  ["room_pillar", 20, -10, "repair=60,0;at=80,-10"],
  ["room_tight", 340, -10, "repair=40,0;at=380,-10"],
  ["room_pillar", 220, 190, "repair=60,0;at=280,190"],
  ["room_pillar", -380, -10, "repair=60,0;at=-320,-10"],
];

// 30 fix rows as base indices. Same params keep same gold.
const FIX_ORDER = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 1, 13, 14, 4, 2, 15, 7, 5, 0, 10, 8, 3, 1, 11, 6, 4, 13];

export function buildRows() {
  const rows = [];
  for (let i = 0; i < 30; i++) {
    const id = `sp12-${String(i + 1).padStart(3, "0")}`;
    const scene = ROOMS[i % 3];
    const x = -450 + ((i * 137) % 900);
    const z = -450 + ((i * 251) % 900);
    rows.push({
      id,
      kind: "place",
      scene,
      tool: "scene.place",
      params: `x=${x};z=${z};hx=40;hz=40`,
      gold: PLACE_GOLDS[i],
      vision: `advisory:confirm-rest-contact;${captureHex(id)}`,
    });
  }
  for (let i = 0; i < 40; i++) {
    const id = `sp12-${String(31 + i).padStart(3, "0")}`;
    const [scene, tool, params, gold, note] = QUERIES[i];
    rows.push({ id, kind: "query", scene, tool, params, gold, vision: `${note};${captureHex(id)}` });
  }
  for (let i = 0; i < 30; i++) {
    const id = `sp12-${String(71 + i).padStart(3, "0")}`;
    const [scene, x, z, gold] = FIX_BASES[FIX_ORDER[i]];
    rows.push({
      id,
      kind: "fix",
      scene,
      tool: "tx.preview",
      params: `x=${x};z=${z};hx=40;hz=40`,
      gold,
      vision: `advisory:confirm-rest-contact;${captureHex(id)}`,
    });
  }
  return rows;
}

export function encodeTsv(rows) {
  const out = [HEADER];
  for (const r of rows) out.push(`${r.id}\t${r.kind}\t${r.scene}\t${r.tool}\t${r.params}\t${r.gold}\t${r.vision}`);
  return out.join("\n") + "\n";
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log("Usage: node generate.mjs [--check | --write | --json]");
    process.exit(0);
  }
  const rows = buildRows();
  if (args.includes("--json")) {
    console.log(JSON.stringify(rows));
    return;
  }
  const made = encodeTsv(rows);
  const disk = readFileSync(TSV, "utf8");
  if (args.includes("--write")) {
    if (made === disk) {
      console.log("[PASS] tasks.tsv identical, no write");
      return;
    }
    writeFileSync(TSV, made, "utf8");
    console.log("[PASS] tasks.tsv regenerated from 16 fix bases");
    return;
  }
  if (args.includes("--check")) { /* explicit --check falls through to default verify below */ }
  if (made === disk) {
    console.log(`[PASS] tasks.tsv matches generator (100 rows, 16 fix bases, capture_hex kept)`);
    return;
  }
  console.error("[FAIL] tasks.tsv drifted from generator; run with --write");
  const a = made.split("\n");
  const b = disk.split("\n");
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] !== b[i]) {
      console.error(`first drift line ${i + 1}: gen=${JSON.stringify(a[i])} disk=${JSON.stringify(b[i])}`);
      break;
    }
  }
  process.exit(1);
}

if (import.meta.main) main();
