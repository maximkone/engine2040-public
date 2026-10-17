// Fighter content gates (CT41 LOD table, CT32 clip dedupe manifest, CT33 sidecar conformance).
//   node tools/fighter-report.mjs           check; exit 1 when a gate or a generated block is stale
//   node tools/fighter-report.mjs --write   rewrite the generated blocks, then check
// Reads content/fighters only: lod/hero.json (CT35 bake), manifest.json (CT7
// cook pins), glb/clips, sidecars and tools/ct7proof/src/main.rs (the 14-clip
// table). Nothing is cooked here; GPU ms stays PENDING until a quiesced run on
// the reference GPU.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileDigest, mdTable, mib, readJson, REPO, syncBlocks } from "./content-lib.mjs";

export const FIGHTERS = join(REPO, "content", "fighters");
/** LOD triangle ratios the cooker targets (`LOD_RATIOS` in mesh_cook.rs). */
export const LOD_RATIOS = [1, 0.5, 0.25, 0.125];
/** Allowed drift of a summed LOD level from its target ratio. */
export const RATIO_SLACK = 0.01;
/** Fighters in the CT27 soak scale card. */
export const SOAK_FIGHTERS = 64;
/** Traversal edge vocabulary (`TRAVERSAL_EDGES` in retarget.rs). */
export const TRAVERSAL_EDGES = ["vault", "hurdle", "mantle", "climb", "drop", "finisher"];

const sum = (xs) => xs.reduce((a, b) => a + b, 0);

/** Per-level triangle and byte totals over the meshes of one LOD bake. */
export function lodTotals(hero) {
  return LOD_RATIOS.map((_, level) => {
    const lods = hero.meshes.map((m) => m.lods[level]);
    return { tris: sum(lods.map((l) => l.tris)), bytes: sum(lods.map((l) => l.bytes)) };
  });
}

/** Problems with the CT35 bake: cook x2, chain gate, monotonic tris, hulls, budget, ratios. */
export function checkLod(hero) {
  const problems = [];
  if (hero.cook_x2_identical !== true) problems.push("hero.json: cook x2 is not identical");
  if (hero.gate !== "check_chain:ok") problems.push(`hero.json: gate is ${hero.gate}`);
  for (const m of hero.meshes) {
    const tris = m.lods.map((l) => l.tris);
    if (tris.length !== LOD_RATIOS.length) problems.push(`mesh ${m.index}: ${tris.length} levels, want ${LOD_RATIOS.length}`);
    if (tris[0] !== m.base_tris) problems.push(`mesh ${m.index}: LOD0 ${tris[0]} tris differs from base ${m.base_tris}`);
    if (m.base_tris > hero.class_max_tris) problems.push(`mesh ${m.index}: ${m.base_tris} tris over class max ${hero.class_max_tris}`);
    if (!tris.every((t, i) => i === 0 || t < tris[i - 1])) problems.push(`mesh ${m.index}: tris do not strictly decrease (${tris.join(">")})`);
    m.lods.filter((l) => !l.hull_contains).forEach((l) => problems.push(`mesh ${m.index} LOD${l.level}: hull does not contain the mesh`));
  }
  const totals = lodTotals(hero);
  totals.forEach((t, i) => {
    if (Math.abs(t.tris / totals[0].tris - LOD_RATIOS[i]) > RATIO_SLACK) problems.push(`LOD${i}: ratio ${(t.tris / totals[0].tris).toFixed(4)} is off the ${LOD_RATIOS[i]} target`);
  });
  return problems;
}

