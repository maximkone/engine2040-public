// Arena content gates (CT82 manifest, CT45 PBR/HDRI VRAM report, CT77 src receipt).
//   node tools/arena-report.mjs           check; exit 1 when a gate or a generated block is stale
//   node tools/arena-report.mjs --write   rewrite the generated blocks, then check
//   node tools/arena-report.mjs --refresh rewrite the bytes and sha256 of drifted manifest rows, then check
// content/arena/manifest.json is the one truth for the kit bytes. gen.mjs only
// writes the flat fallback kit, so a hand-edited file (texture wiring, layout)
// is re-pinned with --refresh. The two night/sunset HDRIs live in
// content/local/hdri (SIZE-BUDGET.md moved them out of git): a fresh checkout
// may lack them, so they are checked when present and pinned by name here.
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { lodTotals } from "./fighter-report.mjs";
import { fileDigest, hdrSize, imageSize, isTextPin, mdTable, mib, readJson, REPO, rgba8MipBytes, syncBlocks } from "./content-lib.mjs";

export const ARENA = join(REPO, "content", "arena");
export const LOCAL_HDRI = join(REPO, "content", "local", "hdri");
/** The four PBR material sets: each needs albedo, normal and roughness 2k maps plus an authored ORM. */
export const MATERIALS = ["sand", "stone", "wood", "cloth"];
/** Texture VRAM cap for the bound set, RGBA8 + mips upper bound (potato floor tier: 2 GB VRAM). */
export const POTATO_BOUND_CAP_BYTES = 256 * 1048576;
/** Same cap on the comfortable tier (3050 class). */
export const COMFORT_BOUND_CAP_BYTES = 1024 * 1048576;
/** Prop LOD0 triangle bar from content/arena/README.md. */
export const PROP_TRI_BUDGET = 5000;
/** Largest single src gltf the kit may commit (ratchet: current max banner.gltf is 7.5 MiB). */
export const SRC_BYTES_BUDGET = 8 * 1048576;
/** Fighters in the CT61 baseline scene (4 variants, 2 of each). */
export const SOAK_FIGHTERS = 8;
/** The crate + torch pair CT77 gates. */
export const PAIR = ["crate", "torch"];

/** Kit HDRIs that live only in the git-ignored local cache, with their size. The size is pinned so the reports read the same on a fresh checkout. */
export const LOCAL_ONLY = { "hdri/arena_sunset.hdr": { w: 4096, h: 2048 }, "hdri/arena_night.hdr": { w: 4096, h: 2048 } };

