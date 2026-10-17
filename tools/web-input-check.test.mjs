// node --test tools/web-input-check.test.mjs: the pure parts of the WEB7 input and audio check.
import test from "node:test";
import assert from "node:assert/strict";
import { clickCommands, keyCommands, parseArgs, parseAudio, parseInput, verdict } from "./web-input-check.mjs";

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
});
