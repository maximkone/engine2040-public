// CT46 provenance audit dry-run for the CT28 release gate.
//   node tools/provenance-audit.mjs           print every refusal; exit 1 only when the doc block is stale
//   node tools/provenance-audit.mjs --strict  also exit 1 when any row would be refused (the release gate)
//   node tools/provenance-audit.mjs --write   rewrite the generated block in content/PROVENANCE-AUDIT.md
// Builds one record per shipped arena and fighter asset (URL + SPDX + author +
// SHA, hand-made chain) from the manifests, checks the SHA against the file on
// disk, then runs every record through a JS mirror of
// crates/assets/src/provenance.rs `check_record` and names EVERY refusal, not
// just the first. The gate code itself stays in Rust (CT28); a test pins the
// SPDX allow-list here against the Rust list. No network, no downloads.
// The disk column reads the same on every checkout: a git-ignored or local-only
// file is always "off-tree" (and "DRIFT" when it is present and differs).
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { ARENA, arenaPath, LOCAL_HDRI, LOCAL_ONLY } from "./arena-report.mjs";
import { provenanceRows } from "./arena-sources.mjs";
import { clipRows, FIGHTERS } from "./fighter-report.mjs";
import { fileDigest, isTextPin, mdTable, readJson, REPO, syncBlocks } from "./content-lib.mjs";

/** Licences a release may carry; a test pins this to `ALLOWED_SPDX` in provenance.rs. */
export const ALLOWED_SPDX = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "Zlib", "ISC", "CC0-1.0", "Unlicense", "0BSD", "MIT-0", "Unicode-3.0", "Unicode-DFS-2016"];
const MAX_NAME_CHARS = 64;
const MAX_FIELD_CHARS = 256;
const SHA_MIN_LEN = 16;
const SHA_MAX_LEN = 64;
const MIRROR = "https://github.com/maximkone/engine2040-public/blob/master/";
const POLY = (slug) => `https://polyhaven.com/a/${slug}`;
const GEN = { url: `${MIRROR}content/arena/tools/gen.mjs`, author: "engine2040 CT8 lane" };
const ORM = { url: `${MIRROR}content/arena/textures/ORM-PROVENANCE.md`, author: "engine2040 CT26 lane" };
const PBR_SETS = {
  sand: [POLY("aerial_beach_01"), "Rob Tuytel"],
  stone: [POLY("sandstone_blocks_04"), "Rob Tuytel"],
  wood: [POLY("wooden_planks"), "Charlotte Baglioni + Dario Barresi"],
  cloth: [POLY("velour_velvet"), "Rico Cilliers (+ colormass)"],
};
const SKY = { "arena_day.hdr": "goegap", "arena_sunset.hdr": "venice_sunset", "arena_night.hdr": "moonlit_golf" };
/** ambientCG names no per-asset author; its pages credit the site and state CC0 (docs.ambientcg.com/license, read 2026-10-18). */
const AMBIENTCG = "ambientCG";
/** The vendored 1k studio HDRI; its `.provenance.json` sidecar holds URL, author, SHA and the tracked copies. */
export const STUDIO_HDRI = "content/incoming/studio_small_09_1k.hdr";

/** Who made an arena manifest file: Poly Haven downloads or the deterministic kit generator. */
function arenaSource(rel) {
  const set = /^textures\/([a-z]+)_(albedo|normal|roughness)_2k\.jpg$/.exec(rel);
  if (set) return { url: PBR_SETS[set[1]][0], author: PBR_SETS[set[1]][1] };
  const sky = /^hdri\/(arena_[a-z]+\.hdr)$/.exec(rel);
  if (sky) return { url: POLY(SKY[sky[1]]), author: "Greg Zaal" };
  return /_orm\.png$/.test(rel) ? ORM : GEN;
}

/** The subset of `rels` that git ignores. Without git (or with nothing ignored) it is empty. */
function gitIgnored(repo, rels) {
  try {
    return new Set(execFileSync("git", ["check-ignore", "--stdin"], { cwd: repo, input: rels.join("\n"), encoding: "utf8" }).split(/\r?\n/));
  } catch {
    return new Set();
  }
}

