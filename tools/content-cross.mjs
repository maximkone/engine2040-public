// Cross-file checks for the faction war content set.
// Every file is authored by a different agent, so the ids they invented must
// agree. This is the gate that catches that.
//
// Run: node tools/content-cross.mjs
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => JSON.parse(readFileSync(join(ROOT, rel), "utf8"));
const problems = [];
const note = (m) => problems.push(m);

const classes = readdirSync(join(ROOT, "content", "classes"))
  .filter((n) => n.endsWith(".json") && n !== "schema.json" && n !== "roster.json")
  .map((n) => read(join("content", "classes", n)));
const byId = new Map(classes.map((c) => [c.id, c]));

const war = read("content/systems/war.json");
const economy = read("content/systems/economy.json");
const items = read("content/systems/items.json");
const maps = ["contested_middle", "homeland_wardens", "homeland_ashkin"].map((n) => read(`content/maps/${n}.json`));

// --- classes ---------------------------------------------------------------
for (const c of classes) {
  for (const counter of c.counters ?? []) {
    const id = String(counter).split(":")[0].trim();
    if (!byId.has(id)) note(`class ${c.id} counters unknown class id "${id}"`);
  }
  if (c.branch && !byId.has(c.branch)) note(`class ${c.id} branches from unknown "${c.branch}"`);
  if (c.branch) {
    const parent = byId.get(c.branch);
    if (parent && parent.tier !== 0) note(`class ${c.id} branches from tier ${parent.tier}, expected tier 0`);
  }
  for (const stat of c.primary ?? []) {
    if (!c.growth || !(c.growth[stat] > 0)) note(`class ${c.id} has primary stat ${stat} with no growth`);
  }
  if (!c.loadout?.weapon) note(`class ${c.id} has no weapon in its loadout`);
}

// --- war nodes vs map nodes -------------------------------------------------
const mapNodes = new Set();
const captureNodes = new Set();
for (const m of maps) {
  for (const n of m.nodes ?? []) {
    mapNodes.add(n.id);
    if ((n.capture_progress_s ?? 0) > 0) captureNodes.add(n.id);
  }
}
const warNodes = (war.nodes ?? []).map((n) => n.id);
for (const id of warNodes) {
  if (!mapNodes.has(id)) note(`war.json node "${id}" has no node in any map`);
}
for (const id of captureNodes) {
  if (!warNodes.includes(id)) note(`map capture node "${id}" is not in war.json`);
}
const middle = maps[0];
const statues = (middle.nodes ?? []).filter((n) => n.kind === "statue");
if (statues.length !== warNodes.length) {
  note(`the contested map has ${statues.length} statue nodes, war.json lists ${warNodes.length}`);
}

// --- map portals -----------------------------------------------------------
const mapIds = new Set(maps.map((m) => m.id));
for (const m of maps) {
  for (const p of m.travel?.portals ?? []) {
    if (!mapIds.has(p.to)) note(`map ${m.id} portal points at unknown map "${p.to}"`);
  }
}

// --- economy vs maps and war ----------------------------------------------
const landmarkIds = new Set();
for (const m of maps) for (const l of m.landmarks ?? []) landmarkIds.add(l.id);
for (const shop of economy.shops ?? []) {
  if (shop.location && !landmarkIds.has(shop.location)) {
    note(`shop "${shop.id}" sits at unknown landmark "${shop.location}"`);
  }
}
const currencyIds = new Set((economy.currencies ?? []).map((c) => c.id));
for (const c of economy.currencies ?? []) {
  for (const s of c.sources ?? []) {
    if (s.kind && /honour|fame/.test(s.kind) && !currencyIds.has(s.kind)) {
      note(`currency ${c.id} has a source in unknown currency "${s.kind}"`);
    }
  }
}
const honour = (economy.currencies ?? []).find((c) => c.id === "honour");
if (war.honour && honour) {
  const warPerMin = war.honour.per_minute ?? war.honour.perMinute;
  if (warPerMin && honour.sources?.some((s) => s.kind === "war_window" && s.amount !== warPerMin * 60)) {
    note(`honour: war.json says ${warPerMin}/min, economy.json says ${honour.sources.find((s) => s.kind === "war_window").amount}/hour`);
  }
}
const warFameOnly = war.fame?.from_other_sources === false;
if (warFameOnly) {
  const allowed = new Set(war.fame?.from ?? []);
  for (const c of economy.currencies ?? []) {
    if (c.id !== "fame") continue;
    const outside = (c.sources ?? []).filter((s) => s.kind && !allowed.has(s.kind));
    if (outside.length) note(`fame is war-only but economy lists sources: ${outside.map((s) => s.kind).join(", ")}`);
  }
}

// --- items vs economy ------------------------------------------------------
const itemMetals = new Set((items.metals ?? []).map((m) => m.id));
const gemTypes = new Set(items.sockets?.gem_types ?? []);
const gemSizes = new Set((items.sockets?.gem_sizes ?? []).map((n) => `gem_${n}`));
const ecoMetals = new Set((economy.materials ?? []).map((m) => m.id));
for (const id of ecoMetals) {
  if (!itemMetals.has(id) && !gemTypes.has(id) && !gemSizes.has(id)) {
    note(`economy material "${id}" is neither a metal, a gem type nor a gem size`);
  }
}
for (const id of itemMetals) if (!ecoMetals.has(id)) note(`items.json metal "${id}" is not in economy.json materials`);

const zoneIds = new Set();
for (const m of maps) for (const z of m.zones ?? []) zoneIds.add(z.id);
const unknownDrops = (items.drops ?? []).filter((d) => d.source && !zoneIds.has(d.source));
if (unknownDrops.length) {
  note(`${unknownDrops.length} item drop sources are not map zones: ${[...new Set(unknownDrops.map((d) => d.source))].slice(0, 6).join(", ")}`);
}

// --- the art brief against the sheet ----------------------------------------
// A concept is drawn from the class sheet, so the drawing carries a hash of
// the loadout it was made from. If the sheet moved since, the art is stale
// and the gate says so. A class with no concept yet is not an error: the
// art has not been drawn, and the absence is not a conflict.
const conceptDir = join(ROOT, "content", "generated", "concepts");
if (existsSync(conceptDir)) {
  for (const c of classes) {
    const briefPath = join(conceptDir, `concept_${c.id}.provenance.json`);
    if (!existsSync(briefPath)) continue;
    let brief;
    try { brief = JSON.parse(readFileSync(briefPath, "utf8")); } catch { continue; }
    if (!brief.loadout_sha) continue;
    const sheet = JSON.stringify(c.loadout ?? {});
    const now = createHash("sha256").update(sheet).digest("hex").slice(0, 12);
    if (brief.loadout_sha !== now) {
      note(`class ${c.id} moved since its concept was drawn: redraw the art brief`);
    }
  }
}

// --- report ----------------------------------------------------------------
if (!problems.length) {
  console.log(`RESULT content-cross ok classes=${classes.length} maps=${maps.length} problems=0`);
} else {
  for (const p of problems) console.log(`cross: FAIL ${p}`);
  console.log(`RESULT content-cross errors=${problems.length}`);
  process.exit(1);
}
