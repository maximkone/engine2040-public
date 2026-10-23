// Run: node --test tools/coverage-gate.test.mjs
// TG4: the coverage floor. The gate reads one `cargo llvm-cov --json
// --summary-only` export and the committed bar, rolls each crate's files up to
// its package and exits 1 under a floor. The seeded summary here is 10% covered,
// so the gate cannot pass it by accident.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { compare, main, percent, rollup } from "./coverage-gate.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// One file per crate: 10 covered of 100 lines (10%), 60 of 100 (60%).
function seededSummary() {
  return {
    data: [
      {
        files: [
          { filename: "/w/crates/core/src/lib.rs", summary: { lines: { count: 100, covered: 10 } } },
          { filename: "/w/crates/proof/src/lib.rs", summary: { lines: { count: 100, covered: 60 } } },
          { filename: "C:\\w\\crates\\endpoint\\src\\mcp.rs", summary: { lines: { count: 50, covered: 50 } } },
        ],
      },
    ],
  };
}

const seededBar = {
  packages: {
    "engine-core": { dir: "crates/core", lines: 50 },
    "engine-proof": { dir: "crates/proof", lines: 50 },
    "engine-endpoint": { dir: "crates/endpoint", lines: 50 },
  },
};

test("rollup sums line counts per crate directory", () => {
  const out = rollup(seededSummary());
  assert.deepEqual(out.get("crates/core"), { covered: 10, count: 100 });
  assert.deepEqual(out.get("crates/proof"), { covered: 60, count: 100 });
  // A Windows path with backslashes is the same crate as the forward-slash one.
  assert.deepEqual(out.get("crates/endpoint"), { covered: 50, count: 50 });
});

test("percent is one decimal and null for an empty crate", () => {
  assert.equal(percent(10, 100), 10);
  assert.equal(percent(2, 3), 66.6);
  assert.equal(percent(0, 0), null);
});

test("a 10% crate is under a 50% floor; a 60% crate holds", () => {
  const { rows, reds, ok } = compare(seededBar, rollup(seededSummary()));
  assert.equal(ok, false);
  const core = rows.find((r) => r.package === "engine-core");
  assert.equal(core.now, 10);
  assert.equal(core.ok, false);
  assert.deepEqual(reds.map((r) => r.package), ["engine-core"]);
  const proof = rows.find((r) => r.package === "engine-proof");
  assert.equal(proof.now, 60);
  assert.equal(proof.ok, true);
});

test("a crate exactly on the floor holds, half a point under is red", () => {
  const onFloor = { packages: { "engine-core": { dir: "crates/core", lines: 10 } } };
  assert.equal(compare(onFloor, rollup(seededSummary())).ok, true);
  const above = { packages: { "engine-core": { dir: "crates/core", lines: 10.1 } } };
  assert.equal(compare(above, rollup(seededSummary())).ok, false);
});

test("a package the run never touched is red, not skipped", () => {
  const bar = { packages: { "engine-absent": { dir: "crates/absent", lines: 1 } } };
  const { rows, reds } = compare(bar, rollup(seededSummary()));
  assert.equal(rows[0].now, null);
  assert.deepEqual(reds.map((r) => r.package), ["engine-absent"]);
});

test("a measured crate with no floor is printed, never red", () => {
  const bar = { packages: { "engine-core": { dir: "crates/core", lines: 10 } } };
  const { ok, reds, others } = compare(bar, rollup(seededSummary()));
  assert.equal(ok, true);
  assert.deepEqual(reds, []);
  assert.deepEqual(others, [
    { dir: "crates/endpoint", now: 100 },
    { dir: "crates/proof", now: 60 },
  ]);
});

test("main exits 1 on a 10% summary and 0 once the crates hold the floor", () => {
  const dir = mkdtempSync(join(tmpdir(), "coverage-gate-"));
  try {
    const summary = join(dir, "summary.json");
    writeFileSync(summary, JSON.stringify(seededSummary()));
    const bar = join(dir, "bar.json");
    writeFileSync(bar, JSON.stringify(seededBar));
    const lines = [];
    assert.equal(main(["--summary", summary, "--bar", bar], (l) => lines.push(l)), 1);
    assert.ok(lines.some((l) => l.includes("RED engine-core")), lines.join("\n"));
    writeFileSync(bar, JSON.stringify({ packages: { "engine-core": { dir: "crates/core", lines: 10 } } }));
    const ok = [];
    assert.equal(main(["--summary", summary, "--bar", bar], (l) => ok.push(l)), 0);
    assert.ok(ok.some((l) => l.includes("coverage-gate: 1 packages, 0 under the floor")), ok.join("\n"));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("main exits 2 on a missing summary and on bad arguments", () => {
  assert.equal(main(["--summary", join(tmpdir(), "no-such-coverage.json")], () => {}), 2);
  assert.equal(main(["--nope"], () => {}), 2);
});

test("the committed bar names measured floors for the row's crates", () => {
  const bar = JSON.parse(readFileSync(join(ROOT, "tools", "coverage-bar.json"), "utf8"));
  const names = Object.keys(bar.packages);
  assert.ok(names.length > 0, "the bar gates at least one crate");
  for (const name of ["engine-core", "engine-proof"]) assert.ok(names.includes(name), `${name} has a floor`);
  for (const name of names) {
    const row = bar.packages[name];
    assert.match(row.dir, /^crates\/[a-z]+$/, `${name} names its dir`);
    assert.equal(typeof row.lines, "number", `${name} has a measured floor`);
    assert.equal(typeof row.measured, "number", `${name} keeps the run's own number`);
    assert.ok(row.lines > 0 && row.lines <= row.measured, `${name} floor sits at or under the measurement`);
    assert.ok(row.lines <= 100, `${name} floor is a percentage`);
  }
  assert.match(bar._how, /cargo llvm-cov/, "the bar names the command that printed it");
  assert.match(bar._measured, /^\d{4}-\d{2}-\d{2}$/, "the bar is dated");
});
