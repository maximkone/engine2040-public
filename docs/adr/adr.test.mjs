// Run: node --test docs/adr/adr.test.mjs
// CQ22: every decision record has a status and a Compliance check whose paths exist,
// the README index lists every record, and the policy records pin the lines they stand on.
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const ADR = join(ROOT, "docs", "adr");
const read = (...p) => readFileSync(join(ROOT, ...p), "utf8").replace(/\r\n/g, "\n");
const records = readdirSync(ADR).filter((f) => /^0\d+-.*\.md$/.test(f)).sort();

// Repo paths in a Compliance line: backticked, no spaces, with a slash; a `::` or `#` suffix is dropped.
function compliancePaths(line) {
  return [...line.matchAll(/`([^`\s]+\/[^`\s]*)`/g)].map((m) => m[1].split(/::|#/)[0]);
}

test("there are at least nine records", () => {
  assert.ok(records.length >= 9, `found ${records.length}`);
});

test("each record names a status and a Compliance check whose paths exist", () => {
  for (const f of records) {
    const text = read("docs", "adr", f);
    assert.match(text, /^- Status: (Proposed|Accepted|Superseded)\b/m, `${f} has no valid Status`);
    const line = /^- Compliance: (.+)$/m.exec(text);
    assert.ok(line, `${f} has no Compliance line`);
    assert.doesNotMatch(line[1], /\bnone yet\b/i, `${f} still says none yet`);
    const paths = compliancePaths(line[1]);
    assert.ok(paths.length > 0, `${f} Compliance names no path`);
    for (const p of paths) assert.ok(existsSync(join(ROOT, p)), `${f} Compliance path ${p} is missing`);
  }
});

test("the README index lists every record with its status", () => {
  const index = read("docs", "adr", "README.md");
  for (const f of records) {
    const id = f.slice(0, 4);
    const status = /^- Status: (\w+)/m.exec(read("docs", "adr", f))[1];
    assert.match(index, new RegExp(`^\\| ${id} \\| .+ \\| ${status} \\|$`, "m"), `${id} row missing or wrong status`);
  }
});

test("0005 the app feature leaves engine-arena/sim off", () => {
  const app = /^app = \[(.*)\]$/m.exec(read("crates", "game", "Cargo.toml"));
  assert.ok(app, "crates/game/Cargo.toml has no app feature line");
  assert.ok(!app[1].includes("engine-arena/sim"), "app enables engine-arena/sim: supersede ADR 0005 first");
});

test("0007 no Cargo.toml names a bevy dependency", () => {
  const manifests = ["Cargo.toml", ...readdirSync(join(ROOT, "crates")).map((c) => join("crates", c, "Cargo.toml"))];
  for (const m of manifests.filter((p) => existsSync(join(ROOT, p)))) {
    const live = read(m).split("\n").filter((l) => !l.trimStart().startsWith("#")).join("\n");
    assert.doesNotMatch(live, /^\s*(\[[\w.-]*dependencies\.bevy[\w-]*\]|bevy[\w-]*\s*[=.])/m, `${m} names bevy`);
  }
});

test("0008 FOCUS keeps the D7 freeze line", () => {
  assert.match(read("docs", "FOCUS.md"), /^- D7 Frozen until the first real frame/m);
});

test("0009 donor rule stays in FOCUS, AGENTS and deny.toml scope", () => {
  assert.match(read("docs", "FOCUS.md"), /^- D5 .*Any donor code may be ported; `deny\.toml` covers Cargo dependencies only/m);
  assert.match(read("AGENTS.md"), /`deny\.toml` governs Cargo dependency licenses only\. Donor code from any source may be ported/);
  assert.match(read("deny.toml"), /Source of truth for dependency licenses/);
});

test("0011 FOCUS keeps audio on the parked line", () => {
  assert.match(read("docs", "FOCUS.md"), /^Parked \(rows stay[^)]*\)[^\n]*\baudio\b/m);
});