/** Markdown body of the CT41 LOD table. VRAM est. is the uploaded buffer bytes (attributes + indices). */
export function lodBody(hero) {
  const cell = (l) => `${l.tris} / ${l.bytes}`;
  const rows = hero.meshes.map((m) => [m.index, ...m.lods.map(cell), `${m.lods.filter((l) => l.hull_contains).length}/${m.lods.length} contain`]);
  const totals = lodTotals(hero);
  rows.push(["**hero total**", ...totals.map((t) => `**${t.tris} / ${t.bytes} (${(t.bytes / 1024).toFixed(1)} KiB)**`), `**${hero.meshes.flatMap((m) => m.lods).filter((l) => l.hull_contains).length}/${hero.meshes.length * LOD_RATIOS.length} contain**`]);
  const table = mdTable(["mesh", "LOD0 tris/bytes", "LOD1 tris/bytes", "LOD2 tris/bytes", "LOD3 tris/bytes", "hulls"], rows);
  const gpu = mdTable(
    ["per-LOD (hero total)", "tris", "VRAM est. bytes (buffers)", "GPU ms"],
    totals.map((t, i) => [`LOD${i}`, t.tris, t.bytes, "PENDING"]),
  );
  return [
    table,
    "",
    gpu,
    "",
    `- Chains: ${hero.meshes.map((m) => m.lods.map((l) => l.tris).join(">")).join("; ")}.`,
    `- Ratios of totals: ${totals.map((t) => (t.tris / totals[0].tris).toFixed(4)).join(" / ")} against \`LOD_RATIOS\` ${LOD_RATIOS.join(" / ")}.`,
    `- LOD0 budgets: ${hero.meshes.map((m) => m.base_tris).join(", ")} all under the ${hero.class} max ${hero.class_max_tris}.`,
    `- ${SOAK_FIGHTERS}-fighter buffer scale (CT27 soak): all-LOD0 ${SOAK_FIGHTERS} x ${totals[0].bytes} = ${mib(SOAK_FIGHTERS * totals[0].bytes)} MiB; all-LOD3 ${SOAK_FIGHTERS} x ${totals[3].bytes} = ${mib(SOAK_FIGHTERS * totals[3].bytes)} MiB.`,
    "- VRAM est. counts the vertex and index buffers as cooked (no textures, no skin matrices). GPU ms is PENDING until the quiesced reference-GPU run; no number is invented.",
  ].join("\n");
}

/** The 14 `(action, short)` clip pairs from the ct7proof CLIPS table. */
export function clipTable(dir = FIGHTERS) {
  const src = readFileSync(join(dir, "tools", "ct7proof", "src", "main.rs"), "utf8");
  return [...src.matchAll(/\("(human_male_[a-z0-9_]+)", "([a-z0-9_]+)"\)/g)].map((m) => ({ action: m[1], short: m[2] }));
}

/** Everything known about each canonical clip: GLB and sidecar digests, cook pins, per-variant digests. */
export function clipRows(dir = FIGHTERS) {
  const manifest = readJson(join(dir, "manifest.json"));
  return clipTable(dir).map(({ action, short }) => {
    const pins = manifest.clips.filter((c) => c.short === short);
    const hero = pins.find((c) => c.variant === "hero");
    return {
      short,
      action,
      glb: fileDigest(join(dir, "glb", "clips", `${action}.glb`)),
      sidecar: fileDigest(join(dir, "sidecars", `${short}.json`), { text: true }),
      hero,
      digests: [...new Set(pins.map((c) => c.digest))],
      slots: pins.length,
    };
  });
}

/** Problems with the dedupe set: files present, one digest per clip across variants, flags agree with sidecars. */
export function checkDedupe(dir = FIGHTERS) {
  const problems = [];
  for (const r of clipRows(dir)) {
    if (!r.glb) problems.push(`${r.short}: glb/clips/${r.action}.glb missing`);
    if (!r.sidecar) problems.push(`${r.short}: sidecars/${r.short}.json missing`);
    if (!r.hero) {
      problems.push(`${r.short}: no hero pin in manifest.json`);
      continue;
    }
    if (r.digests.length !== 1) problems.push(`${r.short}: ${r.digests.length} cook digests across ${r.slots} slots, want 1`);
    if (r.sidecar) {
      const side = readJson(join(dir, "sidecars", `${r.short}.json`));
      if (String(side.loop) !== r.hero.loop) problems.push(`${r.short}: sidecar loop ${side.loop} differs from manifest ${r.hero.loop}`);
      if (String(side.root_motion) !== r.hero.root_motion) problems.push(`${r.short}: sidecar root_motion ${side.root_motion} differs from manifest ${r.hero.root_motion}`);
    }
  }
  return problems;
}

