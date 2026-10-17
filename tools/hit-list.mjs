// CT63 combat sound list with provenance.
//   node tools/hit-list.mjs           check; exit 1 when a pick is unsound or content/sound/HIT-LIST.md is stale
//   node tools/hit-list.mjs --write   rewrite the generated table in content/sound/HIT-LIST.md
//   node tools/hit-list.mjs --json    print the picks as JSON (what the SN10 log bridge reads)
// Maps every fight cue (the `FightCue` enum in crates/audio/src/crowd.rs) to a
// CC0 file already in content/audio, with URL + SPDX + author + SHA. A cue the
// list neither picks nor marks silent fails the check, so a new cue cannot ship
// without a sound decision. No audio is downloaded or decoded here.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { refusal } from "./provenance-audit.mjs";
import { fileDigest, mdTable, readJson, REPO, syncBlocks } from "./content-lib.mjs";

const AUDIO = join("content", "audio");
const PACK_MANIFEST = join(AUDIO, "engine2040-vol0-manifest.json");
const MIRROR = "https://github.com/maximkone/engine2040-public/blob/master/";
/** Source of every pick: the Vol0 procedural CC0 pack (order O-003, seed 7). */
const SOURCE = { url: `${MIRROR}content/audio/engine2040-vol0-manifest.json`, spdx: "CC0-1.0", author: "engine2040 Vol0 pack (procedural, seed 7, order O-003)" };
/** Hit damage in permille at or above which the crowd and the list use the heavy sample (`HIT_LOUD_DAMAGE`). */
export const HIT_LOUD_DAMAGE = 120;

/** The sound decisions: which file voices which cue when. Placeholders until recorded clips land (bank.rs). */
export const PICKS = [
  { cue: "Hit", when: `damage < ${HIT_LOUD_DAMAGE}`, file: "impacts/flesh-small.wav" },
  { cue: "Hit", when: `damage < ${HIT_LOUD_DAMAGE} (variant)`, file: "impacts/flesh-medium.wav" },
  { cue: "Hit", when: `damage >= ${HIT_LOUD_DAMAGE}`, file: "impacts/flesh-large.wav" },
  { cue: "Blocked", when: "chip damage on a held block", file: "impacts/wood-medium.wav" },
  { cue: "Parried", when: "block inside the parry window", file: "impacts/metal-medium.wav" },
  { cue: "Knockdown", when: "victim at zero hp", file: "impacts/flesh-large.wav" },
  { cue: "Knockdown", when: "victim hits the ground", file: "impacts/stone-medium.wav" },
  { cue: "Dodged", when: "i-frames negate the hit (shares the footstep voice)", file: "footsteps/gravel-01.wav" },
];
/** Cues that start no sound on purpose, with the reason. */
export const SILENT = { StaminaOut: "a drained bar starts nothing; the coherence bridge expects silence" };

/** Variant names of the `FightCue` enum, read from the audio crate. */
export function fightCues(repo = REPO) {
  const src = readFileSync(join(repo, "crates", "audio", "src", "crowd.rs"), "utf8");
  const body = /pub enum FightCue \{([\s\S]*?)\n\}/.exec(src)[1];
  return [...body.matchAll(/^    ([A-Z][A-Za-z]+)(?: \{|,)/gm)].map((m) => m[1]);
}

/** One row per pick with the file's SHA-256 and bytes, or null digest when the file is absent. */
export function pickRows(repo = REPO) {
  return PICKS.map((p) => ({ ...p, name: `content/audio/${p.file}`, ...SOURCE, digest: fileDigest(join(repo, AUDIO, p.file)) }));
}

/** Problems with the list: uncovered cues, unknown picks, missing or non-WAV files, files the pack does not call CC0, gate refusals. */
export function checkHitList(repo = REPO) {
  const problems = [];
  const cues = fightCues(repo);
  const picked = new Set(PICKS.map((p) => p.cue));
  for (const cue of cues) if (!picked.has(cue) && !SILENT[cue]) problems.push(`${cue}: no pick and not marked silent`);
  for (const cue of [...picked, ...Object.keys(SILENT)]) if (!cues.includes(cue)) problems.push(`${cue}: not a FightCue variant`);
  const pack = new Map(readJson(join(repo, PACK_MANIFEST)).files.map((f) => [f.path.replace(/^sounds\//, ""), f.license]));
  const seen = new Set();
  for (const r of pickRows(repo)) {
    if (seen.has(`${r.cue} ${r.file}`)) problems.push(`${r.cue}: ${r.file} listed twice`);
    seen.add(`${r.cue} ${r.file}`);
    if (!r.digest) {
      problems.push(`${r.file}: missing on disk`);
      continue;
    }
    const head = readFileSync(join(repo, AUDIO, r.file)).subarray(0, 12);
    if (head.toString("latin1", 0, 4) !== "RIFF" || head.toString("latin1", 8, 12) !== "WAVE") problems.push(`${r.file}: not a RIFF/WAVE file`);
    if (pack.get(r.file) !== "CC0") problems.push(`${r.file}: the Vol0 manifest does not mark it CC0`);
    const code = refusal({ name: r.name, url: r.url, spdx: r.spdx, author: r.author, sha: r.digest.sha256 });
    if (code) problems.push(`${r.file}: the release gate would refuse it (${code})`);
  }
  return problems;
}

/** Markdown body of the generated list. */
export function hitListBody(repo = REPO) {
  const rows = pickRows(repo).map((r) => [r.cue, r.when, `\`${r.file}\``, r.url, r.spdx, r.author, r.digest ? `\`${r.digest.sha256}\`` : "MISSING", r.digest ? r.digest.bytes : "MISSING"]);
  const silent = Object.entries(SILENT).map(([cue, why]) => `- \`${cue}\`: silent, ${why}.`);
  return [mdTable(["cue", "when", "file (under content/audio)", "URL", "SPDX", "author", "SHA-256", "bytes"], rows), "", "Silent cues:", "", ...silent].join("\n");
}

const SEED = `# CT63 combat sound list

Every fight cue in the staged fight (the \`FightCue\` enum in \`crates/audio/src/crowd.rs\`) is either voiced by a CC0 file
below or marked silent. All picks come from the Vol0 procedural pack already in \`content/audio\` (no download). They are
placeholders: recorded clips replace them row by row with their own URL + SPDX + author + SHA.
\`node tools/hit-list.mjs\` checks the table against the files and the enum; \`--json\` is what the SN10 log bridge reads.

<!-- gen:hit-list -->

<!-- /gen:hit-list -->
`;

/** Generated-block specs for syncBlocks. */
export function hitListBlocks(repo = REPO) {
  return [{ file: "content/sound/HIT-LIST.md", name: "hit-list", body: hitListBody(repo), seed: SEED }];
}

if (import.meta.main) {
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify(pickRows().map((r) => ({ cue: r.cue, when: r.when, file: r.name, url: r.url, spdx: r.spdx, author: r.author, sha256: r.digest && r.digest.sha256 })), null, 1));
    process.exit(0);
  }
  const write = process.argv.includes("--write");
  const stale = syncBlocks(hitListBlocks(), { write });
  const problems = checkHitList();
  if (!write) for (const s of stale) problems.push(`stale generated block ${s}`);
  for (const p of problems) console.error(`hit-list FAIL: ${p}`);
  console.log(`hit-list ${problems.length === 0 ? "ok" : "issues=" + problems.length}${write ? ` (wrote ${stale.length} blocks)` : ""}`);
  process.exit(problems.length === 0 ? 0 : 1);
}
