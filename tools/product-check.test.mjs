import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { crc32, deflateSync } from "node:zlib";
import { sandFrames } from "./iqa-gate.mjs";
import { crashName, failLines, lastGreenRun, lookScore, middleCapture, newestFirst, parseReceipt, parseView, passFloors, pruneRuns, redFloors, referenceInterval, scoreCaptures, summary } from "./product-check.mjs";

const RECEIPT = `arena smoke: ticks=300 frames=300
arena frame-split: sim=0.085ms present=117.519ms fighters=2.932ms per frame (mean over 300 frames) cpu_ms p50=7.501 p99=407.215 interval_ms p50=59.597 p99=407.881 gpu_ms p50=0.244 p99=4.156 (forward p50=0.135 blit p50=0.109; 299 frames) acquire=0.068ms present_mode=Fifo
arena framing: frames=300 empty=0 first_empty=- fewest_in_view=2 steepest_down_deg=21.1 tinted_share=0.83
arena view: frames=300 sim_ticks=2.1..153.8 bodies=2400 offscreen_share=0.70 nonfinite=0 error_parts=0 sunk_cm_max=12.0 idle_float_cm_max=0.4 pops=35 pop_acts=attack:11/idle:24 pop_cm_max=102.7 eye_in_body=0 eye_below_ground=0 closest_cm=107 cover_max=0.03
arena per-pass: adapter="RTX" frames=300 sky=0.000ms layers=0.000ms ibl=0.412ms fog=0.000ms gtao=0.000ms validation_errors=0`;
const BAR = { floors: { framing_empty: 0, tinted_share_max: 0.25, stall_p99_ms_max: 1500, fps_drop_max: 0.3 } };
const quietCaps = { captures: 5, blank: 0, frozen: 0 };

test("the receipt lines parse into numbers", () => {
  const r = parseReceipt(RECEIPT);
  assert.equal(r.interval_p50_ms, 59.597);
  assert.equal(r.interval_p99_ms, 407.881);
  assert.equal(r.gpu_p50_ms, 0.244);
  assert.equal(r.empty, 0);
  assert.equal(r.fewest_in_view, 2);
  assert.equal(r.tinted_share, 0.83);
  assert.equal(r.validation_errors, 0);
  assert.equal(r.passes.ibl, 0.412);
  assert.equal(r.panics, 0);
  assert.equal(parseReceipt("nothing").framing, false);
  assert.equal(r.view.sim_first, 2.1);
  assert.equal(r.view.sim_last, 153.8);
  assert.equal(r.view.pops, 35);
  assert.equal(r.view.pop_acts, "attack:11/idle:24");
  assert.equal(r.view.sunk_cm_max, 12);
  assert.equal(r.view.closest_cm, 107);
  assert.equal(parseView(null), null);
});

