// part-score fixtures: each line from fixture inputs, a missing source
// prints UNKNOWN, the round line from a fixture git log.
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  lookLine,
  bodiesLine,
  aidoorLine,
  netLine,
  speedLine,
  buildLines,
  countByPrefix,
  parseBoardCounts,
  finishGate,
  decideFrameMoved,
  decideBreakDigest,
  countTestAttrs,
  decideSpatialScore,
  countVerbsInSources,
  countDoorTools,
  decideDet1,
  decideLoopback,
  perfP95,
  parseGitRecords,
  landedCommits,
  hasResultLine,
  countCapKills,
  countStealsLanded,
  clipDelivered,
  countToolsLanded,
  roundVerdict,
  roundLine,
} from "./part-score.mjs";

const BOARD = [
  "- [ ] VS1 open row",
  "- [x] BX1 done row",
  "- [ ] BX2 open row",
  "- [x] EV1 done row",
  "- [x] MP1 done row",
  "- [ ] OPT1 open row",
  "",
].join("\n");

describe("board counts", () => {
  it("counts one prefix from fixture rows", () => {
    const c = parseBoardCounts(BOARD);
    assert.equal(c.bx.done, 1);
    assert.equal(c.bx.total, 2);
    assert.equal(c.vs.total, 1);
  });
  it("countByPrefix is empty without that prefix", () => {
    const { tasks, order } = { tasks: new Map(), order: [] };
    assert.deepEqual(countByPrefix(tasks, order, /^VS/), { done: 0, total: 0 });
  });
});

describe("look line", () => {
  it("prints Q1 open/total, VS ticked, MOVED and the A4 gate", () => {
    const line = lookLine({ a4: { open: 1, total: 14, status: "partial" }, vs: { done: 0, total: 9 }, moved: "none" });
    assert.equal(line, "look: Q1 1/14 | VS ticked 0/9 | frame MOVED none | A4 gate partial");
  });
  it("missing finish prints UNKNOWN, never a guess", () => {
    const line = lookLine({ a4: null, vs: null, moved: "none" });
    assert.match(line, /Q1 UNKNOWN/);
    assert.match(line, /A4 gate UNKNOWN/);
  });
  it("finishGate reads the A4 row", () => {
    const g = finishGate([{ gate: "A4", status: "partial", openRows: ["Q1"], rows: new Array(14).fill("x") }], "A4");
    assert.deepEqual(g, { open: 1, total: 14, status: "partial" });
  });
  it("frame MOVED takes the biggest named percent, else none", () => {
    assert.equal(decideFrameMoved({ lookMd: "FRAME: cubes | MOVED: none" }), "none");
    assert.equal(decideFrameMoved({ lookMd: "MOVED: pixel 3.5%" }), "pixel 3.5%");
    assert.equal(decideFrameMoved({ frameMovedJson: '{"moved_pct": 12.5}' }), "pixel 12.5%");
  });
});

describe("bodies line", () => {
  it("prints BX ticked, the corpus digest and the test count", () => {
    assert.equal(
      bodiesLine({ bx: { done: 3, total: 17 }, digest: "none", tests: 376 }),
      "bodies: BX ticked 3/17 | break corpus BX28 none | anim and ragdoll tests 376",
    );
  });
  it("a corpus file digest wins over none", () => {
    assert.equal(decideBreakDigest(["nothing here", "suite_digest=a4cf750ab31e87fb"]), "a4cf750ab31e87fb");
    assert.equal(decideBreakDigest([null, "no hex"]), "none");
  });
  it("counts #[test] outside comments", () => {
    assert.equal(countTestAttrs([["a.rs", "#[test]\nfn a() {}\n// #[test]\n"]]), 1);
  });
});

describe("aidoor line", () => {
  it("prints EV passing, the spatial score, verbs and door tools", () => {
    assert.equal(
      aidoorLine({ ev: { done: 13, total: 17 }, spatial: "25", verbs: 2, tools: 21 }),
      "aidoor: EV tasks passing 13/17 | spatial eval score 25 | verbs per system 2 | door tools 21",
    );
  });
  it("a missing spatial source prints UNKNOWN", () => {
    assert.equal(decideSpatialScore([null, "unrelated"]), "UNKNOWN");
    assert.equal(decideSpatialScore(["spatial eval score: 87.5 (mock)"]), "87.5");
    assert.equal(countDoorTools(null), "UNKNOWN");
    assert.equal(countDoorTools('name: "a",\nname: "b",\n'), 2);
    assert.equal(countVerbsInSources([["t.rs", "crate::e2_tool!(\n// crate::e2_tool!(\n"]]), 1);
  });
});

