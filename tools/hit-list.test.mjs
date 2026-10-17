// Run: node --test tools/hit-list.test.mjs
// CT63: every fight cue is voiced by a CC0 file with provenance or marked silent.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { syncBlocks } from "./content-lib.mjs";
import { checkHitList, fightCues, hitListBlocks, hitListBody, PICKS, pickRows } from "./hit-list.mjs";

const CUES = ["Hit", "Blocked", "Parried", "Dodged", "StaminaOut", "Knockdown"];
const wav = () => Buffer.concat([Buffer.from("RIFF"), Buffer.alloc(4), Buffer.from("WAVEfmt "), Buffer.alloc(8)]);

/** A repo with the enum, every picked file as a WAV, and a pack manifest that calls them CC0. */
function fixture(cues = CUES) {
  const repo = mkdtempSync(join(tmpdir(), "hit-list-"));
  const put = (rel, bytes) => {
    mkdirSync(dirname(join(repo, rel)), { recursive: true });
    writeFileSync(join(repo, rel), bytes);
  };
  put("crates/audio/src/crowd.rs", `pub enum FightCue {\n${cues.map((c) => `    /// doc\n    ${c} {\n        /// slot\n        a: u8,\n    },`).join("\n")}\n}\n`);
  for (const p of PICKS) put(`content/audio/${p.file}`, wav());
  const files = PICKS.map((p) => ({ license: "CC0", path: `sounds/${p.file}` }));
  put("content/audio/engine2040-vol0-manifest.json", JSON.stringify({ files }));
  return repo;
}

test("the enum is read from the audio crate", () => {
  assert.deepEqual(fightCues(fixture()), CUES);
});

test("a clean fixture passes and every row carries URL, SPDX, author and SHA", () => {
  const repo = fixture();
  assert.deepEqual(checkHitList(repo), []);
  for (const r of pickRows(repo)) assert.ok(r.url.includes("://") && r.spdx === "CC0-1.0" && r.author && /^[0-9a-f]{64}$/.test(r.digest.sha256));
  assert.match(hitListBody(repo), /StaminaOut`: silent/);
});

test("a new cue with no sound decision fails, and so does a pick for a cue that does not exist", () => {
  assert.match(checkHitList(fixture([...CUES, "Stagger"])).join("\n"), /Stagger: no pick and not marked silent/);
  assert.match(checkHitList(fixture(CUES.filter((c) => c !== "Parried"))).join("\n"), /Parried: not a FightCue variant/);
});

test("a missing file, a non-WAV and a file the pack does not call CC0 are named", () => {
  const repo = fixture();
  writeFileSync(join(repo, "content/audio", PICKS[0].file), "not a wav at all");
  writeFileSync(join(repo, "content/audio", PICKS[1].file), "");
  writeFileSync(join(repo, "content/audio/engine2040-vol0-manifest.json"), JSON.stringify({ files: PICKS.slice(1).map((p) => ({ license: p.file === PICKS[2].file ? "CC-BY-4.0" : "CC0", path: `sounds/${p.file}` })) }));
  const out = checkHitList(repo).join("\n");
  assert.match(out, /impacts\/flesh-small\.wav: not a RIFF\/WAVE file/);
  assert.match(out, /impacts\/flesh-small\.wav: the Vol0 manifest does not mark it CC0/);
  assert.match(out, /impacts\/flesh-large\.wav: the Vol0 manifest does not mark it CC0/);
  assert.match(out, /impacts\/flesh-medium\.wav: not a RIFF\/WAVE file/);
});

test("real tree: every FightCue is covered, files are CC0 WAVs, the table is current", () => {
  assert.deepEqual(fightCues(), CUES);
  assert.deepEqual(checkHitList(), []);
  assert.deepEqual(syncBlocks(hitListBlocks()), []);
});
