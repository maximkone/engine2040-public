import assert from "node:assert/strict";
import test from "node:test";
import { ALLOW, donorInfo, failedByTeam, formatLine, gateLines, nextFor, parseSteals, partOf, verdictOf } from "./steal-land.mjs";

const DAY = 86_400_000;
const NOW = Date.parse("2026-10-04T00:00:00Z");
const noGit = { nowMs: NOW, hasCommitSince: () => true, commitTouches: () => true };

const open = (n, extra = {}) => ({
  id: n, date: "2026-10-03", bar: "A8 (GM3)", donor: `o/r@abc123${n} (branch main)`,
  license: "MIT", our: "tools/ship.mjs", size: "M", status: "open",
  sha: null, promise: "what to change, and the number it moves: 0 -> 1.", line: n, ...extra,
});

test("unsized old lines parse as M; sized lines keep their size", () => {
  const md = ["# t", "", "## Steals", "",
    "2026-10-03 | A8 (GM3) | forge log_triage (our-repo tool) | ours | tools/ship.mjs | open",
    "  what to change, and the number it moves: 0 -> 1.",
    "2026-10-04 | ON40 | gschup/ggrs@e97e3d2:src/input_queue.rs (branch main) | UNVERIFIED | crates/net/src/netcode.rs | S | open",
    "  what to change, and the number it moves: unbounded -> bounded.",
  ].join("\n");
  const lines = parseSteals(md);
  assert.equal(lines.length, 2);
  assert.equal(lines[0].size, "M");
  assert.equal(lines[1].size, "S");
  assert.equal(lines[0].promise, "what to change, and the number it moves: 0 -> 1.");
});

test("a 4th open line per part FAILs; the first three pass", () => {
  const lines = [open(1), open(2), open(3), open(4)];
  const res = gateLines(lines, noGit);
  assert.deepEqual(res.map((r) => r.ok), [true, true, true, false]);
  assert.match(res[3].reasons.join(";"), /4th open line for part tools/);
});

test("a 9-day-old open line with no commit since FAILs; a fresh commit saves it", () => {
  const old = open(1, { date: "2026-09-25" }); // 9 days before NOW
  const stale = gateLines([old], { nowMs: NOW, hasCommitSince: () => false, commitTouches: () => true });
  assert.equal(stale[0].ok, false);
  assert.match(stale[0].reasons.join(";"), /no commit to tools\/ship\.mjs since 2026-09-25/);
  const fresh = gateLines([old], { nowMs: NOW, hasCommitSince: () => true, commitTouches: () => true });
  assert.equal(fresh[0].ok, true);
  const young = gateLines([open(1, { date: "2026-10-03" })], { nowMs: NOW, hasCommitSince: () => false, commitTouches: () => true });
  assert.equal(young[0].ok, true, "1 day old: youth covers a quiet file");
});

test("a landed line whose commit touches our/file passes; a miss FAILs", () => {
  const done = { ...open(1), status: "landed", sha: "abc1234" };
  const hit = gateLines([done], { nowMs: NOW, hasCommitSince: () => true, commitTouches: () => true });
  assert.equal(hit[0].ok, true);
  const miss = gateLines([done], { nowMs: NOW, hasCommitSince: () => true, commitTouches: () => false });
  assert.equal(miss[0].ok, false);
  assert.match(miss[0].reasons.join(";"), /landed abc1234 touches no tools\/ship\.mjs/);
});

test("off-allowlist donors print PORT (owner 2026-10-10: any donor may be ported); the allowlist passes through", () => {
  assert.equal(verdictOf("UNVERIFIED"), "PORT");
  assert.equal(verdictOf("GPL-3.0-only"), "PORT");
  assert.equal(verdictOf("no-license"), "PORT");
  for (const a of ALLOW) assert.equal(verdictOf(a), a);
  assert.equal(verdictOf("ours"), "OURS");
  const line = open(1, { license: "UNVERIFIED", donor: "mystery/box@abc1234 (branch main)" });
  assert.match(formatLine(line), /PORT/);
});

test("donor cells become pinned file URLs; our-repo tools have no fetch", () => {
  const pinned = donorInfo("bevyengine/bevy@844390b279c6ef252d10df2a17912761325db7f2:crates/bevy_anti_alias/src/taa/mod.rs (branch main, read live 2026-10-03)");
  assert.equal(pinned.url, "https://github.com/bevyengine/bevy/blob/844390b279c6ef252d10df2a17912761325db7f2/crates/bevy_anti_alias/src/taa/mod.rs");
  const bare = donorInfo("tauri-apps/tauri@ece2c876 (blob SHA, branch dev, read live 2026-10-03)");
  assert.equal(bare.url, "https://github.com/tauri-apps/tauri/tree/ece2c876");
  const ours = donorInfo("forge log_triage (our-repo tool, arsenal 2026-10-03)");
  assert.equal(ours.kind, "our-repo");
  assert.equal(ours.url, null);
});

