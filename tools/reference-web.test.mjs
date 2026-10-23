// Unit tests for reference-web's argument parsing, page clock driver, adapter
// naming and fragment/PNG shape. They launch no browser; the capture proof is
// the real two-pass run in the tool's report.
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { verifyTier } from "./reference-captures.mjs";
import { encodePng } from "./web-browse.mjs";
import {
  ArenaClock,
  PageFatal,
  adapterLabel,
  browserFlagSets,
  buildFragment,
  commandLine,
  findBrowser,
  gpuDeviceFor,
  outputRoot,
  pageUrl,
  parseArgs,
  parseHudFrame,
  parseLine,
  parseTicks,
  shotName,
  sourceLine,
  tierAdapter,
  verifyPng,
} from "./reference-web.mjs";

const ROOT = resolve(import.meta.dirname, "..");

test("the defaults are the ten camera-path ticks and the web canvas", () => {
  const args = parseArgs(["--scratch", "out/ref-web"]);
  assert.equal(args.scratch, "out/ref-web");
  assert.equal(args.width, 640);
  assert.equal(args.height, 360);
  assert.equal(args.seed, 2040);
  assert.equal(args.camera, "procedural-arena");
  assert.deepEqual(args.ticks, [0, 60, 120, 180, 240, 300, 360, 420, 480, 540]);
  assert.equal(args.passes, 1);
});

test("scratch mirrors the content/reference set root", () => {
  const args = parseArgs(["--scratch", "out/ref-web"]);
  const root = outputRoot(args);
  assert.equal(root, resolve(ROOT, "out/ref-web"));
  assert.equal(join(root, "web", "web-t000.png"), join(resolve(ROOT, "out/ref-web"), "web", "web-t000.png"));
});

test("the real content folder needs the explicit write word", () => {
  assert.throws(() => outputRoot(parseArgs([])), /refusing to write content\/reference/);
  const args = parseArgs(["--write-content"]);
  assert.equal(outputRoot(args), join(ROOT, "content", "reference"));
  assert.throws(
    () => parseArgs(["--scratch", "out/x", "--write-content"]),
    /two different output folders/,
  );
});

test("ticks parse, ascend and reject repeats", () => {
  assert.deepEqual(parseTicks("0,60,540"), [0, 60, 540]);
  assert.throws(() => parseTicks("60,0"), /ascend/);
  assert.throws(() => parseTicks("0,0"), /ascend/);
  assert.throws(() => parseTicks("0,,60"), /whole numbers/);
  assert.throws(() => parseTicks(""), /whole numbers/);
  assert.throws(() => parseArgs(["--ticks"]), /needs a value/);
});

test("bad values and unknown flags are refused", () => {
  assert.throws(() => parseArgs(["--width", "0"]), /at least 1/);
  assert.throws(() => parseArgs(["--height", "x"]), /whole number/);
  assert.throws(() => parseArgs(["--seed", "-1"]), /whole number/);
  assert.throws(() => parseArgs(["--passes", "0"]), /at least 1/);
  assert.throws(() => parseArgs(["--camera", "  "]), /needs a name/);
  assert.throws(() => parseArgs(["--nope"]), /unknown flag/);
  assert.equal(parseArgs(["--help"]).help, true);
});

test("shot names carry at least three tick digits", () => {
  assert.equal(shotName("web", 0), "web-t000");
  assert.equal(shotName("web", 60), "web-t060");
  assert.equal(shotName("web", 540), "web-t540");
  assert.equal(shotName("web", 1000), "web-t1000");
});

test("HUD and JSON lines parse without swallowing bad text", () => {
  assert.equal(parseHudFrame("frame 12 | 16.7 ms | 60 fps"), 12);
  assert.equal(parseHudFrame("frame 3"), 3);
  assert.equal(parseHudFrame("loading"), null);
  assert.equal(parseLine(null), null);
  assert.equal(parseLine(""), null);
  assert.equal(parseLine("null"), null);
  assert.deepEqual(parseLine('{"ok":true}'), { ok: true });
  assert.throws(() => parseLine("capture: waiting"), /not JSON/);
});

