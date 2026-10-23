// web-perf drills: the gate prints the two arms, passes when the replay's CPU fell with the same
// picture, and fails on a rise, a different picture or a different draw count.
//   node --test tools/web-perf.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PerfError, formatLine, main, parseArgs, perfOf, receiptOfCapture, verdict } from "./web-perf.mjs";

// A page receipt carrying a healthy A/B: the same 14 draws and the same frozen picture, and the
// replay's CPU median 0.014 ms under the rebuild's.
function perf(over = {}) {
  const off = { frames: 60, cpuP50Ms: 0.412, cpuP95Ms: 0.628, cpuMaxMs: 1.204, listP50Us: 7.2, draws: 14, ...over.off };
  const on = { frames: 60, cpuP50Ms: 0.398, cpuP95Ms: 0.610, cpuMaxMs: 1.180, listP50Us: 0.9, draws: 14, ...over.on };
  return {
    mode: "ab",
    warmup: 45,
    block: 20,
    off,
    on,
    deltaCpuMs: over.deltaCpuMs ?? Number((off.cpuP50Ms - on.cpuP50Ms).toFixed(3)),
    deltaListUs: over.deltaListUs ?? Number((off.listP50Us - on.listP50Us).toFixed(1)),
    drawsEqual: over.drawsEqual ?? off.draws === on.draws,
    pixels: { off: "0123456789abcdef", on: "0123456789abcdef", equal: true, ...over.pixels },
  };
}

// A capture file as web-browse.mjs writes it: the page's receipt text and the facts around it.
function captureFile(dir, receipt) {
  const path = join(dir, "arena-capture.json");
  const text = JSON.stringify({ capture: "ready", receiptText: JSON.stringify(receipt), receipt });
  writeFileSync(path, text);
  return path;
}

const run = (argv) => {
  const out = [], err = [];
  const code = main(argv, (l) => out.push(l), (l) => err.push(l));
  return { code, out, err };
};

const withDir = (fn) => {
  const dir = mkdtempSync(join(tmpdir(), "web-perf-"));
  try { return fn(dir); } finally { rmSync(dir, { recursive: true, force: true }); }
};

test("a healthy A/B prints both arms and passes", () => {
  withDir((dir) => {
    const r = run(["--capture", captureFile(dir, { ok: true, perf: perf() })]);
    assert.equal(r.code, 0, r.err.join("\n"));
    assert.equal(r.err.length, 0);
    assert.match(r.out[0], /^web-perf: mode=ab block=20 off_frames=60 on_frames=60 draws=14\/14 /);
    assert.match(r.out[0], /off_cpu_p50=0\.412 on_cpu_p50=0\.398 delta_cpu=0\.014ms /);
    assert.match(r.out[0], /list_off=7\.2us list_on=0\.9us pixels=EQUAL$/);
  });
});

test("a CPU that did not fall fails the row's own gate", () => {
  withDir((dir) => {
    const r = run(["--capture", captureFile(dir, { perf: perf({ on: { cpuP50Ms: 0.44 }, deltaCpuMs: -0.028 }) })]);
    assert.equal(r.code, 1);
    assert.match(r.err.join("\n"), /no win: the on arm's cpu p50 0\.440 ms is not below the off arm's 0\.412 ms/);
  });
});

test("a different picture fails even when the CPU fell", () => {
  withDir((dir) => {
    const r = run(["--capture", captureFile(dir, { perf: perf({ pixels: { on: "fedcba9876543210", equal: false } }) })]);
    assert.equal(r.code, 1);
    assert.match(r.err.join("\n"), /the two captures differ \(off 0123456789abcdef, on fedcba9876543210\)/);
  });
});

test("a different draw count fails", () => {
  withDir((dir) => {
    const r = run(["--capture", captureFile(dir, { perf: perf({ on: { draws: 13 } }) })]);
    assert.equal(r.code, 1);
    assert.match(r.err.join("\n"), /different draw counts \(off 14, on 13\)/);
  });
});

test("a short arm fails", () => {
  withDir((dir) => {
    const r = run(["--capture", captureFile(dir, { perf: perf({ on: { frames: 9 } }) })]);
    assert.equal(r.code, 1);
    assert.match(r.err.join("\n"), /the on arm measured 9 frames, fewer than 20/);
  });
});

test("a page that did not run the A/B is a setup failure", () => {
  withDir((dir) => {
    const plain = run(["--capture", captureFile(dir, { perf: { mode: "on", arm: { frames: 60, draws: 14, cpuP50Ms: 0.4, cpuP95Ms: 0.6, listP50Us: 7, cpuMaxMs: 1 } } })]);
    assert.equal(plain.code, 2);
    assert.match(plain.err.join("\n"), /no pixels object/);
    const missing = run(["--capture", join(dir, "nothing.json")]);
    assert.equal(missing.code, 2);
    assert.match(missing.err.join("\n"), /does not exist/);
  });
});

test("a capture file that is not JSON is refused", () => {
  withDir((dir) => {
    const path = join(dir, "bad.json");
    writeFileSync(path, "not json");
    assert.throws(() => receiptOfCapture("nope"), PerfError);
    const r = run(["--capture", path]);
    assert.equal(r.code, 2);
    assert.match(r.err.join("\n"), /the capture file is not JSON/);
  });
});

test("the line is one line and names the verdict", () => {
  const line = formatLine(perfOf({ perf: perf() }));
  assert.equal(line.split("\n").length, 1);
  assert.match(line, /pixels=EQUAL/);
});

test("parseArgs defaults to the workflow's capture path and refuses an unknown flag", () => {
  assert.match(parseArgs([]).capture, /arena-capture\.json$/);
  assert.match(parseArgs(["--capture", "x.json"]).capture, /x\.json$/);
  assert.match(parseArgs(["--nope"]).error, /unknown argument --nope/);
  assert.match(parseArgs(["--capture"]).error, /--capture needs a value/);
});
