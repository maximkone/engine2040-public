import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import {
  checkQ1, blessQ1, knownFailures, stillsOf, sha256Hex,
  EXPECTED_Q1, SYNTH_REL, ORBIT_REL, GOLDEN_REL, KNOWN_FAILURES_FILE,
} from "./q1-check.mjs";

// The live target/q1 tree is untracked local captures, absent on a fresh
// machine, so every case below builds its own fixture root instead.
function makeFixture({ synth = 4, orbit = 30, manifests = true, ra8 = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "q1-"));
  const synthDir = join(root, SYNTH_REL);
  const orbitDir = join(root, ORBIT_REL);
  mkdirSync(synthDir, { recursive: true });
  mkdirSync(orbitDir, { recursive: true });
  for (let i = 0; i < synth; i++) writeFileSync(join(synthDir, "synth_" + i + ".png"), "");
  for (let i = 0; i < orbit; i++) writeFileSync(join(orbitDir, "orbit_" + i + ".png"), "");
  if (manifests) {
    const body = ra8 ? "ra8 reference-gpu look manifest" : "cpu staged stills manifest";
    writeFileSync(join(synthDir, "manifest.json"), body);
    writeFileSync(join(orbitDir, "manifest.json"), body);
  }
  return root;
}

test("q1 fixture pins the expected capture counts", () => {
  const r = checkQ1(makeFixture());
  assert.equal(r.synthPng, 4, "synth png count " + r.synthPng);
  assert.equal(r.orbitPng, 30, "orbit png count " + r.orbitPng);
  assert.equal(r.totalPng, 34, "total png count " + r.totalPng);
  assert.equal(r.manifests, 2, "manifest count " + r.manifests);
  assert.equal(r.ra8Manifests, 0, "RA8 manifests pinned as gap, got " + r.ra8Manifests);
  assert.deepEqual(r.gaps, ["skipped: RA8 reference-GPU look manifest absent"]);
  assert.equal(EXPECTED_Q1.synthPng, 4);
  assert.equal(EXPECTED_Q1.orbitPng, 30);
  assert.equal(EXPECTED_Q1.ra8Manifests, 0);
});

test("missing capture dirs warn and report zeros, never throw", () => {
  const r = checkQ1(mkdtempSync(join(tmpdir(), "q1-empty-")));
  assert.equal(r.synthPng, 0);
  assert.equal(r.orbitPng, 0);
  assert.equal(r.manifests, 0);
  assert.ok(r.gaps.length > 0, "gaps name the missing captures");
});

test("an ra8 manifest counts toward the ra8 pin", () => {
  const r = checkQ1(makeFixture({ ra8: true }));
  assert.equal(r.ra8Manifests, 2);
  assert.ok(r.gaps.some((g) => g === "ra8 2 != 0"), "gaps were " + JSON.stringify(r.gaps));
});

// The bless flow: goldens under `golden/`, one tree per case. Bytes are the
// gate here (tier 1); the tool never decodes a PNG, so a still is any bytes.
const STILL_A = join(SYNTH_REL, "synth_0.png");
const STILL_B = join(ORBIT_REL, "orbit_7.png");
// Two same-length streams: a changed still differs in bytes, not in size.
const BODY_FILED = "filed-bytes";
const BODY_CANDIDATE = "moved-bytes";

function makeTree({ changed = [], goldens = true, known = null } = {}) {
  const root = makeFixture();
  const goldRoot = join(root, GOLDEN_REL);
  const stills = [...stillsOf(root)];
  assert.equal(stills.length, 34, "the fixture offers the Q1 suite of stills");
  const names = new Set(stills.map((s) => s.still));
  for (const still of [...changed, ...(known || [])]) {
    assert.ok(names.has(still), "the fixture holds " + still);
  }
  for (const { still } of stills) {
    // The capture holds the candidate, the golden holds what was filed; a
    // `changed` still is one where the two streams part.
    writeFileSync(join(root, still), changed.includes(still) ? BODY_CANDIDATE : BODY_FILED);
    if (!goldens) continue;
    const path = join(goldRoot, still);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, BODY_FILED);
  }
  if (known) {
    mkdirSync(goldRoot, { recursive: true });
    writeFileSync(join(goldRoot, KNOWN_FAILURES_FILE), JSON.stringify(known));
  }
  return { root, goldRoot };
}

test("bless flow compares the 34 stills and passes when the goldens match", () => {
  const { root } = makeTree();
  const r = blessQ1({ root });
  assert.deepEqual(
    [r.stills, r.compared, r.matched, r.changed.length, r.missing.length],
    [34, 34, 34, 0, 0],
    "every filed golden compared and matched",
  );
  assert.deepEqual(r.gaps, []);
  assert.deepEqual(r.wrote, [], "a match writes no receipt");
});