test("adapter names prefer the full description, then name, then vendor/arch", () => {
  assert.equal(
    adapterLabel({ adapter: { description: "NVIDIA GeForce RTX 3050", name: "short" } }),
    "NVIDIA GeForce RTX 3050",
  );
  assert.equal(adapterLabel({ adapter: { description: " ", name: "named card" } }), "named card");
  assert.equal(adapterLabel({ adapter: { vendor: "nvidia", architecture: "ampere" } }), "nvidia/ampere");
  assert.equal(adapterLabel({}), "");
  assert.equal(tierAdapter("tier=web why=query adapter=nvidia/ampere late_compile=0"), "nvidia/ampere");
  assert.equal(tierAdapter("tier=web why=query"), "");
});

test("the browser GPU list supplies the full card and driver", () => {
  const report = { adapter: { vendor: "nvidia", architecture: "ampere" } };
  const devices = [
    { deviceString: "Intel(R) UHD Graphics", driverVendor: "", driverVersion: "1" },
    {
      deviceString: "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
      driverVendor: "NVIDIA",
      driverVersion: "32.0.16.1062",
    },
  ];
  assert.equal(gpuDeviceFor(report, devices)?.deviceString, "NVIDIA GeForce RTX 3050 6GB Laptop GPU");
  assert.equal(gpuDeviceFor({ adapter: { vendor: "google" } }, devices), null);
  assert.equal(gpuDeviceFor(report, []), null);
});

