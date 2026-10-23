// Headless Chrome check of input and audio on dist/web (WEB7): serve it on 127.0.0.1, load it, send real (trusted) key and
// mouse events over CDP and read back the two lines the page prints.
//   node tools/web-input-check.mjs --dir dist/web --chrome google-chrome [--out out/web-input] [--script tools/fixtures/input-tape.script] [--native-tape <file>]
// What it proves, in one browser:
//  1. Before any gesture: `input: kb=0 mouse=0 pad=0`, `audio: state=suspended`, and no AudioContext exists
//     (body data-audio-context=none).
//  2. Escape (a key the browser does not count as a user activation) is counted by the input line but does not
//     start the audio context.
//  3. A real key press (W down and up) raises kb and creates the context inside the gesture.
//  4. A mouse click raises mouse and the page ends on `audio: state=running` within the wait.
//  5. The input tape (WEB7): the events of --script are sent as real key and mouse events, the page samples one record at each
//     `tick` line (window.inputSample), and the tape it gives must equal, byte for byte and in digest, the one the native
//     `input_tape` bin printed for the same script (--native-tape names its output). Without --native-tape the browser tape is
//     printed and the run says it was NOT COMPARED.
//  6. The AudioWorklet sink (A5): the receipt is `audio: state=running underruns=<n>`, `#audioworklet` says path=worklet, the ring
//     is primed with six blocks and keeps growing (the audio thread consumes), and the scripted fight's six cues start six
//     voices in the page's own mixer (window.audioCueCheck).
//  7. The forced fallback (A5): a second load at ?worklet=broken must print `worklet=fallen-back reason="<browser error>"`
//     with the missing module's 404, take path=script, and play the same six cues there.
// The pad count is not driven here (Chrome cannot fake a gamepad over CDP); crates/web/src/input.rs tests it.
// Only loopback is served and visited. The Chrome process started here is ours and is closed here.
// Exit: 0 proven, 1 a proof failed, 2 could not run (no Chrome, bad arguments, no page).
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { fnv1a64Hex } from "./fnv1a64.mjs";
import { BASE_FLAGS, FLAG_SETS, launchChrome, serve } from "./web-browse.mjs";