/** One record per shipped asset: {rec, disk} where disk is "match", "DRIFT" or "off-tree". */
export function auditRecords(repo = REPO) {
  const arena = join(repo, "content", "arena");
  const fighters = join(repo, "content", "fighters");
  const rows = [];
  const add = (name, src, sha, path, local = false) => rows.push({ name, src, sha, path, local });
  for (const [rel, f] of Object.entries(readJson(join(arena, "manifest.json")).files)) {
    add(`content/arena/${rel}`, arenaSource(rel), f.sha256, arenaPath(rel, arena, join(repo, "content", "local", "hdri")), rel in LOCAL_ONLY);
  }
  // The VR2 sets: each cooked KTX2 map is credited to the page of the source set it came from.
  for (const { cells } of provenanceRows(readFileSync(join(arena, "sources", "PROVENANCE.md"), "utf8"))) {
    for (const kind of ["base", "normal", "orm"]) {
      const file = `${cells[6].split(" ")[0]}_${kind}.ktx2`;
      // Poly Haven names the site as the set's publisher on the page (the artist
    // per set sits in that set's LICENCE.txt); ambientCG credits the site only.
    const author = /polyhaven\.com/.test(cells[1]) ? "Poly Haven" : AMBIENTCG;
    add(`content/arena/cooked/${file}`, { url: cells[1], author }, fileDigest(join(arena, "cooked", file))?.sha256 ?? "", join(arena, "cooked", file));
    }
  }
  // CT80: the studio HDRI is pinned by the sidecar next to the Q1 copy; the arena copy is the same bytes.
  const hdri = readJson(join(repo, "content", "incoming", "studio_small_09_1k.hdr.provenance.json"));
  for (const name of [STUDIO_HDRI, ...hdri.copies]) add(name, hdri, hdri.sha256, join(repo, name));
  const man = readJson(join(fighters, "manifest.json"));
  const page = man.sources[0].page;
  const pack = { url: page, author: "System G6 (aka Qoma)" };
  for (const f of man.incoming_files.filter((x) => x.sha256)) {
    const local = join(repo, "content", "local", f.path.replace(/^incoming\//, "").replace(/^staged\//, ""));
    add(`content/fighters/${f.path}`, { url: page, author: f.author }, f.sha256, existsSync(join(fighters, f.path)) ? join(fighters, f.path) : local);
  }
  for (const v of man.variants) add(`content/fighters/glb/${v.name}_body.glb`, pack, v.body_glb_sha256, join(fighters, "glb", `${v.name}_body.glb`));
  for (const c of clipRows(fighters)) add(`content/fighters/glb/clips/${c.action}.glb`, pack, c.glb.sha256, join(fighters, "glb", "clips", `${c.action}.glb`));
  const ignored = gitIgnored(repo, rows.map((r) => r.name));
  return rows.map(({ name, src, sha, path, local }) => ({
    rec: { name, url: src.url, spdx: src.spdx ?? "CC0-1.0", author: src.author, sha },
    disk: diskState(path && fileDigest(path, { text: isTextPin(name) }), sha, local || ignored.has(name)),
  }));
}

/** The disk column: "DRIFT" when the file is present and differs, "match" for a present tracked file, else "off-tree". */
export function diskState(digest, sha, outside) {
  if (digest && digest.sha256 !== sha) return "DRIFT";
  return digest && !outside ? "match" : "off-tree";
}

const isDigest = (s) => s.length >= SHA_MIN_LEN && s.length <= SHA_MAX_LEN && /^[0-9a-f]+$/.test(s);

/** JS mirror of provenance.rs `check_record` for the hand-made chain: the refusal code, or null when the row passes. */
export function refusal(r) {
  if (!r.name || r.name.length > MAX_NAME_CHARS || !/^[a-z0-9_\-/.]+$/.test(r.name)) return "bad_name";
  if (!r.spdx || r.spdx.length > MAX_FIELD_CHARS || /\s/.test(r.spdx)) return "bad_spdx";
  if (!ALLOWED_SPDX.some((s) => s.toLowerCase() === r.spdx.toLowerCase())) return "license_refused";
  if (!isDigest(r.sha)) return "bad_sha";
  if (!r.url.trim() || r.url.length > MAX_FIELD_CHARS || !r.url.includes("://") || /\s/.test(r.url)) return "missing_url";
  if (!r.author.trim() || r.author.length > MAX_FIELD_CHARS) return "missing_author";
  return null;
}

/** Every refusal in a release, in row order: gate refusals, duplicate names, and SHAs that no longer match the file. */
export function dryRun(rows) {
  const seen = new Set();
  const gaps = [];
  for (const { rec, disk } of rows) {
    if (seen.has(rec.name)) gaps.push({ asset: rec.name, code: "duplicate" });
    seen.add(rec.name);
    const code = refusal(rec);
    if (code) gaps.push({ asset: rec.name, code });
    if (disk === "DRIFT") gaps.push({ asset: rec.name, code: "sha_drift" });
  }
  return gaps;
}

/** Trees the matrix cannot list per file, derived from the fighter manifest and .gitignore. Each names its owner row. */
export function unlistedTrees(repo = REPO) {
  const man = readJson(join(repo, "content", "fighters", "manifest.json"));
  const ignore = readFileSync(join(repo, ".gitignore"), "utf8");
  const trees = man.incoming_files.filter((f) => !f.sha256).map((f) => ({ tree: `content/fighters/${f.path}`, why: f.change, owner: "CT58/CT75" }));
  if (ignore.includes("content/incoming/*")) trees.push({ tree: "content/incoming/", why: "git-ignored dropbox; only the studio HDRI is carved out and listed", owner: "CT80" });
  if (ignore.includes("content/local/")) trees.push({ tree: "content/local/", why: "git-ignored cache; raw donor bytes never ship, only cooked output with provenance", owner: "CT4 flow" });
  if (existsSync(join(repo, "content", "arena", "sources"))) trees.push({ tree: "content/arena/sources/", why: "raw CC0 ambientCG inputs of the cooked maps; mixed-case names the gate refuses, they never ship", owner: "CT74" });
  return trees;
}

/** Markdown body of the generated matrix and refusal list. */
export function auditBody(repo = REPO) {
  const rows = auditRecords(repo);
  const gaps = dryRun(rows);
  const table = mdTable(
    ["#", "gate name", "URL", "SPDX", "author", "SHA-256", "disk"],
    rows.map(({ rec, disk }, i) => [i + 1, rec.name, rec.url, rec.spdx, rec.author, rec.sha, disk]),
  );
  const refusals = gaps.length ? gaps.map((g) => `- \`${g.code}\` names \`${g.asset}\``) : ["- none: every row passes the gate and matches its file"];
  const trees = unlistedTrees(repo).map((t) => `- \`${t.tree}\` (owner ${t.owner}): ${t.why}. Listed per file it would refuse with \`bad_sha\`.`);
  return [
    `${rows.length} records, all hand-made chain (no AI generator rows yet; CT71 adds them with generator + model + prompt digest). Disk column: \`match\` SHA equals the file, \`DRIFT\` it does not, \`off-tree\` the file is not in the tracked tree.`,
    "",
    table,
    "",
    `Release-refusal dry-run: ${gaps.length} refusals.`,
    "",
    ...refusals,
    "",
    "Trees outside the matrix (a release would refuse them until each has rows):",
    "",
    ...trees,
  ].join("\n");
}

/** Generated-block specs for syncBlocks. */
export function auditBlocks(repo = REPO) {
  return [{ file: "content/PROVENANCE-AUDIT.md", name: "audit-matrix", body: auditBody(repo) }];
}

if (import.meta.main) {
  const write = process.argv.includes("--write");
  const stale = syncBlocks(auditBlocks(), { write });
  const gaps = dryRun(auditRecords());
  for (const g of gaps) console.log(`provenance-audit would refuse ${g.code}: ${g.asset}`);
  if (!write) for (const s of stale) console.error(`provenance-audit: stale generated block ${s}`);
  const bad = (write ? 0 : stale.length) + (process.argv.includes("--strict") ? gaps.length : 0);
  console.log(`provenance-audit ${gaps.length} refusals, ${bad === 0 ? "ok" : "issues=" + bad}${write ? ` (wrote ${stale.length} blocks)` : ""}`);
  process.exit(bad === 0 ? 0 : 1);
}