/** Manifest-relative path to the file on disk: the arena tree, else the local HDRI cache. */
export function arenaPath(rel, root = ARENA, local = LOCAL_HDRI) {
  const here = join(root, rel);
  if (existsSync(here)) return here;
  const cached = join(local, rel.replace(/^hdri\//, ""));
  return rel.startsWith("hdri/") && existsSync(cached) ? cached : here;
}

/** Problems with manifest.json as the sole truth for the kit bytes. Empty means clean. */
export function checkManifest(root = ARENA, local = LOCAL_HDRI) {
  const problems = [];
  if (existsSync(join(root, "manifest_before.json"))) problems.push("manifest_before.json: stale pre-kit manifest still present");
  const { files } = readJson(join(root, "manifest.json"));
  for (const [rel, want] of Object.entries(files)) {
    const got = fileDigest(arenaPath(rel, root, local), { text: isTextPin(rel) });
    if (!got && rel in LOCAL_ONLY) continue;
    if (!got) problems.push(`${rel}: missing on disk`);
    else if (got.bytes !== want.bytes) problems.push(`${rel}: ${got.bytes} bytes on disk, manifest says ${want.bytes}`);
    else if (got.sha256 !== want.sha256) problems.push(`${rel}: sha256 differs from the manifest`);
  }
  const required = MATERIALS.flatMap((m) => [`${m}_albedo_2k.jpg`, `${m}_normal_2k.jpg`, `${m}_roughness_2k.jpg`, `${m}_orm.png`]);
  for (const f of required) if (!files[`textures/${f}`]) problems.push(`textures/${f}: required map is not in the manifest`);
  const onDisk = [
    ...readdirSync(join(root, "src")).filter((f) => f.endsWith(".gltf")).map((f) => `src/${f}`),
    ...readdirSync(join(root, "textures")).filter((f) => /_2k\.jpg$|_orm\.png$/.test(f)).map((f) => `textures/${f}`),
  ];
  for (const rel of onDisk) if (!files[rel]) problems.push(`${rel}: on disk but not in the manifest`);
  return problems;
}

/** Re-pin the bytes and sha256 of every manifest row whose file is on disk and differs. Keys, order and format stay; returns the rows it changed. */
export function refreshManifest(root = ARENA, local = LOCAL_HDRI) {
  const path = join(root, "manifest.json");
  const manifest = readJson(path);
  const changed = [];
  for (const [rel, want] of Object.entries(manifest.files)) {
    const got = fileDigest(arenaPath(rel, root, local), { text: isTextPin(rel) });
    if (!got || (got.bytes === want.bytes && got.sha256 === want.sha256)) continue;
    manifest.files[rel] = { ...want, ...got };
    changed.push(rel);
  }
  if (changed.length) writeFileSync(path, JSON.stringify(manifest, null, 2));
  return changed;
}

/** Problems with the proof report: spatial findings and cook x2 must hold. Empty means clean. */
export function checkProofReport(root = ARENA) {
  const r = readJson(join(root, "report.json"));
  const problems = [];
  if (r.spatial.findings !== 0) problems.push(`report.json: spatial findings ${r.spatial.findings}, want 0`);
  if (!r.spatial.identical_twice) problems.push("report.json: spatial run is not identical twice");
  if (!r.bundle.identical_twice) problems.push("report.json: bundle cook is not identical twice");
  return problems;
}

/** The texture files the glTF embeds bind: albedo, normal and ORM per material (roughness JPEGs are staged, not bound). */
function boundMaps() {
  return MATERIALS.flatMap((m) => [`textures/${m}_albedo_2k.jpg`, `textures/${m}_normal_2k.jpg`, `textures/${m}_orm.png`]);
}

/** One VRAM row. A local-only HDRI takes its bytes and sha256 from the manifest and its size from LOCAL_ONLY, never from the disk. */
function vramRow(rel, root, local, want) {
  const pinned = LOCAL_ONLY[rel];
  const path = arenaPath(rel, root, local);
  const d = pinned ? want : fileDigest(path);
  const size = pinned ?? (d && (rel.endsWith(".hdr") ? hdrSize(path) : imageSize(path)));
  return { rel, d, size, vram: size ? rgba8MipBytes(size.w, size.h) : 0, cached: Boolean(pinned) };
}

/** Per-file VRAM rows plus the bound and active totals for the CT45 report. */
export function pbrStats(root = ARENA, local = LOCAL_HDRI) {
  const manifest = readJson(join(root, "manifest.json")).files;
  const rels = Object.keys(manifest).filter((r) => /_2k\.jpg$|_orm\.png$|^hdri\/arena_/.test(r));
  const rows = rels.map((r) => vramRow(r, root, local, manifest[r]));
  const bound = new Set(boundMaps());
  const boundVram = rows.filter((r) => bound.has(r.rel)).reduce((s, r) => s + r.vram, 0);
  const hdriVram = rows.find((r) => r.rel === "hdri/arena_day.hdr").vram;
  return { rows, bound, boundVram, hdriVram, active: boundVram + hdriVram };
}

/** Markdown body of the CT45 per-map bytes + VRAM block. */
export function pbrBody(root = ARENA, local = LOCAL_HDRI) {
  const { rows, bound, boundVram, hdriVram, active } = pbrStats(root, local);
  const table = mdTable(
    ["File", "Disk bytes", "SHA-256", "Dims", "VRAM est. RGBA8+mips", "Bound"],
    rows.map((r) => [
      `\`${r.rel}\`${r.cached ? " (content/local/hdri)" : ""}`,
      r.d ? r.d.bytes : "MISSING",
      r.d ? `\`${r.d.sha256}\`` : "MISSING",
      r.size ? `${r.size.w}x${r.size.h}` : "?",
      r.vram,
      bound.has(r.rel) ? "yes" : r.rel === "hdri/arena_day.hdr" ? "one active HDRI" : "no",
    ]),
  );
  const verdict = (cap) => (active <= cap ? "PASS" : "FAIL");
  return [
    table,
    "",
    `- Bound PBR set (albedo + normal + ORM per material): ${boundVram} bytes (${mib(boundVram)} MiB).`,
    `- Plus one active 4k HDRI: ${hdriVram} bytes. Active bound estimate: ${active} bytes (${mib(active)} MiB).`,
    `- Cap, potato tier (2 GB VRAM floor): ${POTATO_BOUND_CAP_BYTES} bytes (${mib(POTATO_BOUND_CAP_BYTES)} MiB): ${verdict(POTATO_BOUND_CAP_BYTES)}.`,
    `- Cap, comfortable tier (3050 class): ${COMFORT_BOUND_CAP_BYTES} bytes (${mib(COMFORT_BOUND_CAP_BYTES)} MiB): ${verdict(COMFORT_BOUND_CAP_BYTES)}.`,
    "- Estimate is an upper bound (RGBA8 plus full mips, no compressed upload). Measured VRAM with the adapter and driver line needs the reference GPU and is not claimed here.",
  ].join("\n");
}

/** Problems with the CT45 caps: every map present and the active bound set under the potato cap. */
export function checkPbrCaps(root = ARENA, local = LOCAL_HDRI) {
  const { rows, active } = pbrStats(root, local);
  const problems = rows.filter((r) => !r.d).map((r) => `${r.rel}: missing`);
  if (active > POTATO_BOUND_CAP_BYTES) problems.push(`active bound set ${active} bytes is over the potato cap ${POTATO_BOUND_CAP_BYTES}`);
  return problems;
}

/** The crate + torch rows: disk bytes and digest, manifest agreement, LOD0 tris. */
export function pairRows(root = ARENA) {
  const manifest = readJson(join(root, "manifest.json")).files;
  const report = readJson(join(root, "report.json"));
  return PAIR.map((name) => {
    const rel = `src/${name}.gltf`;
    const d = fileDigest(join(root, rel));
    const m = manifest[rel];
    return { rel, ...d, match: Boolean(m) && m.sha256 === d.sha256 && m.bytes === d.bytes, tris: report.assets.find((a) => a.asset === name).lod0_tris };
  });
}

/** Markdown body of the CT77 crate + torch commit receipt. */
export function srcReceiptBody(root = ARENA) {
  const report = readJson(join(root, "report.json"));
  const rows = pairRows(root).map((p) => [`\`${p.rel}\``, p.bytes, `\`${p.sha256}\``, p.match ? "match" : "DRIFT", p.tris, PROP_TRI_BUDGET, SRC_BYTES_BUDGET]);
  return [
    mdTable(["File", "Bytes", "SHA-256", "Manifest", "LOD0 tris", "Tri budget", "Byte budget"], rows),
    "",
    `- Cook x2: bundle identical_twice ${report.bundle.identical_twice}, spatial identical_twice ${report.spatial.identical_twice} (\`report.json\`).`,
    `- SP5 spatial over the placed kit: ${report.spatial.actors} actors, ${report.spatial.findings} findings, verdict ${report.spatial.verdict}, hash \`${report.spatial.hash}\`.`,
    "- Golden hash `d57b5c8ae3096614` is untouched: the kit is float content outside the hashed simulation.",
    "- Gate: `node tools/arena-report.mjs` fails when either file's bytes move without this receipt being rewritten with `--write`.",
  ].join("\n");
}

/** Problems with the CT77 pair: budgets, drift against the manifest, proof facts. */
export function checkSrcPair(root = ARENA) {
  const problems = checkProofReport(root);
  for (const p of pairRows(root)) {
    if (!p.match) problems.push(`${p.rel}: bytes differ from manifest.json`);
    if (p.bytes > SRC_BYTES_BUDGET) problems.push(`${p.rel}: ${p.bytes} bytes over the ${SRC_BYTES_BUDGET} budget`);
    if (p.tris > PROP_TRI_BUDGET) problems.push(`${p.rel}: ${p.tris} tris over the ${PROP_TRI_BUDGET} budget`);
  }
  return problems;
}

/** Payload and VRAM estimate for the CT61 empty-stands scene: the arena kit with the day HDRI plus the fighters (the hero LOD0 buffers stand in for all of them). */
export function soakStats(root = ARENA, local = LOCAL_HDRI) {
  const { active } = pbrStats(root, local);
  const fighters = join(root, "..", "fighters");
  const variants = readJson(join(fighters, "manifest.json")).variants;
  const atlas = variants.map((v) => {
    const [, w, h] = /^(\d+)x(\d+)/.exec(v.texture);
    return rgba8MipBytes(Number(w), Number(h));
  });
  const mesh = lodTotals(readJson(join(fighters, "lod", "hero.json")))[0].bytes;
  const atlases = atlas.reduce((a, b) => a + b, 0);
  return { kitTris: readJson(join(root, "report.json")).total_lod0_tris, active, atlases, variants: variants.length, meshBytes: SOAK_FIGHTERS * mesh, total: active + atlases + SOAK_FIGHTERS * mesh };
}

/** Markdown body of the CT61 baseline: estimates from files, GPU and CPU ms PENDING. */
export function soakBody(root = ARENA, local = LOCAL_HDRI) {
  const s = soakStats(root, local);
  const report = readJson(join(root, "report.json"));
  return [
    mdTable(
      ["Part", "Estimate"],
      [
        [`Arena kit LOD0 triangles (empty stands, ${report.spatial.actors} placed actors)`, s.kitTris],
        ["Kit PBR maps + one day HDRI, VRAM bytes", s.active],
        [`${s.variants} fighter body atlases (shared by ${SOAK_FIGHTERS} fighters), VRAM bytes`, s.atlases],
        [`${SOAK_FIGHTERS} x hero LOD0 mesh buffers, bytes`, s.meshBytes],
        ["**Total VRAM estimate (RGBA8 + mips upper bound)**", `**${s.total} bytes (${mib(s.total)} MiB)**`],
        ["CPU ms / GPU ms", "PENDING (reference GPU run)"],
      ],
    ),
    "",
    `- Cap, potato tier: ${POTATO_BOUND_CAP_BYTES} bytes (${mib(POTATO_BOUND_CAP_BYTES)} MiB): ${s.total <= POTATO_BOUND_CAP_BYTES ? "PASS" : "FAIL"}.`,
    `- Findings: spatial ${report.spatial.findings} over ${report.spatial.actors} actors, verdict ${report.spatial.verdict}; cook x2 identical: bundle ${report.bundle.identical_twice}, spatial ${report.spatial.identical_twice}.`,
    "- The 8-fighter day capture with CPU/GPU ms and measured VRAM (adapter + driver line) needs the quiesced reference-GPU run (PF13 rule: CPU load <10%, 3 runs within 10%, 21 samples). None is claimed here.",
  ].join("\n");
}

/** Problems with the CT61 baseline: the estimate must fit the potato cap. */
export function checkSoak(root = ARENA, local = LOCAL_HDRI) {
  const { total } = soakStats(root, local);
  return total > POTATO_BOUND_CAP_BYTES ? [`soak scene VRAM estimate ${total} bytes is over the potato cap ${POTATO_BOUND_CAP_BYTES}`] : [];
}

const SOAK_SEED = "# CT61 empty-stands baseline for the CT27 soak\n\n<!-- gen:soak-baseline -->\n\n<!-- /gen:soak-baseline -->\n";

const RECEIPT_SEED = "# CT77 arena src commit receipt (crate + torch)\n\nThe pair is committed only with this receipt current: cook x2 identical, SP5 0 findings, SHAs and budgets named.\n\n<!-- gen:src-receipt -->\n\n<!-- /gen:src-receipt -->\n";

/** Generated-block specs for syncBlocks. */
export function arenaBlocks(root = ARENA, local = LOCAL_HDRI) {
  return [
    { file: "content/arena/pbr/REPORT.md", name: "pbr-vram", body: pbrBody(root, local) },
    { file: "content/arena/SRC-RECEIPT.md", name: "src-receipt", body: srcReceiptBody(root), seed: RECEIPT_SEED },
    { file: "content/arena/SOAK-BASELINE.md", name: "soak-baseline", body: soakBody(root, local), seed: SOAK_SEED },
  ];
}

/** Every arena problem, one line each. */
export function arenaProblems(root = ARENA, local = LOCAL_HDRI) {
  return [...checkManifest(root, local), ...checkPbrCaps(root, local), ...checkSrcPair(root), ...checkSoak(root, local)];
}

if (import.meta.main) {
  const write = process.argv.includes("--write");
  if (process.argv.includes("--refresh")) for (const rel of refreshManifest()) console.log(`arena-report refreshed ${rel}`);
  const stale = syncBlocks(arenaBlocks(), { write });
  const problems = arenaProblems();
  for (const s of write ? [] : stale) problems.push(`stale generated block ${s}`);
  for (const p of problems) console.error(`arena-report FAIL: ${p}`);
  console.log(`arena-report ${problems.length === 0 ? "ok" : "issues=" + problems.length}${write ? ` (wrote ${stale.length} blocks)` : ""}`);
  process.exit(problems.length === 0 ? 0 : 1);
}