test("--next tools prints the oldest no-cargo line and skips FAILED team marks", () => {
  const lines = [
    open(1, { our: "crates/render/src/gpu_cull.rs" }),
    open(2, { our: "tools/ship.mjs", date: "2026-10-02" }),
    open(3, { our: "tools/ship.mjs", date: "2026-10-03" }),
  ];
  const first = nextFor(lines, { part: "tools" });
  assert.equal(first.id, 2, "oldest no-cargo tools line first");
  assert.match(formatLine(first), /tools\/ship\.mjs/);
  const skipped = nextFor(lines, { part: "tools", teamMd: "2026-10-04: tools/ship.mjs attempt FAILED (keeper timeout), try another donor" });
  assert.equal(skipped, null, "every tools line names the FAILED file: nothing left");
  assert.equal(partOf("tools/ship.mjs"), "tools");
  assert.equal(partOf("crates/render/src/gpu_cull.rs"), "look");
  assert.equal(partOf("crates/assets/src/meshlet.rs"), "bodies");
  assert.equal(partOf("crates/net/src/netcode.rs"), "net");
  assert.ok(failedByTeam("FAILED: tauri pin dead", { ...open(1), donor: "tauri-apps/tauri@ece2c876 (branch dev)", bar: "A8 (GM3)", our: "tools/ship.mjs" }) === false, "an unnamed FAILED does not skip a stranger");
});

test("partOf routes every program-area crate to its seat part", () => {
  assert.equal(partOf("crates/physics/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/destruction/src/shape_match.rs"), "bodies");
  assert.equal(partOf("crates/combat/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/geo/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/proof/kani/"), "bodies");
  assert.equal(partOf("crates/nav/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/save/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/observe/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/playtest/src/lib.rs"), "bodies");
  assert.equal(partOf("crates/viewer/src/main.rs"), "look");
  assert.equal(partOf("crates/material/src/lib.rs"), "look");
  assert.equal(partOf("crates/world/src/terrain.rs"), "look");
  assert.equal(partOf("crates/rules/src/lib.rs"), "aidoor");
  assert.equal(partOf("crates/scene/src/lib.rs"), "aidoor");
  assert.equal(partOf("crates/editor/src/lib.rs"), "aidoor");
  assert.equal(partOf("crates/spatial/src/index3.rs"), "aidoor");
  assert.equal(partOf("crates/arena/src/lib.rs"), "speed");
  assert.equal(partOf("crates/ecs/src/lib.rs"), "speed");
  assert.equal(partOf("crates/core/src/lib.rs"), "speed");
  assert.equal(partOf("crates/app/src/lib.rs"), "speed");
  assert.equal(partOf("crates/game/src/main.rs"), "speed");
  assert.equal(partOf("crates/input/src/lib.rs"), "speed");
});

test("partOf never falls through to a shared crates part", () => {
  for (const c of ["world", "spatial", "save", "input", "app", "game", "nav", "observe", "playtest"]) {
    assert.notEqual(partOf(`crates/${c}/src/lib.rs`), "crates", `${c} must map to a seat part`);
  }
});

test("a landed sha with a trailing provenance comment still lands", () => {
  const md = ["# t", "", "## Steals", "",
    "2026-10-03 | A8 (GM3) | console-rs/indicatif@53cf51c6 (branch main) | MIT | tools/ship.mjs | landed ce21f2a (owner orphan-gold wave: wall-probe 13/13 on master; lane r3/r4 duplicates superseded)",
    "  what to change, and the number it moves: 0 projection aborts -> 1 per over-budget run.",
  ].join("\n");
  const lines = parseSteals(md);
  assert.equal(lines.length, 1);
  assert.equal(lines[0].status, "landed");
  assert.equal(lines[0].sha, "ce21f2a");
  const hit = gateLines(lines, noGit);
  assert.equal(hit[0].ok, true, "landed comment must not FAIL the gate");
  const bare = parseSteals(md.replace(/ \(owner.*$/, ""));
  assert.equal(bare[0].status, "landed");
  assert.equal(bare[0].sha, "ce21f2a");
});
