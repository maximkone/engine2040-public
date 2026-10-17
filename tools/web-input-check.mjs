// Headless Chrome check of input and audio on dist/web (WEB7): serve it on 127.0.0.1, load it, send real (trusted) key and
// mouse events over CDP and read back the two lines the page prints.
//   node tools/web-input-check.mjs --dir dist/web --chrome google-chrome [--out out/web-input]
// What it proves, in one browser:
//  1. Before any gesture: `input: kb=0 mouse=0 pad=0`, `audio: state=suspended`, and no AudioContext exists
//     (body data-audio-context=none).
//  2. Escape (a key the browser does not count as a user activation) is counted by the input line but does not
//     start the audio context.
//  3. A real key press (W down and up) raises kb and creates the context inside the gesture.
//  4. A mouse click raises mouse and the page ends on `audio: state=running` within the wait.
// The pad count is not driven here (Chrome cannot fake a gamepad over CDP); crates/web/src/input.rs tests it.
// Only loopback is served and visited. The Chrome process started here is ours and is closed here.
// Exit: 0 proven, 1 a proof failed, 2 could not run (no Chrome, bad arguments, no page).
import { appendFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { BASE_FLAGS, FLAG_SETS, launchChrome, serve } from "./web-browse.mjs";

export const INPUT_LINE = /^input: kb=(\d+) mouse=(\d+) pad=(\d+)$/;
export const AUDIO_LINE = /^audio: state=(suspended|running)$/;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The numbers and words of the two page lines; null for a line that has the wrong shape.
export function parseInput(text) {
  const m = INPUT_LINE.exec(String(text ?? "").trim());
  return m ? { kb: Number(m[1]), mouse: Number(m[2]), pad: Number(m[3]) } : null;
}
export function parseAudio(text) {
  const m = AUDIO_LINE.exec(String(text ?? "").trim());
  return m ? m[1] : null;
}

// CDP commands for one key press (down then up). `vk` is the Windows virtual key code Chrome wants.
export const KEYS = {
  Escape: { key: "Escape", vk: 27 },
  KeyW: { key: "w", vk: 87, text: "w" },
  KeyD: { key: "d", vk: 68, text: "d" },
};
export function keyCommands(code) {
  const k = KEYS[code];
  if (!k) throw new Error(`no key table row for ${code}`);
  const base = { key: k.key, code, windowsVirtualKeyCode: k.vk, nativeVirtualKeyCode: k.vk };
  return [
    { method: "Input.dispatchKeyEvent", params: { type: k.text ? "keyDown" : "rawKeyDown", ...base, ...(k.text ? { text: k.text } : {}) } },
    { method: "Input.dispatchKeyEvent", params: { type: "keyUp", ...base } },
  ];
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

// What a page read looks like: { input, audio, context, gestures } from #input, #audio and the body data attributes.
export const READ_JS = `(() => ({
  input: document.getElementById('input')?.textContent ?? null,
  audio: document.getElementById('audio')?.textContent ?? null,
  context: document.body.dataset.audioContext ?? null,
  gestures: Number(document.body.dataset.audioGestures ?? -1),
  state: document.body.dataset.state ?? null,
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
  need(i(before).kb === 0 && i(before).mouse === 0 && i(before).pad === 0, `before any event the counts must be 0: ${before.input}`);
  need(parseAudio(before.audio) === "suspended", `before a gesture the audio must be suspended: ${before.audio}`);
  need(before.context === "none", `an AudioContext exists before any gesture (data-audio-context=${before.context})`);
  need(i(escape).kb === 2, `Escape down and up must count 2 key events: ${escape.input}`);
  need(escape.context === "none", `Escape started the audio context (data-audio-context=${escape.context}); it is not a user activation`);
  need(parseAudio(escape.audio) === "suspended", `audio must still be suspended after Escape: ${escape.audio}`);
  need(i(keys).kb === 4, `W down and up must bring kb to 4: ${keys.input}`);
  need(keys.context === "created", `the key press did not create the audio context (data-audio-context=${keys.context})`);
  need(i(click).mouse >= 2, `a click must count at least 2 mouse events (press, release): ${click.input}`);
  need(parseAudio(click.audio) === "running", `after a key press and a click the audio must be running: ${click.audio}`);
  return problems;
}

export function parseArgs(argv) {
  const o = { dir: "dist/web", chrome: null, out: "out/web-input", error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") o.dir = argv[++i];
    else if (a === "--chrome") o.chrome = argv[++i];
    else if (a === "--out") o.out = argv[++i];
    else o.error = `unknown argument ${a}`;
  }
  if (!o.chrome) o.error ??= "--chrome <path to Chrome> is required";
  return o;
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
      if (parseAudio(click.audio) === "running") break;
    }
    const problems = verdict({ before, escape, keys, click });
    if (exceptions.length) problems.push(`uncaught exceptions: ${exceptions.join(" | ")}`);
    const record = { before, escape, keys, click, exceptions, problems };
    writeFileSync(join(out, "input-audio.json"), `${JSON.stringify(record, null, 2)}\n`);
    for (const [name, x] of Object.entries({ before, escape, keys, click })) console.log(`${name.padEnd(7)} ${x.input} | ${x.audio} | context=${x.context} gestures=${x.gestures}`);
    const summary = problems.length ? `FAILED - ${problems.join("; ")}` : "OK (no context before a gesture, Escape is not one, a key press starts it, a click leaves it running)";
    console.log(`input and audio: ${summary}`);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### web-input-check\n\`\`\`\n${click.input}\n${click.audio}\n\`\`\`\n${summary}\n`);
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
