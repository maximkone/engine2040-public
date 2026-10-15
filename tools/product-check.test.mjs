import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { packetText, parseReceipt, parseView, passFloors, redFloors, referenceInterval, scoreCaptures } from "./product-check.mjs";

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

test("fps is gated only on a quiet run, against the median of recent quiet runs", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0 };
  const history = [50, 60, 55, 400, 58].map((ms, i) => ({ timing: { interval_p50_ms: ms, loaded: i === 3 ? 2 : 0 } }));
  assert.equal(referenceInterval(history), 58);
  const slow = { interval_p50_ms: 100, interval_p99_ms: 300, loaded: 0 };
  assert.ok(redFloors({ look, timing: slow, caps: quietCaps, ref: 58 }, BAR).slower);
  assert.equal(redFloors({ look, timing: { ...slow, loaded: 3 }, caps: quietCaps, ref: 58 }, BAR).slower, undefined);
  assert.equal(redFloors({ look, timing: { ...slow, interval_p50_ms: 70 }, caps: quietCaps, ref: 58 }, BAR).slower, undefined);
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

test("the bug packet names its row, suspects, proof and the last green tree", () => {  const text = packetText({ metric: "strobe", why: "83% of drawn bodies wore the red hit tint", sha: "abc1234", runDir: "target/reports/product/runs/x", history: [{ sha: "0ld0000", red: {} }, { sha: "bad0000", red: { strobe: "x" } }] });
  assert.match(text, /^row: bug:product-strobe$/m);
  assert.match(text, /^role: builder$/m);
  assert.match(text, /^chain: start$/m);
  assert.match(text, /^paths: crates\/game\/src\/fighter_look\.rs$/m);
  assert.match(text, /--features app/);
  assert.match(text, /0ld0000\.\.HEAD/);
  for (const head of ["Goal:", "Scope:", "Proof:", "Stop:"]) assert.ok(text.includes(`\n${head}`), head);
});

test("GS3 pass floors: a required pass with no ms is red and names bug:product-pass-<name>", () => {
  const look = { ...parseReceipt(RECEIPT), tinted_share: 0, exit: 0, timedOut: false };
  const passBar = { passes: { bloom: "required" } };
  const red = redFloors({ look, timing: null, caps: quietCaps, ref: null }, BAR, passBar);
  assert.ok(red["pass-bloom"], "bloom is required but absent from the canned log");
  const text = packetText({ metric: "pass-bloom", why: red["pass-bloom"], sha: "abc1234", runDir: "target/reports/product/runs/x", history: [] });
  assert.match(text, /^row: bug:product-pass-bloom$/m);
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
