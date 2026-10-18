// node --test tools/web-input-check.test.mjs: the pure parts of the WEB7 input and audio check.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { clickCommands, keyCommands, parseArgs, parseAudio, parseInput, parseScript, parseTape, stepCommands, tapeVerdict, verdict } from "./web-input-check.mjs";

const read = (kb, mouse, audio, context, gestures = 0) => ({ input: `input: kb=${kb} mouse=${mouse} pad=0`, audio: `audio: state=${audio}`, context, gestures });
const good = () => ({
  before: read(0, 0, "suspended", "none"),
  escape: read(2, 0, "suspended", "none"),
  keys: read(4, 0, "suspended", "created", 1),
  click: read(4, 2, "running", "created", 2),
});

test("the two page lines parse only in their exact shape", () => {
  assert.deepEqual(parseInput("input: kb=3 mouse=2 pad=0"), { kb: 3, mouse: 2, pad: 0 });
  assert.equal(parseInput("input: kb=3 mouse=2"), null);
  assert.equal(parseInput("input: kb=x mouse=2 pad=0"), null);
  assert.equal(parseInput(null), null);
  assert.equal(parseAudio("audio: state=running"), "running");
  assert.equal(parseAudio("audio: state=suspended"), "suspended");
  assert.equal(parseAudio("audio: state=closed"), null);
  assert.equal(parseAudio("audio: running"), null);
});

test("a run that follows the contract has no problems", () => {
  assert.deepEqual(verdict(good()), []);
});

test("an audio context before any gesture is named", () => {
  const r = good();
  r.before.context = "created";
  assert.match(verdict(r).join(";"), /AudioContext exists before any gesture/);
});

test("Escape starting the audio is named, so is a key press that does not", () => {
  let r = good();
  r.escape.context = "created";
  assert.match(verdict(r).join(";"), /Escape started the audio context/);
  r = good();
  r.keys.context = "none";
  assert.match(verdict(r).join(";"), /key press did not create/);
});

test("counts that do not move are named", () => {
  let r = good();
  r.keys.input = "input: kb=2 mouse=0 pad=0";
  assert.match(verdict(r).join(";"), /kb to 4/);
  r = good();
  r.click.input = "input: kb=4 mouse=0 pad=0";
  assert.match(verdict(r).join(";"), /at least 2 mouse events/);
  r = good();
  r.before.input = "input: kb=1 mouse=0 pad=0";
  assert.match(verdict(r).join(";"), /counts must be 0/);
});

test("audio that never runs is named", () => {
  const r = good();
  r.click.audio = "audio: state=suspended";
  assert.match(verdict(r).join(";"), /audio must be running/);
});