test("bless flow reports a missing golden and writes nothing for it", () => {
  const { root, goldRoot } = makeTree();
  rmSync(join(goldRoot, STILL_A));
  const r = blessQ1({ root });
  assert.deepEqual(r.missing, [STILL_A]);
  assert.deepEqual(r.gaps, ["missing golden: " + STILL_A]);
  assert.equal(existsSync(join(goldRoot, STILL_A)), false, "a refusal files no golden");
  assert.equal(r.compared, 33);
});

test("bless flow writes new, old and diff receipts and edits no golden", () => {
  const { root, goldRoot } = makeTree({ changed: [STILL_B] });
  const r = blessQ1({ root });
  assert.deepEqual(r.changed, [STILL_B]);
  assert.ok(r.gaps.some((g) => g === "golden differs: " + STILL_B), "gaps were " + JSON.stringify(r.gaps));
  const stem = STILL_B.replace(/\.png$/, "");
  assert.equal(readFileSync(join(goldRoot, stem + ".new.png"), "utf8"), BODY_CANDIDATE);
  assert.equal(readFileSync(join(goldRoot, stem + ".old.png"), "utf8"), BODY_FILED, "the golden is kept as .old");
  assert.equal(readFileSync(join(goldRoot, STILL_B), "utf8"), BODY_FILED, "the golden itself is untouched");
  const receipt = JSON.parse(readFileSync(join(goldRoot, stem + ".diff.json"), "utf8"));
  assert.equal(receipt.firstDiffByte, 0, "the two streams part at once");
  assert.equal(receipt.lengthsEqual, true, "same length, different bytes");
  assert.match(receipt.capture.sha256, /^[0-9a-f]{64}$/, "the candidate is pinned by sha256");
  // The receipts are never scanned as stills.
  assert.equal(blessQ1({ root }).stills, 34, "receipts are receipts, not stills");
});

test("bless flow promotes the candidate under bless and keeps the old golden", () => {
  const { root, goldRoot } = makeTree({ changed: [STILL_B] });
  const r = blessQ1({ root, bless: true });
  assert.deepEqual(r.blessed, [STILL_B]);
  assert.deepEqual(r.gaps, [], "a bless resolves the change it was asked to take");
  assert.equal(readFileSync(join(goldRoot, STILL_B), "utf8"), BODY_CANDIDATE, "the golden took the candidate");
  const stem = STILL_B.replace(/\.png$/, "");
  assert.equal(readFileSync(join(goldRoot, stem + ".old.png"), "utf8"), BODY_FILED, "the blessed change keeps .old");
  assert.deepEqual(blessQ1({ root }).gaps, [], "the same run twice is clean after the bless");
});

test("bless flow seeds a first golden only when blessing", () => {
  const { root, goldRoot } = makeTree({ goldens: false });
  const cold = blessQ1({ root });
  assert.equal(cold.missing.length, 34, "nothing filed yet");
  assert.equal(cold.compared, 0);
  const warm = blessQ1({ root, bless: true });
  assert.equal(warm.seeded.length, 34);
  assert.deepEqual(warm.gaps, []);
  assert.equal(readFileSync(join(goldRoot, STILL_A), "utf8"), readFileSync(join(root, STILL_A), "utf8"));
  assert.deepEqual(blessQ1({ root }).matched, 34, "the seeded set now compares");
});

test("bless flow forgives a known failure that still differs", () => {
  const { root } = makeTree({ changed: [STILL_B], known: [STILL_B] });
  const r = blessQ1({ root });
  assert.deepEqual(r.known, [STILL_B]);
  assert.deepEqual(r.changed, [], "the table carries it");
  assert.deepEqual(r.stale, []);
  assert.deepEqual(r.gaps, []);
});

test("bless flow refuses a stale known failure", () => {
  const { root } = makeTree({ known: [STILL_A] });
  const r = blessQ1({ root });
  assert.deepEqual(r.stale, [STILL_A]);
  assert.ok(r.gaps.some((g) => g === "stale known failure: " + STILL_A), "gaps were " + JSON.stringify(r.gaps));
  const blessed = blessQ1({ root, bless: true });
  assert.deepEqual(blessed.stale, [STILL_A], "a bless never prunes the table");
});

test("a known-failure table that is not a list of names is a gap", () => {
  const { root, goldRoot } = makeTree();
  writeFileSync(join(goldRoot, KNOWN_FAILURES_FILE), "not json");
  assert.match(knownFailures(root).gaps[0], /is not readable JSON/);
  writeFileSync(join(goldRoot, KNOWN_FAILURES_FILE), JSON.stringify([7]));
  assert.match(knownFailures(root).gaps[0], /must hold a list of still names/);
  assert.deepEqual(knownFailures(mkdtempSync(join(tmpdir(), "q1-none-"))).names, [], "absent table is empty");
});
