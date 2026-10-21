// The three statue points, read the way the game will read them.
//
// RY1's acceptance: the map loads headless, the three points report their
// positions and radii. This is that loader: it reads the contested map and
// the war rules, prints every statue with its place and its size, and fails
// when the map and the war rules disagree about the three points.
//
//   node tools/faction-war-map.mjs
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => JSON.parse(readFileSync(join(ROOT, rel), "utf8"));

const map = read("content/maps/contested_middle.json");
const war = read("content/systems/war.json");
const problems = [];

const statues = (map.nodes ?? []).filter((n) => n.kind === "statue");
const warNodes = war.nodes ?? [];
const warById = new Map(warNodes.map((n) => [n.id, n]));

console.log(`map ${map.id}: ${statues.length} statue points, ${warNodes.length} in the war rules`);
for (const statue of statues) {
  const rule = warById.get(statue.id);
  if (!rule) {
    problems.push(`the war rules have no entry for ${statue.id}`);
    console.log(`  ${statue.id}: NO WAR RULE`);
    continue;
  }
  if (rule.capture_progress !== statue.capture_progress_s) {
    problems.push(`${statue.id}: the war says ${rule.capture_progress}s, the map says ${statue.capture_progress_s}s`);
  }
  const radius = statue.radius ?? 0;
  const spawns = (map.spawns ?? []).filter((s) => Math.hypot(s.x - statue.x, s.z - statue.z) < radius);
  console.log(
    `  ${statue.id} (${rule.label ?? "no label"}): x ${statue.x} z ${statue.z}, radius ${radius} m, ` +
    `capture ${statue.capture_progress_s}s, contested by ${statue.contested_by}, spawns inside: ${spawns.length}`);
}

if (statues.length !== warNodes.length) {
  problems.push(`the map has ${statues.length} statues, the war rules list ${warNodes.length}`);
}
if (statues.length !== 3) {
  problems.push(`the war needs exactly three points, the map has ${statues.length}`);
}

// Every statue must be reachable: a point nobody can stand on is not a point.
for (const statue of statues) {
  const radius = statue.radius ?? 0;
  if (!(radius > 0)) problems.push(`${statue.id} has no radius`);
  const near = (map.landmarks ?? []).filter((l) => Math.hypot((l.x ?? 0) - statue.x, (l.z ?? 0) - statue.z) < 80);
  if (!near.length) problems.push(`${statue.id} has no landmark near it: the player would not know it is there`);
}

if (problems.length) {
  for (const p of problems) console.log(`war-map: FAIL ${p}`);
  console.log(`RESULT faction-war-map errors=${problems.length}`);
  process.exit(1);
}
console.log(`RESULT faction-war-map ok points=${statues.length} problems=0`);