test("a malformed line or a missing read is reported before anything is compared", () => {
  const r = good();
  r.escape.input = "kb=2";
  assert.match(verdict(r).join(";"), /escape: #input is not/);
  assert.match(verdict({ ...good(), click: null }).join(";"), /click: no read/);
});

test("key and click commands are real CDP input calls in order", () => {
  const w = keyCommands("KeyW");
  assert.deepEqual(w.map((c) => c.params.type), ["keyDown", "keyUp"]);
  assert.equal(w[0].params.code, "KeyW");
  assert.equal(w[0].params.text, "w");
  const esc = keyCommands("Escape");
  assert.deepEqual(esc.map((c) => c.params.type), ["rawKeyDown", "keyUp"]);
  assert.throws(() => keyCommands("KeyZ"), /no key table row/);
  const c = clickCommands(10, 20);
  assert.deepEqual(c.map((x) => x.params.type), ["mouseMoved", "mousePressed", "mouseReleased"]);
  assert.ok(c.every((x) => x.method === "Input.dispatchMouseEvent" && x.params.x === 10 && x.params.y === 20));
});

test("parseArgs needs a Chrome and refuses unknown flags", () => {
  assert.equal(parseArgs([]).error, "--chrome <path to Chrome> is required");
  assert.equal(parseArgs(["--chrome", "c", "--wat"]).error, "unknown argument --wat");
  const ok = parseArgs(["--chrome", "c", "--dir", "d", "--out", "o"]);
  assert.deepEqual([ok.chrome, ok.dir, ok.out, ok.error], ["c", "d", "o", null]);
  assert.equal(ok.script, "tools/fixtures/input-tape.script");
  const t = parseArgs(["--chrome", "c", "--script", "s", "--native-tape", "n"]);
  assert.deepEqual([t.script, t.nativeTape], ["s", "n"]);
});

// ---- the input tape (WEB7) ----
const SCRIPT = "# fight\nkey KeyW down\ntick\nmouse 2 down\nkey KeyF down # attack\ntick\nmouse 2 up\nkey KeyF up\nkey KeyW up\ntick\n";

test("a script reads like the Rust parser does: commands, comments, a named bad line", () => {
  const steps = parseScript(SCRIPT);
  assert.deepEqual(steps.map((s) => s.type), ["key", "tick", "mouse", "key", "tick", "mouse", "key", "key", "tick"]);
  assert.deepEqual(steps[0], { type: "key", code: "KeyW", down: true });
  assert.deepEqual(steps[2], { type: "mouse", button: 2, down: true });
  assert.deepEqual(parseScript("\n# nothing\n"), []);
  for (const bad of ["jump", "key KeyW", "key KeyW sideways", "mouse left down", "tick now", "key KeyW down up"]) {
    assert.throws(() => parseScript(`tick\n${bad}\n`), /script line 2:/, bad);
  }
  // The committed script is valid and has ticks.
  const real = parseScript(readFileSync(new URL("./fixtures/input-tape.script", import.meta.url), "utf8"));
  assert.ok(real.filter((s) => s.type === "tick").length >= 4);
  assert.ok(real.some((s) => s.type === "mouse") && real.some((s) => s.type === "key"));
});

test("steps become single CDP calls; the held mouse buttons add up and a tick sends nothing", () => {
  const held = { mask: 0 };
  assert.deepEqual(stepCommands({ type: "tick" }, held), []);
  const [down] = stepCommands({ type: "key", code: "KeyF", down: true });
  assert.equal(down.params.type, "keyDown");
  assert.equal(down.params.code, "KeyF");
  const [up] = stepCommands({ type: "key", code: "KeyF", down: false });
  assert.equal(up.params.type, "keyUp");
  const [shift] = stepCommands({ type: "key", code: "ShiftLeft", down: true });
  assert.equal(shift.params.type, "rawKeyDown");
  assert.equal(shift.params.modifiers, 8);
  const [l] = stepCommands({ type: "mouse", button: 0, down: true }, held);
  assert.deepEqual([l.params.type, l.params.button, l.params.buttons], ["mousePressed", "left", 1]);
  const [r] = stepCommands({ type: "mouse", button: 2, down: true }, held);
  assert.deepEqual([r.params.button, r.params.buttons], ["right", 3]);
  const [rup] = stepCommands({ type: "mouse", button: 2, down: false }, held);
  assert.deepEqual([rup.params.type, rup.params.buttons], ["mouseReleased", 1]);
  assert.throws(() => stepCommands({ type: "mouse", button: 9, down: true }, held), /no mouse button 9/);
  assert.throws(() => stepCommands({ type: "key", code: "KeyZ", down: true }), /no key table row/);
});

const TAPE = "input-tape: ticks=2 bytes=00000000006400000000010000000064000000 01";
const goodTape = (bytes = "0000000000640000000001000000006400000001") => `input-tape: ticks=2 bytes=${bytes}`;

test("a tape line parses with its digest, takes the last line of a log and refuses a bad shape", () => {
  const t = parseTape(`noise\n${goodTape()}\n`);
  assert.equal(t.ticks, 2);
  assert.equal(t.bytes.length, 40);
  assert.match(t.digest, /^[0-9a-f]{16}$/);
  assert.equal(parseTape(TAPE), null, "a space inside the hex is not a tape");
  assert.equal(parseTape("input-tape: ticks=1 bytes=abc"), null, "odd hex");
  assert.equal(parseTape(null), null);
  assert.equal(parseTape("input-tape: ticks=0 bytes=").digest, "cbf29ce484222325", "the empty tape is the FNV offset");
});

test("tapeVerdict: equal bytes and digest pass, any difference is named", () => {
  const page = parseTape(goodTape());
  assert.deepEqual(tapeVerdict(page, parseTape(goodTape()), 2), []);
  assert.deepEqual(tapeVerdict(page, null, 2), [], "nothing to compare with is not a problem here; the caller says NOT COMPARED");
  const other = parseTape(goodTape("0000000000640000000001000000006400000003"));
  const named = tapeVerdict(page, other, 2).join(" | ");
  assert.match(named, /tape bytes differ/);
  assert.match(named, /digest: page /);
  assert.match(tapeVerdict(page, parseTape("input-tape: ticks=1 bytes=00000000006400000000"), 2).join(), /ticks: page 2, native 1/);
  assert.match(tapeVerdict(page, null, 3).join(), /sampled 2 ticks, the script has 3/);
  assert.match(tapeVerdict(null, page)[0], /no input-tape line/);
  assert.match(tapeVerdict({ ticks: 2, bytes: "00", digest: "x" }, null).join(), /1 bytes for 2 ticks/);
});
