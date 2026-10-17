// Run: node --test skills/land-gate/SKILL.test.mjs
// Guards the cites in this skill: every `target/queue/` packet the Test note and
// tests/pairs.md run must exist, and the commands must print the verdicts the note
// promises. The queue prunes done/ records, so a cite can die (2026-10-17:
// 0-cq10-time-checked-20261014 and 0-arena-shake-watch-20261014 were gone and
// 0-dp1-loopback-knobs never reached done/): move the cite in both files, never
// leave a dead one.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Packets the `land.mjs <id> --dry` lines run. A prune or a rename moves the cite.
const DRY_ID = /land\.mjs ([A-Za-z0-9._-]+) --dry/g;

// Live paths the note hands to packet-gate/land (`skills/SKILL.md` is the skill
// file a packet owns, a placeholder, so it is not a tree path). The two `git diff
// -- <file> | packet-gate --patch -` lines need a real diff on disk, so their
// verdicts are not asserted here; only the paths they name are.
const LIVE_PATHS = ["skills/SKILLS.md", "crates/core/src/time.rs", "tools/packet-gate.mjs", "tools/land.mjs"];

const PACKET_CITE = /target\/queue\/[A-Za-z0-9_./-]+\.md/g;

function text(rel) {
  return readFileSync(join(HERE, rel), "utf8").replace(/\r\n/g, "\n");
}

function run(args) {
  try {
    return { code: 0, out: execFileSync(process.execPath, args, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }) };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
}

test("every cited target/queue packet exists in the tree", () => {
  const loose = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(PACKET_CITE)) {
      const cite = m[0];
      if (!existsSync(join(ROOT, cite))) loose.push(`${rel}: ${cite}`);
    }
  }
  assert.deepEqual(loose, [], `cited packet is gone: move the cite in this skill: ${loose.join(" | ")}`);
});

test("the note's live paths exist and stay cited", () => {
  const skill = text("SKILL.md");
  for (const rel of LIVE_PATHS) {
    assert.ok(skill.includes(rel), `SKILL.md no longer cites ${rel}`);
    assert.ok(existsSync(join(ROOT, rel)), `${rel} is gone: move this skill's cite`);
  }
});

test("every `land.mjs <id> --dry` id in this skill has a done/ record", () => {
  const loose = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(DRY_ID)) {
      const record = `target/queue/done/${m[1]}.md`;
      if (!existsSync(join(ROOT, record))) loose.push(`${rel}: ${m[1]} (${record})`);
    }
  }
  assert.deepEqual(loose, [], `land --dry id has no record: move the cite in this skill: ${loose.join(" | ")}`);
});

test("packet-gate fails a record with no RESULT line", () => {
  const gate = run(["tools/packet-gate.mjs", "--result", "target/queue/done/builder-fixer-r161.md"]);
  assert.notEqual(gate.code, 0, "a FAIL exits 1");
  assert.match(gate.out, /RESULT FAIL/, "the audit prints RESULT FAIL");
  assert.match(gate.out, /no RESULT line/, "the finding names the missing RESULT line");
});

test("land refuses a no-RESULT record, a runner role and a missing review", () => {
  const noResult = run(["tools/land.mjs", "builder-fixer-r161", "--dry"]);
  assert.notEqual(noResult.code, 0, "a REFUSED exits 1");
  assert.match(noResult.out, /REFUSED builder-fixer-r161: the run ended with no RESULT line, not DONE, PARTIAL or NOOP/);

  const runner = run(["tools/land.mjs", "runner-proof-r304", "--dry"]);
  assert.notEqual(runner.code, 0, "a REFUSED exits 1");
  assert.match(runner.out, /REFUSED runner-proof-r304: role runner lands nothing/);

  const noReview = run(["tools/land.mjs", "builder-integrator-r72", "--dry"]);
  assert.notEqual(noReview.code, 0, "a REFUSED exits 1");
  assert.match(noReview.out, /REFUSED builder-integrator-r72: no review record for builder-integrator-r72 in done\//);
});