test("view truth floors: NaN, error material, camera in a body or under the sand, a deep sink and a fight played unseen", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false };
  assert.deepEqual(redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR), {}, "today's numbers are green; pops are a target, not a floor");
  const bad = { ...look.view, nonfinite: 3, error_parts: 1, eye_in_body: 2, eye_below_ground: 1, sunk_cm_max: 31 };
  const red = redFloors({ look: { ...look, view: bad }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(Object.keys(red).sort(), ["camera_in_body", "camera_under_ground", "error_material", "nan", "sunk"]);
  // The window opened minimized: 300 ticks ran before the first frame, every frame showed the aftermath.
  const unseen = redFloors({ look: { ...look, view: { ...look.view, sim_first: 299.5, sim_last: 299.8 } }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.match(unseen.unseen_fight, /299.5..299.8/);
  const silent = redFloors({ look: { ...look, view: null }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(Object.keys(silent), ["view_receipt"]);
});

test("a red strobe and an empty frame break floors; a green run breaks none", () => {
  const look = { ...parseReceipt(RECEIPT), exit: 0, timedOut: false };
  const red = redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(Object.keys(red), ["strobe"]);
  const green = redFloors({ look: { ...look, tinted_share: 0.05 }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(green, {});
  const empty = redFloors({ look: { ...look, tinted_share: 0, empty: 7, exit: 1, framingFailed: true }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(Object.keys(empty), ["framing"], "a --require-framing exit is the framing floor, not a second exit floor");
});

test("a crashed run names the fault instead of showing a bare status code", () => {
  assert.equal(crashName(3221225725), "a stack overflow (0xC00000FD)");
  assert.equal(crashName(3221225477), "an access violation (0xC0000005)");
  assert.equal(crashName(2), null);
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 3221225725, timedOut: false };
  const red = redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.match(red.exit, /the look run died with a stack overflow/);
  const timing = { ...parseReceipt(RECEIPT), exit: 3221225725, loaded: 0 };
  const redTiming = redFloors({ look: { ...look, exit: 0 }, timing, caps: quietCaps, ref: null }, BAR);
  assert.match(redTiming.exit, /the timing run died with a stack overflow/);
});

test("fps is gated only on a quiet run, against the median of recent quiet runs", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0 };
  assert.equal(parseReceipt(RECEIPT).vsync, "Fifo");
  const history = [50, 60, 55, 400, 58].map((ms, i) => ({ timing: { interval_p50_ms: ms, loaded: i === 3 ? 2 : 0 } }));
  assert.equal(referenceInterval(history), 58);
  // An Immediate run at 8.3 ms and a Fifo run at the 16.7 ms display period are
  // different caps: the reference keeps to the run's own present mode.
  const mixed = [
    { timing: { interval_p50_ms: 8.3, loaded: 0, vsync: "Immediate" } },
    { timing: { interval_p50_ms: 16.6, loaded: 0, vsync: "Fifo" } },
    { timing: { interval_p50_ms: 16.8, loaded: 0, vsync: "Fifo" } },
    { timing: { interval_p50_ms: 16.7, loaded: 0, vsync: "Fifo" } },
  ];
  assert.equal(referenceInterval(mixed, "Fifo"), 16.7);
  assert.equal(referenceInterval(mixed, "Immediate"), 8.3);
  assert.equal(referenceInterval(mixed, "Mailbox"), null);
  assert.equal(referenceInterval([{ timing: { interval_p50_ms: 8.3, loaded: 0, vsync: "Immediate" } }], "Fifo"), null);
  assert.equal(redFloors({ look, timing: { interval_p50_ms: 16.7, interval_p99_ms: 26, loaded: 0, vsync: "Fifo" }, caps: quietCaps, ref: null }, BAR).slower, undefined);
  const slow = { interval_p50_ms: 100, interval_p99_ms: 300, loaded: 0 };
  assert.ok(redFloors({ look, timing: slow, caps: quietCaps, ref: 58 }, BAR).slower);
  assert.equal(redFloors({ look, timing: { ...slow, loaded: 3 }, caps: quietCaps, ref: 58 }, BAR).slower, undefined);
  assert.equal(redFloors({ look, timing: { ...slow, interval_p50_ms: 70 }, caps: quietCaps, ref: 58 }, BAR).slower, undefined);
});

test("a run that scored no captures is RED, not a silent green", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false };
  const red = redFloors({ look, timing: null, caps: { captures: 0, blank: 0, frozen: 0 }, ref: null }, BAR);
  assert.deepEqual(Object.keys(red), ["captures"]);
  assert.match(red.captures, /no captures/);
  // The same numbers with captures present are green: the floor is the empty dir, not a weaker bar.
  assert.deepEqual(redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR), {});
});

test("blank and frozen captures are counted from the files", () => {
  const dir = mkdtempSync(join(tmpdir(), "product-"));
  const big = Buffer.alloc(80_000, 7);
  const files = ["a", "b", "c", "d"].map((n) => join(dir, `${n}.png`));
  writeFileSync(files[0], big);
  writeFileSync(files[1], big); // same bytes as the one before: frozen
  writeFileSync(files[2], Buffer.alloc(80_000, 9));
  writeFileSync(files[3], Buffer.alloc(1_000, 1)); // tiny: blank
  assert.deepEqual(scoreCaptures(files), { captures: 4, blank: 1, frozen: 1 });
});

test("GS3 pass floors: a required pass with no ms is red and printed as a FAIL line", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false };
  const passBar = { passes: { bloom: "required" } };
  const red = redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR, passBar);
  assert.ok(red["pass-bloom"], "bloom is required but absent from the canned log");
  assert.match(failLines(red)[0], /^FAIL pass-bloom: the bloom pass reported no ms/);
});