export const INPUT_LINE = /^input: kb=(\d+) mouse=(\d+) pad=(\d+)$/;
// A5: the receipt keeps `audio: state=<s> underruns=<n>`; a fallback adds `worklet=` and a quoted
// `reason=`. Fields may follow in any order (the page writes them in a fixed one).
export const AUDIO_LINE = /^audio: state=(suspended|running)((?: (?:worklet=[a-z-]+|reason="[^"]*"|underruns=\d+))*)$/;
export const PATH_LINE = /^audio: path=(worklet|script|none)((?: (?:blocks=\d+|reason="[^"]*"|underruns=\d+))*)$/;
export const CUE_LINE = /^audio: cues expected=(\d+) played=(\d+) culled=(\d+) rejected=(\d+)$/;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The numbers and words of the two page lines; null for a line that has the wrong shape.
export function parseInput(text) {
  const m = INPUT_LINE.exec(String(text ?? "").trim());
  return m ? { kb: Number(m[1]), mouse: Number(m[2]), pad: Number(m[3]) } : null;
}

// The fields of one `key=value` tail. `grammar` maps a field name to a pattern for its value;
// a field outside the grammar makes the whole line null (a typo must never pass as proof).
function parseFields(rest, grammar) {
  const out = {};
  let left = rest;
  while (left) {
    const m = /^ ([a-z]+)=("([^"]*)"|([^ ]+))/.exec(left);
    if (!m || !Object.hasOwn(grammar, m[1])) return null;
    const quoted = m[3] !== undefined;
    // Only a reason carries quotes; every other field is a bare word.
    if (quoted !== (m[1] === "reason")) return null;
    const value = quoted ? m[3] : m[4];
    if (!new RegExp(`^(?:${grammar[m[1]]})$`).test(value)) return null;
    out[m[1]] = value;
    left = left.slice(m[0].length);
  }
  return out;
}

// { state, worklet, reason, underruns } of the audio receipt, or null. `underruns` is null until
// the sink exists (the bare `audio: state=suspended` before the first gesture).
export function parseAudio(text) {
  const m = AUDIO_LINE.exec(String(text ?? "").trim());
  const fields = m && parseFields(m[2] ?? "", { worklet: "[a-z-]+", reason: '[^"]*', underruns: "\\d+" });
  if (!fields) return null;
  return {
    state: m[1],
    worklet: fields.worklet ?? null,
    reason: fields.reason ?? null,
    underruns: fields.underruns === undefined ? null : Number(fields.underruns),
  };
}

// { path, blocks, reason, underruns } of the `#audioworklet` line, or null.
export function parsePath(text) {
  const m = PATH_LINE.exec(String(text ?? "").trim());
  const fields = m && parseFields(m[2] ?? "", { blocks: "\\d+", reason: '[^"]*', underruns: "\\d+" });
  if (!fields) return null;
  return {
    path: m[1],
    blocks: fields.blocks === undefined ? null : Number(fields.blocks),
    reason: fields.reason ?? null,
    underruns: fields.underruns === undefined ? null : Number(fields.underruns),
  };
}

// { expected, played, culled, rejected } of the mixer's cue proof, or null.
export function parseCues(text) {
  const m = CUE_LINE.exec(String(text ?? "").trim());
  return m ? { expected: Number(m[1]), played: Number(m[2]), culled: Number(m[3]), rejected: Number(m[4]) } : null;
}

// CDP commands for one key press (down then up). `vk` is the Windows virtual key code Chrome wants.
export const KEYS = {
  Escape: { key: "Escape", vk: 27 },
  KeyW: { key: "w", vk: 87, text: "w" },
  KeyA: { key: "a", vk: 65, text: "a" },
  KeyS: { key: "s", vk: 83, text: "s" },
  KeyD: { key: "d", vk: 68, text: "d" },
  KeyE: { key: "e", vk: 69, text: "e" },
  KeyF: { key: "f", vk: 70, text: "f" },
  KeyQ: { key: "q", vk: 81, text: "q" },
  ShiftLeft: { key: "Shift", vk: 16, modifiers: 8, location: 1 },
  ControlLeft: { key: "Control", vk: 17, modifiers: 2, location: 1 },
  Space: { key: " ", vk: 32, text: " " },
  Enter: { key: "Enter", vk: 13, text: "\r" },
};
// One key event of the table: `down` true for the press, false for the release.
export function keyCommand(code, down) {
  const k = KEYS[code];
  if (!k) throw new Error(`no key table row for ${code}`);
  const base = { key: k.key, code, windowsVirtualKeyCode: k.vk, nativeVirtualKeyCode: k.vk, ...(k.location ? { location: k.location } : {}) };
  if (!down) return { method: "Input.dispatchKeyEvent", params: { type: "keyUp", ...base } };
  return { method: "Input.dispatchKeyEvent", params: { type: k.text ? "keyDown" : "rawKeyDown", ...base, ...(k.text ? { text: k.text } : {}), ...(k.modifiers ? { modifiers: k.modifiers } : {}) } };
}
export function keyCommands(code) {
  return [keyCommand(code, true), keyCommand(code, false)];
}
// A left click at (x, y): move, press, release.
export function clickCommands(x, y) {
  const p = { x, y, modifiers: 0, pointerType: "mouse" };
  return [
    { method: "Input.dispatchMouseEvent", params: { type: "mouseMoved", ...p } },
    { method: "Input.dispatchMouseEvent", params: { type: "mousePressed", ...p, button: "left", buttons: 1, clickCount: 1 } },
    { method: "Input.dispatchMouseEvent", params: { type: "mouseReleased", ...p, button: "left", buttons: 0, clickCount: 1 } },
  ];
}

// ---- the input tape (WEB7) ----
// A script is one command per line, # starts a comment: `key <code> down|up`, `mouse <button> down|up`, `tick`. The same file is read
// by crates/web/src/input_tape.rs (the input_tape bin), which also refuses anything else.
export function parseScript(text) {
  const steps = [];
  String(text).split(/\r?\n/).forEach((raw, i) => {
    const words = raw.split("#")[0].trim().split(/\s+/).filter(Boolean);
    if (!words.length) return;
    const fail = (why) => { throw new Error(`script line ${i + 1}: ${why}`); };
    const upDown = (w) => (w === "down" ? true : w === "up" ? false : fail(`expected down or up, got ${w}`));
    if (words[0] === "tick" && words.length === 1) steps.push({ type: "tick" });
    else if (words[0] === "key" && words.length === 3) steps.push({ type: "key", code: words[1], down: upDown(words[2]) });
    else if (words[0] === "mouse" && words.length === 3 && /^\d+$/.test(words[1])) steps.push({ type: "mouse", button: Number(words[1]), down: upDown(words[2]) });
    else fail(`not a command: ${words.join(" ")}`);
  });
  return steps;
}

const MOUSE_BUTTONS = [{ name: "left", bit: 1 }, { name: "middle", bit: 4 }, { name: "right", bit: 2 }];

// The CDP calls for one key or mouse step; `held` ({ mask }) is the mouse buttons down so far, updated here. A tick has none.
export function stepCommands(step, held = { mask: 0 }, x = 640, y = 360) {
  if (step.type === "key") return [keyCommand(step.code, step.down)];
  if (step.type !== "mouse") return [];
  const b = MOUSE_BUTTONS[step.button];
  if (!b) throw new Error(`no mouse button ${step.button}`);
  held.mask = step.down ? held.mask | b.bit : held.mask & ~b.bit;
  const p = { x, y, modifiers: 0, pointerType: "mouse", button: b.name, buttons: held.mask, clickCount: 1 };
  return [{ method: "Input.dispatchMouseEvent", params: { type: step.down ? "mousePressed" : "mouseReleased", ...p } }];
}

export const TAPE_LINE = /^input-tape: ticks=(\d+) bytes=([0-9a-f]*)$/;
// { ticks, bytes (hex), digest (FNV-1a 64 of the bytes) } of the last tape line in `text`, or null.
export function parseTape(text) {
  const line = String(text ?? "").split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith("input-tape:")).pop();
  const m = TAPE_LINE.exec(line ?? "");
  if (!m || m[2].length % 2) return null;
  return { ticks: Number(m[1]), bytes: m[2], digest: fnv1a64Hex(Buffer.from(m[2], "hex")) };
}
// The problems with the browser's tape against the native one (null native: nothing to compare with). Each record is 10 bytes.
export function tapeVerdict(browser, native, wantTicks = null) {
  const problems = [];
  if (!browser) return ["the page gave no input-tape line (window.inputSample)"];
  if (browser.bytes.length !== browser.ticks * 20) problems.push(`the page's tape has ${browser.bytes.length / 2} bytes for ${browser.ticks} ticks, not ${browser.ticks * 10}`);
  if (wantTicks !== null && browser.ticks !== wantTicks) problems.push(`the page sampled ${browser.ticks} ticks, the script has ${wantTicks}`);
  if (!native) return problems;
  if (native.ticks !== browser.ticks) problems.push(`ticks: page ${browser.ticks}, native ${native.ticks}`);
  if (native.bytes !== browser.bytes) problems.push(`tape bytes differ: page ${browser.bytes}, native ${native.bytes}`);
  if (native.digest !== browser.digest) problems.push(`digest: page ${browser.digest}, native ${native.digest}`);
  return problems;
}

// What a page read looks like: { input, audio, context, gestures } from #input, #audio and the body data attributes.
export const READ_JS = `(() => ({
  input: document.getElementById('input')?.textContent ?? null,
  audio: document.getElementById('audio')?.textContent ?? null,
  context: document.body.dataset.audioContext ?? null,
  gestures: Number(document.body.dataset.audioGestures ?? -1),
  state: document.body.dataset.state ?? null,
}))()`;

// A5: the sink receipt, the ring line, the cue proof the page holds, and whether listeners are installed.
export const AUDIO_JS = `(() => ({
  audio: document.getElementById('audio')?.textContent ?? null,
  path: document.getElementById('audioworklet')?.textContent ?? null,
  cues: document.body.dataset.audioCues ?? null,
  context: document.body.dataset.audioContext ?? null,
}))()`;

// The checks over the four reads (before, after Escape, after the key press, after the click). Returns the
// problems found; an empty list is a pass.
export function verdict({ before, escape, keys, click }) {
  const problems = [];
  const need = (cond, why) => { if (!cond) problems.push(why); };
  const reads = { before, escape, keys, click };
  for (const [name, r] of Object.entries(reads)) {
    if (!r) { problems.push(`${name}: no read`); continue; }
    need(parseInput(r.input), `${name}: #input is not "input: kb=<n> mouse=<n> pad=<n>": ${JSON.stringify(r.input)}`);
    need(parseAudio(r.audio), `${name}: #audio is not "audio: state=<suspended|running>": ${JSON.stringify(r.audio)}`);
  }
  if (problems.length) return problems;
  const i = (r) => parseInput(r.input);
  const a = (r) => parseAudio(r.audio).state;
  need(i(before).kb === 0 && i(before).mouse === 0 && i(before).pad === 0, `before any event the counts must be 0: ${before.input}`);
  need(a(before) === "suspended", `before a gesture the audio must be suspended: ${before.audio}`);
  need(before.context === "none", `an AudioContext exists before any gesture (data-audio-context=${before.context})`);
  need(i(escape).kb === 2, `Escape down and up must count 2 key events: ${escape.input}`);
  need(escape.context === "none", `Escape started the audio context (data-audio-context=${escape.context}); it is not a user activation`);
  need(a(escape) === "suspended", `audio must still be suspended after Escape: ${escape.audio}`);
  need(i(keys).kb === 4, `W down and up must bring kb to 4: ${keys.input}`);
  need(keys.context === "created", `the key press did not create the audio context (data-audio-context=${keys.context})`);
  need(i(click).mouse >= 2, `a click must count at least 2 mouse events (press, release): ${click.input}`);
  need(a(click) === "running", `after a key press and a click the audio must be running: ${click.audio}`);
  return problems;
}

// A5: the worklet path. The receipt must be the exact `audio: state=running underruns=<n>`, the ring
// line must name the worklet, the ring must be primed, and `later` (a second read about a second on)
// must show more blocks: the audio thread consumed what the mixer rendered.
export function workletVerdict({ receipt, later }) {
  const problems = [];
  const audio = parseAudio(receipt?.audio);
  if (!audio) return [`the worklet receipt is not an audio line: ${JSON.stringify(receipt?.audio)}`];
  if (audio.state !== "running") problems.push(`the worklet receipt is ${audio.state}, not running: ${receipt.audio}`);
  if (audio.worklet !== null) problems.push(`the page fell back instead of running the worklet: ${receipt.audio}`);
  if (audio.underruns === null) problems.push(`the worklet receipt has no counted underruns: ${receipt.audio}`);
  const path = parsePath(receipt.path);
  if (!path) return [...problems, `#audioworklet is not a path line: ${JSON.stringify(receipt?.path)}`];
  if (path.path !== "worklet") problems.push(`#audioworklet says path=${path.path}, not worklet: ${receipt.path}`);
  if (path.blocks === null || path.blocks < 6) problems.push(`the ring was not primed with the page's six blocks: ${receipt.path}`);
  if (path.underruns !== null && audio.underruns !== null && path.underruns !== audio.underruns) {
    problems.push(`the receipt and the path line disagree on underruns: ${audio.underruns} / ${path.underruns}`);
  }
  const grown = parsePath(later?.path);
  if (!grown || grown.blocks === null || path.blocks === null || grown.blocks <= path.blocks) {
    problems.push(`the worklet did not consume the ring: blocks ${path.blocks} -> ${grown?.blocks ?? "no later read"}`);
  }
  return problems;
}

// A5: the scripted fight through the page's own mixer: every queued cue started a voice, none was
// culled or refused. The numbers are the mixer's own counters, not the page's.
export function cueVerdict(line) {
  const p = parseCues(line);
  if (!p) return [`the cue check gave no counts: ${JSON.stringify(line)}`];
  const problems = [];
  if (p.expected !== 6) problems.push(`the script queues 6 cues, the mixer expected ${p.expected}: ${line}`);
  if (p.played !== p.expected) problems.push(`the mixer started ${p.played} of ${p.expected} cues: ${line}`);
  if (p.culled !== 0) problems.push(`${p.culled} scripted cues were culled: ${line}`);
  if (p.rejected !== 0) problems.push(`${p.rejected} scripted cues were rejected: ${line}`);
  return problems;
}

// A5: the forced fallback (`?worklet=broken`). The receipt keeps `state=running` and `underruns=`,
// names `worklet=fallen-back`, and the reason is the browser's own refusal of the missing module.
// The script node plays the same mixer, so the cue proof must pass there too.
export function fallbackVerdict({ receipt, cues }) {
  const problems = [];
  const audio = parseAudio(receipt?.audio);
  if (!audio) return [`the fallback receipt is not an audio line: ${JSON.stringify(receipt?.audio)}`];
  if (audio.state !== "running") problems.push(`the fallback receipt is ${audio.state}, not running: ${receipt.audio}`);
  if (audio.worklet !== "fallen-back") problems.push(`the page did not name the fallback: ${receipt.audio}`);
  if (!audio.reason) problems.push(`the fallback printed no reason: ${receipt.audio}`);
  else if (!/404/.test(audio.reason) || !/audio-worklet-missing\.js/.test(audio.reason)) {
    problems.push(`the fallback reason is not the browser's 404 for the missing module: ${receipt.audio}`);
  }
  if (audio.underruns === null) problems.push(`the fallback receipt has no counted underruns: ${receipt.audio}`);
  const path = parsePath(receipt.path);
  if (path?.path !== "script") problems.push(`#audioworklet says ${path?.path ?? "nothing"}, not script: ${receipt.path}`);
  problems.push(...cueVerdict(cues).map((p) => `the script path: ${p}`));
  return problems;
}

export function parseArgs(argv) {
  const o = { dir: "dist/web", chrome: null, out: "out/web-input", script: "tools/fixtures/input-tape.script", nativeTape: null, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") o.dir = argv[++i];
    else if (a === "--chrome") o.chrome = argv[++i];
    else if (a === "--out") o.out = argv[++i];
    else if (a === "--script") o.script = argv[++i];
    else if (a === "--native-tape") o.nativeTape = argv[++i];
    else o.error = `unknown argument ${a}`;
  }
  if (!o.chrome) o.error ??= "--chrome <path to Chrome> is required";
  return o;
}

// Replay the script as real events, sampling the page's tape at each tick; compare with the native tape when one is given.
async function replayTape({ script, native, send, evalJs }) {
  const steps = parseScript(readFileSync(resolve(script), "utf8"));
  const held = { mask: 0 };
  let line = null;
  for (const step of steps) {
    if (step.type === "tick") line = await evalJs("window.inputSample()");
    else await send(stepCommands(step, held));
    await sleep(40);
  }
  const browser = parseTape(line);
  const nativeTape = native ? parseTape(readFileSync(resolve(native), "utf8")) : null;
  const problems = tapeVerdict(browser, nativeTape, steps.filter((s) => s.type === "tick").length);
  if (native && !nativeTape) problems.push(`${native} holds no input-tape line`);
  const compared = Boolean(nativeTape) && problems.length === 0;
  const say = `input tape: ${browser ? `${browser.ticks} ticks digest ${browser.digest}` : "none"} | ${native ? (compared ? "EQUAL to the native tape" : "DIFFERENT from the native tape") : "NOT COMPARED (no --native-tape)"}`;
  return { problems, say, record: { script, line, browser, native: nativeTape } };
}

export async function main(argv) {
  const a = parseArgs(argv);
  if (a.error) { console.error(`web-input-check: ${a.error}`); return 2; }
  const dir = resolve(a.dir), out = resolve(a.out);
  if (!existsSync(join(dir, "index.html"))) { console.error(`web-input-check: ${dir} has no index.html (run node tools/web-build.mjs)`); return 2; }
  mkdirSync(out, { recursive: true });
  const logFile = join(out, "chrome.log");
  writeFileSync(logFile, "");
  const server = await serve(dir);
  // Input and audio do not need an adapter, but the page is the real one, so it gets the flags that give it one.
  const flags = [...BASE_FLAGS, ...FLAG_SETS[0].flags];
  let browser;
  try {
    browser = await launchChrome(a.chrome, flags, logFile);
    const { cdp } = browser;
    const exceptions = [];
    cdp.on("Runtime.exceptionThrown", (p) => exceptions.push(String(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text ?? "exception").slice(0, 400)));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    const evalJs = async (expression) => {
      const r = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (r.exceptionDetails) throw new Error(`evaluate: ${r.exceptionDetails.text} ${r.exceptionDetails.exception?.description ?? ""}`);
      return r.result.value;
    };
    const send = async (cmds) => { for (const c of cmds) { await cdp.send(c.method, c.params); await sleep(30); } };
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.port}/index.html` });
    // The shell sets data-audio-context as soon as it installs the listeners, before the GPU or the golden check.
    let r = null;
    for (let t0 = Date.now(); Date.now() - t0 < 60000; ) {
      await sleep(250);
      try { r = await evalJs(READ_JS); } catch { r = null; }
      if (r?.context) break;
    }
    if (!r?.context) { console.error("web-input-check: the page never installed input and audio (no data-audio-context in 60 s)"); return 1; }
    await sleep(500); // time alone must not create a context
    const before = await evalJs(READ_JS);
    await send(keyCommands("Escape"));
    await sleep(200);
    const escape = await evalJs(READ_JS);
    await send(keyCommands("KeyW"));
    await sleep(300);
    const keys = await evalJs(READ_JS);
    await send(clickCommands(640, 360));
    let click = null;
    for (let t0 = Date.now(); Date.now() - t0 < 5000; ) {
      await sleep(250);
      click = await evalJs(READ_JS);
      if (parseAudio(click.audio)?.state === "running") break;
    }
    const problems = verdict({ before, escape, keys, click });
    // A5: the worklet receipt, the ring it pulls, and the scripted fight through the page's mixer.
    let worklet = await evalJs(AUDIO_JS);
    for (let t0 = Date.now(); parseAudio(worklet?.audio)?.underruns === null && Date.now() - t0 < 15000; ) {
      await sleep(250);
      worklet = await evalJs(AUDIO_JS);
    }
    let later = worklet;
    const startBlocks = parsePath(worklet?.path)?.blocks ?? null;
    for (let t0 = Date.now(); Date.now() - t0 < 5000; ) {
      await sleep(500);
      later = await evalJs(AUDIO_JS);
      const blocks = parsePath(later?.path)?.blocks ?? null;
      if (startBlocks !== null && blocks !== null && blocks > startBlocks) break;
    }
    problems.push(...workletVerdict({ receipt: worklet, later }));
    const cueLine = await evalJs("window.audioCueCheck?.() ?? null").catch(() => null);
    problems.push(...cueVerdict(cueLine));
    // WEB7: the input tape of the script, against the native one.
    const tape = await replayTape({ script: a.script, native: a.nativeTape, send, evalJs });
    problems.push(...tape.problems);
    // A5: the forced worklet failure. A second load asks for a module that does not exist; the
    // receipt must carry the browser's own reason and the script node must play the same mixer.
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.port}/index.html?worklet=broken` });
    let forced = null;
    for (let t0 = Date.now(); Date.now() - t0 < 60000; ) {
      await sleep(250);
      try { forced = await evalJs(AUDIO_JS); } catch { forced = null; }
      if (forced?.context) break;
    }
    let fallback = null;
    let fallbackCues = null;
    if (!forced?.context) {
      problems.push("the ?worklet=broken page never installed input and audio (no data-audio-context in 60 s)");
    } else {
      await send(clickCommands(640, 360));
      for (let t0 = Date.now(); Date.now() - t0 < 15000; ) {
        await sleep(250);
        try { fallback = await evalJs(AUDIO_JS); } catch { fallback = null; }
        if (parseAudio(fallback?.audio)?.worklet === "fallen-back") break;
      }
      fallbackCues = await evalJs("window.audioCueCheck?.() ?? null").catch(() => null);
      problems.push(...fallbackVerdict({ receipt: fallback, cues: fallbackCues }));
    }
    if (exceptions.length) problems.push(`uncaught exceptions: ${exceptions.join(" | ")}`);
    const record = {
      before, escape, keys, click, worklet, later, cueLine, tape: tape.record,
      forced, fallback, fallbackCues, exceptions, problems,
    };
    writeFileSync(join(out, "input-audio.json"), `${JSON.stringify(record, null, 2)}\n`);
    for (const [name, x] of Object.entries({ before, escape, keys, click })) console.log(`${name.padEnd(7)} ${x.input} | ${x.audio} | context=${x.context} gestures=${x.gestures}`);
    console.log(`worklet ${worklet?.audio ?? "none"} | ${worklet?.path ?? "none"}`);
    console.log(`cues    ${cueLine ?? "none"}`);
    console.log(`fallback ${fallback?.audio ?? "none"} | ${fallback?.path ?? "none"}`);
    console.log(`cues    ${fallbackCues ?? "none"} (script path)`);
    console.log(tape.say);
    const summary = problems.length ? `FAILED - ${problems.join("; ")}` : "OK (no context before a gesture, Escape is not one, a key press starts it, the worklet pulls the mixer's ring, every scripted cue plays, the forced failure prints its reason)";
    console.log(`input and audio: ${summary}`);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### web-input-check\n\`\`\`\n${click.input}\n${click.audio}\n${worklet?.audio}\n${worklet?.path}\n${cueLine}\n${fallback?.audio}\n\`\`\`\n${summary}\n`);
    return problems.length ? 1 : 0;
  } catch (e) {
    console.error(`web-input-check: ${e.message ?? e}`);
    return 2;
  } finally {
    if (browser) await browser.close();
    await server.close();
  }
}

if (import.meta.main) main(process.argv.slice(2)).then((c) => process.exit(c), (e) => { console.error(e); process.exit(2); });