describe("net line", () => {
  it("prints MP ticked, DET1 and the loopback bit", () => {
    assert.equal(netLine({ mp: { done: 5, total: 23 }, det1: "none", loopback: "UNKNOWN" }), "net: MP ticked 5/23 | DET1 sync test none | loopback hash equal UNKNOWN");
  });
  it("DET1 is green only on a PASS file", () => {
    assert.equal(decideDet1([null]), "none");
    assert.equal(decideDet1(["RESULT PASS"]), "green");
  });
  it("loopback is UNKNOWN without a comparison file", () => {
    assert.equal(decideLoopback([null]), "UNKNOWN");
    assert.equal(decideLoopback(['{"server_hash":"aa","client_hash":"aa"}']), "1");
    assert.equal(decideLoopback(["hash_equal: 0"]), "0");
  });
  it("loopback reads mp6 and on30 bout receipts", () => {
    assert.equal(decideLoopback(['{"lan":{"hash_x2_identical":true},"wan":{"hash_x2_identical":true}}']), "1");
    assert.equal(
      decideLoopback(['{"relay":{"hash_x2_identical":true,"bout":{"hash_x2_identical":true}},"direct":{"hash_x2_identical":true,"bout":{"hash_x2_identical":true}}}']),
      "1",
    );
    assert.equal(decideLoopback(['{"lan":{"hash_x2_identical":true},"wan":{"hash_x2_identical":false}}']), "0");
    assert.equal(decideLoopback(["hash_x2_identical: false"]), "0");
  });
});

describe("speed line", () => {
  it("prints p95s, the cold-check and OPT ticked", () => {
    assert.equal(
      speedLine({ cpu: "UNKNOWN", gpu: "UNKNOWN", cold: "0.9s", opt: { done: 5, total: 34 } }),
      "speed: CPU p95 UNKNOWN | GPU p95 UNKNOWN | lane cold-check 0.9s | OPT ticked 5/34",
    );
  });
  it("PENDING baselines print UNKNOWN", () => {
    assert.equal(perfP95('{"rows":[{"cpu_ms":null}]}', "cpu"), "UNKNOWN");
    assert.equal(perfP95('{"rows":[{"gpu_ms":{"p50":1,"p95":2.5,"p99":3}}]}', "gpu"), "2.5ms");
  });
});

describe("five lines together", () => {
  it("builds one line per front", () => {
    const lines = buildLines({
      a4: { open: 1, total: 14, status: "partial" },
      vs: { done: 0, total: 9 },
      moved: "none",
      bx: { done: 3, total: 17 },
      digest: "none",
      tests: 1,
      ev: { done: 1, total: 2 },
      mp: { done: 1, total: 2 },
      opt: { done: 0, total: 1 },
    });
    assert.equal(Object.keys(lines).join(","), "look,bodies,aidoor,net,speed");
    assert.match(lines.look, /^look: /);
  });
});

describe("round line on a fixture git log", () => {
  const raw = [
    "abc123CQ: hygiene sweepbody",
    "def456VS2 textured drawsMOVED: pixel 4%",
    "789abcfmt: tidybody",
  ].join("");
  it("counts only non-hygiene commits as landed", () => {
    const landed = landedCommits(parseGitRecords(raw));
    assert.equal(landed.length, 1);
    assert.equal(landed[0].subject, "VS2 textured draws");
  });
  it("a MOVED commit passes the round", () => {
    const line = roundLine({ landed: 1, owners: 2, movedPct: "4%", kills: 0, steals: 0, clip: "0", tools: 1 });
    assert.equal(line, "ROUND: landed 1 | owners reported 2/5 | frame MOVED 4% | cap kills 0 | steals landed 0 | clip delivered 0 | tools landed 1");
    assert.equal(roundVerdict({ movedCommit: true }).line, "RESULT: PASS - something real was made");
  });
  it("nothing real fails the round", () => {
    assert.equal(roundVerdict({}).line, "RESULT: FAIL - nothing real was made");
  });
  it("counts cap kills, steals, clips, tools and RESULT lines", () => {
    assert.equal(countCapKills("2026-10-04T00:01:00Z batch over helper_max_min stopped\n2026-10-03T00:01:00Z batch over helper_max_min stopped\n", Date.parse("2026-10-04T00:00:00Z")), 1);
    assert.equal(countStealsLanded("2026-10-04 | bar | donor | MIT | file | landed abc\n2026-10-03 | bar | donor | MIT | file | open\n", Date.parse("2026-10-04T00:00:00Z")), 1);
    assert.equal(clipDelivered("id,product,status,date\n1,weekly clip,delivered,2026-10-04\n", Date.parse("2026-10-04T00:00:00Z")), "1");
    assert.equal(clipDelivered("id,product,status,date\n1,weekly clip,open,2026-10-04\n", 0), "0");
    assert.equal(countToolsLanded(["tools/foo.mjs", "tools/foo.test.mjs", "tools/bar.test.mjs"], (s) => s === "foo.test.mjs"), 1);
    assert.equal(hasResultLine("noise\nRESULT: DONE - x | proof: y\n"), true);
    assert.equal(hasResultLine("no result here"), false);
  });
});

describe("board counts read the archive", () => {
  it("a row moved to docs/archive keeps its done and total counts", () => {
    const full = "- [x] VS1 a done row. needs: none.\n- [ ] VS2 open row. needs: none.\n";
    const shardOnly = "- [ ] VS2 open row. needs: none.\n";
    const archived = new Map([["VS1", "- [x] VS1 a done row. needs: none."]]);
    assert.deepEqual(parseBoardCounts(shardOnly, archived).vs, { done: 1, total: 2 });
    assert.deepEqual(parseBoardCounts(shardOnly, archived).vs, parseBoardCounts(full).vs);
  });
});
