// codex-check.test.mjs - the codex check against local fixtures.
// No network and no real corpus: a fixture corpus and two fixture tables stand
// in for the Latin Library mirror. A number the passage supports passes, a
// number no passage supports is `unmatched`, a passage whose hash moved is
// `changed`, and the same input twice prints the same lines.
import { deepStrictEqual, equal, match, ok } from "node:assert";
import test from "node:test";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { passageHash } from "./codex-check.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "codex-check.mjs");

const HYGINUS = [
  "Pseudo-Hyginus: de Munitionibus Castrorum",
  "",
  "1. Quibus latitudo dari debeat ad minimum pedum quinque; altum pedes tres.",
  "",
  "2. Vallum loco suspectiori extrui debet. Sufficit latum pedes VIII, altum pedes VI.",
  "",
].join("\n");

const VEGETIUS = [
  "Vegetius Liber I",
  "",
  "I. Nulla enim alia re uidemus populum Romanum orbem subegisse terrarum.",
  "",
  "XXIIII. Castrorum autem diuersa triplexque munitio est. deinde tumultuaria fossa fit lata pedes nouem et alta pedes VII.",
  "",
].join("\n");

/** A fixture corpus and two fixture tables in a temp folder. */
function fixture({ widthFt = 5, hash = null } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "codex-check-"));
  const corpus = join(dir, "corpus");
  mkdirSync(corpus, { recursive: true });
  writeFileSync(join(corpus, "hyginus6.txt"), HYGINUS, "utf8");
  writeFileSync(join(corpus, "vegetius1.txt"), VEGETIUS, "utf8");
  const ditchHash = hash ?? passageHash("Quibus latitudo dari debeat ad minimum pedum quinque; altum pedes tres.");
  const camp = [
    "[units]",
    "mm_per_ft = 296",
    'ref = "fixture"',
    "",
    "[ditch.one_night]",
    `width_ft = ${widthFt}`,
    "depth_ft = 3",
    'ref = "fixture"',
    "",
    "[codex.ditch_width]",
    'entry = "ditch.one_night"',
    'key = "width_ft"',
    'ref = "PHG 1"',
    'form = "quinque"',
    `hash = "${ditchHash}"`,
    "",
    "[codex.ditch_depth]",
    'entry = "ditch.one_night"',
    'key = "depth_ft"',
    'ref = "PHG 1"',
    'form = "tres"',
    `hash = "${ditchHash}"`,
    "",
  ].join("\n");
  const line = [
    "[units]",
    "mm_per_ft = 296",
    'ref = "fixture"',
    "",
    "[ditch.hasty]",
    "width_ft = 9",
    'ref = "fixture"',
    "",
    "[codex.hasty_ditch_width]",
    'entry = "ditch.hasty"',
    'key = "width_ft"',
    'ref = "VEG 1.24"',
    'form = "nouem"',
    `hash = "${passageHash("Castrorum autem diuersa triplexque munitio est. deinde tumultuaria fossa fit lata pedes nouem et alta pedes VII.")}"`,
    "",
  ].join("\n");
  const campFile = join(dir, "camp.toml");
  const lineFile = join(dir, "line.toml");
  writeFileSync(campFile, camp, "utf8");
  writeFileSync(lineFile, line, "utf8");
  return { dir, corpus, campFile, lineFile };
}

/** The tool runs as a child process, so its exit code is the proof. */
function cli(args) {
  return new Promise((done) => {
    const child = spawn("node", [TOOL, ...args], { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (chunk) => { out += chunk; });
    child.stderr.on("data", (chunk) => { err += chunk; });
    child.on("close", (code) => done({ code, out, err }));
  });
}

function run(fx) {
  return cli(["--corpus", fx.corpus, "--camp", fx.campFile, "--line", fx.lineFile]);
}

test("a number the passage supports passes and prints its millimetres", async () => {
  const fx = fixture();
  const res = await run(fx);
  equal(res.code, 0, res.out + res.err);
  match(res.out, /codex: numbers=3 unmatched=0/);
  match(res.out, /camp ditch\.one_night\.width_ft {2}PHG 1 {2}quinque = 5 ft {2}1480 mm {2}table 5 ft {2}ok/);
  match(res.out, /camp ditch\.one_night\.depth_ft {2}PHG 1 {2}tres = 3 ft {2}888 mm {2}table 3 ft {2}ok/);
  match(res.out, /line ditch\.hasty\.width_ft {2}VEG 1\.24 {2}nouem = 9 ft {2}2664 mm {2}table 9 ft {2}ok/);
});

test("a value no passage supports is unmatched and the run exits 1", async () => {
  const fx = fixture({ widthFt: 6 });
  const res = await run(fx);
  equal(res.code, 1, res.out + res.err);
  match(res.out, /unmatched/);
  match(res.out, /codex: numbers=3 unmatched=1/);
  ok(!/ok$/.test(res.out.split("\n").find((line) => line.includes("ditch.width_ft"))), "the row is not ok");
});

test("a passage whose hash moved is changed and the run exits 1", async () => {
  const fx = fixture({ hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000" });
  const res = await run(fx);
  equal(res.code, 1, res.out + res.err);
  match(res.out, /changed/);
  match(res.out, /codex: numbers=3 unmatched=0 changed=2/);
});

test("the same input twice prints the same lines", async () => {
  const fx = fixture();
  const first = await run(fx);
  const second = await run(fx);
  equal(first.code, 0);
  deepStrictEqual(first.out, second.out);
});

test("a missing corpus file names itself and the fetch command", async () => {
  const fx = fixture();
  const res = await cli(["--corpus", join(fx.dir, "empty"), "--camp", fx.campFile, "--line", fx.lineFile]);
  ok(res.code !== 0, "the run fails without the corpus");
  match(res.out, /missing corpus file\(s\): hyginus6\.txt, vegetius1\.txt/);
  match(res.out, /node tools\/codex-check\.mjs --fetch/);
});

test("the shipped tables carry a codex row for every number they name", () => {
  const root = resolve(HERE, "..");
  const camp = readFileSync(join(root, "content", "rome", "camp.toml"), "utf8");
  const line = readFileSync(join(root, "content", "rome", "line.toml"), "utf8");
  const rows = (text) => (text.match(/^\[codex\./gm) ?? []).length;
  equal(rows(camp) + rows(line), 25, "the two tables carry 25 codex rows");
  ok(camp.includes('ref = "PHG 49"'), "the ditch names Pseudo-Hyginus 49");
});