test("GS3 pass floors: every required pass above 0 ms is green; pending never turns red", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false };
  const greenBar = { passes: { ibl: "required", bloom: "pending", sky: "pending" } };
  const greenLook = { ...look, passes: { ...look.passes, bloom: 0 } };
  assert.deepEqual(passFloors(greenLook.passes, { passes: { bloom: "pending" } }), {}, "pending bloom at 0 ms never turns red");
  const green = redFloors({ look: { ...look, passes: { ...look.passes, bloom: 1.2 } }, timing: null, caps: quietCaps, ref: null }, BAR, greenBar);
  assert.deepEqual(green, {}, "ibl and bloom above 0 ms are green");
  assert.deepEqual(passFloors({}, { passes: { sky: "pending" } }), {}, "a missing pending pass is ignored");
});

// LF15's consumer: the machine look score reads this run's middle capture
// against the last green run's, with the FLIP metric iqa-gate.mjs mirrors.
const pngOf = (w, h, data) => {
  const raw = Buffer.alloc(h * (w * 4 + 1));
  for (let y = 0; y < h; y++) data.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc32(td) >>> 0); return Buffer.concat([l, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
};

test("the look score reads the middle capture against the last green run's", () => {
  assert.equal(middleCapture(["frame_000000.png", "frame_000060.png", "frame_000120.png"]), "frame_000060.png");
  assert.equal(middleCapture(["a.png", "b.png"]), "a.png");
  assert.equal(middleCapture([]), null);
  const history = [{ sha: "aaa1111", red: { strobe: "x" } }, { sha: "bbb2222", red: {} }, { sha: "ccc3333", red: {} }];
  assert.deepEqual(lastGreenRun(history, ["aaa1111-2026", "bbb2222-2026"]), { sha: "bbb2222", dir: "bbb2222-2026" }, "the newest green run whose captures are on disk wins");
  assert.equal(lastGreenRun(history, ["aaa1111-2026"]), null, "a red run is never the reference");
  assert.equal(lastGreenRun([{ sha: "ddd4444", red: {} }], []), null, "a green run whose captures were pruned is skipped");
  const { ref, cap, w, h } = sandFrames();
  const prior = mkdtempSync(join(tmpdir(), "product-")), cur = mkdtempSync(join(tmpdir(), "product-"));
  writeFileSync(join(prior, "frame_000000.png"), pngOf(w, h, ref));
  writeFileSync(join(cur, "frame_000000.png"), pngOf(w, h, cap));
  const s = lookScore(prior, cur);
  assert.equal(s.flip, 1268, "the same PNG pair the Rust and iqa-gate tests pin (LF15)");
  assert.equal(s.vs, prior);
  assert.equal(lookScore(prior, mkdtempSync(join(tmpdir(), "product-"))).why, "this run has no captures");
  assert.equal(lookScore(null, cur).flip, null);
  assert.match(lookScore(null, cur).why, /no green run/);
  assert.equal(lookScore(prior, prior).flip, 0, "a run compared with itself reads 0");
});

test("the look score never throws: broken captures read as none with the reason", () => {
  const prior = mkdtempSync(join(tmpdir(), "product-")), cur = mkdtempSync(join(tmpdir(), "product-"));
  assert.match(lookScore(prior, cur).why, /this run has no captures/);
  writeFileSync(join(cur, "frame_000000.png"), Buffer.from("not a png"));
  assert.match(lookScore(prior, cur).why, /the last green run holds no captures/);
  writeFileSync(join(prior, "frame_000000.png"), Buffer.from("not a png"));
  assert.match(lookScore(prior, cur).why, /capture decode failed/);
  assert.equal(lookScore(prior, cur).flip, null);
  assert.match(lookScore(join(prior, "gone"), cur).why, /the last green run holds no captures/, "a reference dir that vanished reads as none");
  assert.equal(lookScore(prior, join(cur, "gone")).why, "this run has no captures");
});

test("the kept runs are the six newest by their UTC stamp, not by sha", () => {
  assert.deepEqual(newestFirst(["ffffffff-20261015T1242Z", "00000000-20261012T2223Z", "aaaaaaaa-20261013T0335Z"]), ["ffffffff-20261015T1242Z", "aaaaaaaa-20261013T0335Z", "00000000-20261012T2223Z"]);
});

test("the look score's reference run survives pruning", () => {
  const root = mkdtempSync(join(tmpdir(), "product-"));
  const dirs = ["aaaa1111-20261001T0000Z", "bbbb2222-20261002T0000Z", "cccc3333-20261003T0000Z", "dddd4444-20261004T0000Z", "eeee5555-20261005T0000Z", "ffff6666-20261006T0000Z", "gggg7777-20261007T0000Z", "hhhh8888-20261008T0000Z"];
  for (const d of dirs) mkdirSync(join(root, d));
  pruneRuns(root, "aaaa1111-20261001T0000Z");
  assert.deepEqual(readdirSync(root).sort(), ["aaaa1111-20261001T0000Z", "cccc3333-20261003T0000Z", "dddd4444-20261004T0000Z", "eeee5555-20261005T0000Z", "ffff6666-20261006T0000Z", "gggg7777-20261007T0000Z", "hhhh8888-20261008T0000Z"].sort(), "the six newest plus the pinned reference");
});

test("a look score above the floor is red and printed as a FAIL line", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false, flip: 1500, flip_max: 8000, flip_vs: "abc1234" };
  const red = redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(Object.keys(red), ["look"]);
  assert.match(red.look, /1500 x10000 against the last green run abc1234 \(floor 1200\)/);
  assert.match(failLines(red)[0], /^FAIL look: the machine look score read 1500 x10000/);
  const worstGreen = redFloors({ look: { ...look, flip: 609 }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(worstGreen, {}, "the worst green-vs-green drift observed (609 between two trees) stays green");
  const none = redFloors({ look: { ...look, flip: null, flip_vs: null, flip_why: "no green run to compare against" }, timing: null, caps: quietCaps, ref: null }, BAR);
  assert.deepEqual(none, {}, "no comparable run is not a red");
});

test("a broken floor shows as a FAIL line in the output and in latest.md", () => {
  const red = { strobe: "83% of drawn bodies wore the red hit tint (floor 25%)" };
  assert.deepEqual(failLines(red), ["FAIL strobe: 83% of drawn bodies wore the red hit tint (floor 25%)"]);
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0.83, exit: 0, timedOut: false, flip: null, flip_why: "no green run to compare against" };
  const base = { sha: "abc1234", at: "2026-10-21T00:00:00.000Z", build: "fresh", look, caps: quietCaps, timing: null, ref: null, runDir: "target/reports/product/runs/x", bar: BAR, passBar: null };
  const broken = summary({ ...base, red });
  assert.match(broken, /^Verdict: RED \(strobe\)/m);
  assert.match(broken, /^- FAIL strobe: 83% of drawn bodies wore the red hit tint \(floor 25%\)$/m);
  assert.match(summary({ ...base, red: {} }), /^Verdict: GREEN/m);
});