/** Markdown body of the CT32 dedupe table: 14 unique clips across all variant slots. */
export function dedupeBody(dir = FIGHTERS) {
  const rows = clipRows(dir);
  const sha16 = (d) => (d ? d.sha256.slice(0, 16) : "MISSING");
  const table = mdTable(
    ["#", "short", "source action", "glb bytes", "glb sha16", "cook digest", "dur s", "keys", "loop", "root", "sidecar bytes", "sidecar sha16"],
    rows.map((r, i) => [
      i + 1, r.short, r.action, r.glb ? r.glb.bytes : "MISSING", sha16(r.glb), r.digests.join(","), r.hero.duration_s, r.hero.keys,
      r.hero.loop, r.hero.root_motion, r.sidecar ? r.sidecar.bytes : "MISSING", sha16(r.sidecar),
    ]),
  );
  const slots = sum(rows.map((r) => r.slots));
  const unique = new Set(rows.flatMap((r) => r.digests)).size;
  const glbBytes = sum(rows.map((r) => (r.glb ? r.glb.bytes : 0)));
  return [
    table,
    "",
    `- ${slots} clip slots (${rows[0].slots} variants x ${rows.length} clips) hold ${unique} unique cook digests; one GLB per clip is ${glbBytes} bytes in total, so ${slots - unique} slots are duplicates of a pinned digest.`,
  ].join("\n");
}

/** Problems with the sidecars as the CB4 (hit windows), LM2 (contacts) and CT24 (traversal) consumers read them. */
export function checkSidecars(dir = FIGHTERS) {
  const problems = [];
  const durations = new Map(readJson(join(dir, "manifest.json")).clips.filter((c) => c.variant === "hero").map((c) => [c.short, Number(c.duration_s)]));
  const files = readdirSync(join(dir, "sidecars")).filter((f) => f.endsWith(".json")).sort();
  const shorts = [...durations.keys()].sort();
  if (files.join() !== shorts.map((s) => `${s}.json`).join()) problems.push(`sidecars ${files.length} files do not match the ${shorts.length} manifest clips`);
  for (const f of files) {
    const side = readJson(join(dir, "sidecars", f));
    const at = f.replace(".json", "");
    if (side.clip !== at) problems.push(`${f}: clip is ${side.clip}`);
    if (!side.feet || !side.feet.left || !side.feet.right) problems.push(`${f}: LM2 reads contacts, feet.left and feet.right are required`);
    if (typeof side.ground_y !== "number") problems.push(`${f}: ground_y is required`);
    const windows = side.hit_windows || [];
    if ((side.tags || []).includes("attack") && windows.length === 0) problems.push(`${f}: attack clip has no hit window for CB4`);
    for (const w of windows) {
      if (!/^SOCKET_[A-Za-z0-9_]+$/.test(w.socket)) problems.push(`${f}: socket ${w.socket} is not SOCKET_<name>`);
      if (!(w.start_s >= 0 && w.end_s > w.start_s && w.end_s <= durations.get(at))) problems.push(`${f}: window ${w.start_s}..${w.end_s} is outside the ${durations.get(at)} s clip`);
    }
    for (const t of side.traversal || []) {
      if (!TRAVERSAL_EDGES.includes(t.edge)) problems.push(`${f}: traversal edge ${t.edge} is off the vocabulary`);
    }
  }
  return problems;
}

const LOD_SEED = null;
const dedupeSeed = (name) => `<!-- gen:${name} -->\n<!-- /gen:${name} -->`;

/** Generated-block specs for syncBlocks. */
export function fighterBlocks(dir = FIGHTERS) {
  return [
    { file: "content/fighters/LOD-REPORT.md", name: "lod-table", body: lodBody(readJson(join(dir, "lod", "hero.json"))) },
    { file: "content/fighters/DEDUPE-MANIFEST.md", name: "dedupe-clips", body: dedupeBody(dir) },
  ];
}

/** Every fighter problem, one line each. */
export function fighterProblems(dir = FIGHTERS) {
  return [...checkLod(readJson(join(dir, "lod", "hero.json"))), ...checkDedupe(dir), ...checkSidecars(dir)];
}

if (import.meta.main) {
  const write = process.argv.includes("--write");
  const stale = syncBlocks(fighterBlocks(), { write });
  const problems = fighterProblems();
  for (const s of write ? [] : stale) problems.push(`stale generated block ${s}`);
  for (const p of problems) console.error(`fighter-report FAIL: ${p}`);
  console.log(`fighter-report ${problems.length === 0 ? "ok" : "issues=" + problems.length}${write ? ` (wrote ${stale.length} blocks)` : ""}`);
  process.exit(problems.length === 0 ? 0 : 1);
}