test("the fragment names browser, adapter, command and every row", () => {
  const shots = Array.from({ length: 10 }, (_, at) => ({
    shot: shotName("web", at * 60),
    png: `web/${shotName("web", at * 60)}.png`,
    tick: at * 60,
    width: 640,
    height: 360,
    seed: 2040,
    pixel_fnv: "0123456789abcdef",
    png_fnv: "fedcba9876543210",
    bytes: 123,
  }));
  const fragment = buildFragment({
    browser: "Chrome/155.0.8059.85",
    camera: "procedural-arena",
    seed: 2040,
    width: 640,
    height: 360,
    adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
    driver: "32.0.16.1062",
    command: "node tools/reference-web.mjs --scratch out/ref-web",
    shots,
  });
  assert.equal(fragment.tier, "web");
  assert.equal(fragment.schema, 1);
  assert.equal(fragment.shots.length, 10);
  assert.equal(fragment.camera, "procedural-arena");
  assert.match(fragment.source, /browser \(Chrome\/155/);
  assert.match(fragment.source, /adapter=NVIDIA GeForce RTX 3050/);
  assert.match(fragment.source, /cmd=node tools\/reference-web.mjs/);
  assert.equal(fragment.shots[0].pixel_fnv, "0123456789abcdef");
});

test("a captured PNG checks its canvas and decoded digest", () => {
  const rgba = Buffer.from([1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
  const png = encodePng(2, 2, rgba);
  const shot = { shot: "web-t000", width: 2, height: 2, pixel_fnv: fnvHexForTest(rgba) };
  const checked = verifyPng(png, shot);
  assert.equal(checked.width, 2);
  assert.equal(checked.height, 2);
  assert.notEqual(checked.pixel_fnv, "0000000000000000");
  assert.throws(() => verifyPng(png, { ...shot, width: 3 }), /PNG is 2x2/);
  assert.throws(() => verifyPng(png, { ...shot, pixel_fnv: "0000000000000000" }), /decoded pixels hash/);
});

test("the page clock counts arena callbacks and ignores the other animation loop", async () => {
  let frame = 3;
  let calls = 0;
  const evalJs = async (expression) => {
    if (expression === "window.requestArenaCapture()") return undefined;
    const match = /__referenceFrame\(([^)]+)\)/.exec(expression);
    assert.ok(match, expression);
    calls += 1;
    if (calls % 2 === 1) return [true, `frame ${frame} | non-arena callback`];
    frame += 1;
    return [true, `frame ${frame} | arena`];
  };
  const clock = new ArenaClock(evalJs, 3);
  await clock.target(0);
  assert.equal(clock.frame, 4);
  await clock.runTo(2);
  assert.equal(clock.frame, 5);
  await clock.target(2);
  assert.equal(clock.frame, 6);
  await clock.frozenAt(2);
  assert.equal(clock.frame, 7);
  await clock.runTo(3);
  assert.equal(clock.frame, 7);
  await clock.target(3);
  assert.equal(clock.frame, 8);
});

test("a clock with no queued callback is a named failure", async () => {
  const clock = new ArenaClock(async () => [false, "frame 0"], 0);
  await assert.rejects(() => clock.arenaStepAt(0), /no animation frame queued/);
});

test("page URLs force the web tier, carry the seed and hold the fps ladder", () => {
  assert.equal(pageUrl(1234), "http://127.0.0.1:1234/?tier=web&seed=2040&ladder=off");
  assert.match(pageUrl(1, 7), /tier=web/);
  assert.match(pageUrl(1, 7), /seed=7/);
  assert.match(pageUrl(1, 7), /ladder=off/);
});

test("browser flags start with the real GPU and keep a software fallback", () => {
  const sets = browserFlagSets(640, 360);
  assert.equal(sets.length, 2);
  assert.equal(sets[0].name, "hardware");
  assert.ok(sets[0].flags.includes("--window-size=640,360"));
  assert.ok(!sets[0].flags.includes("--use-webgpu-adapter=swiftshader"));
  assert.equal(sets[1].name, "swiftshader");
  assert.ok(sets[1].flags.includes("--use-webgpu-adapter=swiftshader"));
});

test("an explicit browser must exist", () => {
  const dir = mkdtempSync(join(tmpdir(), "reference-web-test-"));
  const exe = join(dir, "chrome.exe");
  writeFileSync(exe, "");
  assert.equal(findBrowser(exe), resolve(exe));
  assert.throws(() => findBrowser(join(dir, "missing.exe")), /no browser/);
});

test("the command line names the tool and keeps quoted paths", () => {
  const line = commandLine(
    ["--scratch", "out/my shots", "--passes", "2"],
    "C:\\node.exe",
    join(ROOT, "tools", "reference-web.mjs"),
  );
  assert.match(line, /reference-web\.mjs/);
  assert.match(line, /--scratch/);
  assert.match(line, /"out\/my shots"/);
  assert.match(line, /--passes 2/);
});

test("a page fatal is a stable error type", () => {
  const error = new PageFatal("no adapter");
  assert.equal(error.name, "PageFatal");
  assert.equal(error.message, "no adapter");
});

test("the merge verifier keeps the browser source from the fragment", () => {
  const dir = mkdtempSync(join(tmpdir(), "reference-web-set-"));
  mkdirSync(join(dir, "web"), { recursive: true });
  const png = encodePng(640, 360, Buffer.alloc(640 * 360 * 4, 60));
  const shots = [];
  for (const tick of [0, 60, 120, 180, 240, 300, 360, 420, 480, 540]) {
    const shot = `web-t${String(tick).padStart(3, "0")}`;
    writeFileSync(join(dir, "web", `${shot}.png`), png);
    shots.push({
      shot,
      png: `web/${shot}.png`,
      tick,
      width: 640,
      height: 360,
      seed: 2040,
      png_fnv: fnvHexForTest(png),
      bytes: png.length,
    });
  }
  const fragment = buildFragment({
    browser: "Chrome/155.0.8059.85",
    camera: "procedural-arena",
    seed: 2040,
    width: 640,
    height: 360,
    adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
    driver: "32.0.16.1062",
    command: "node tools/reference-web.mjs --scratch out/ref-web",
    shots,
  });
  writeFileSync(join(dir, "web", "fragment.json"), JSON.stringify(fragment));
  const checked = verifyTier(dir, "web");
  assert.equal(checked.rows.length, 10);
  assert.match(checked.rows[0].source, /^browser \(Chrome\/155/);
  assert.equal(checked.rows[0].camera, "procedural-arena");
  assert.equal(checked.rows[0].tick, 0);
});

function fnvHexForTest(bytes) {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of bytes) hash = ((hash ^ BigInt(byte)) * prime) & mask;
  return hash.toString(16).padStart(16, "0");
}
