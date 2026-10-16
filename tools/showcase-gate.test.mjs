// Run: node --test tools/showcase-gate.test.mjs
// Fresh set with fighters passes and prints age hours plus fighter count.
// Old set fails on age. Zero fighters fails. Empty dir fails.
// NOTES.md rules and the manifest fallback hold. JSON prints machine output.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync, utimesSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { parseFightersFromNotes, checkShowcase } from "./showcase-gate.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "showcase-gate.mjs");

// One fixture set: NOTES.md plus one clip with a manifest and dummy PNGs.
// ageHours backdates the set dir mtime so the age gate can be tripped.
function makeSet(root, name, { notes = null, manifestFighters = null, pngs = 1, ageHours = 0 } = {}) {
  const dir = join(root, name);
  mkdirSync(join(dir, "clip"), { recursive: true });
  if (notes !== null) writeFileSync(join(dir, "NOTES.md"), notes, "utf8");
  if (manifestFighters !== null) {
    writeFileSync(join(dir, "clip", "manifest.json"), JSON.stringify({
      renderer: "presenter-twin", frames: [{ file: "a.png" }], fighters: manifestFighters,
    }), "utf8");
  }
  for (let i = 0; i < pngs; i += 1) {
    writeFileSync(join(dir, "clip", "frame" + String(i).padStart(4, "0") + ".png"), Buffer.alloc(8, i));
  }
  if (ageHours) {
    const t = new Date(Date.now() - ageHours * 3_600_000);
    utimesSync(dir, t, t);
  }
  return dir;
}

function runGate(args) {
  const r = spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8" });
  return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "" };
}

test("fresh set with staged fighters passes and prints age hours and fighter count", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  makeSet(root, "2026-10-09", {
    notes: "V1 hero plus V2 arena-day plus V3 fight with 2 staged fighters in motion, look_stage fighters, 0 red-box stand-ins.",
    pngs: 3,
  });
  const { code, out, err } = runGate(["--dir", root]);
  assert.equal(code, 0, "fresh fighters set is exit 0, got " + code + ": " + err + out);
  assert.match(out, /showcase-gate PASS/);
  assert.match(out, /age .* h/);
  assert.match(out, /fighters=2/);
});

test("newest set older than 5 waves (24 h) fails loudly on age", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  makeSet(root, "2026-09-01", { notes: "Clip with 2 staged fighters in motion.", ageHours: 30 });
  const { code, err } = runGate(["--dir", root]);
  assert.equal(code, 1, "old set is exit 1");
  assert.match(err, /showcase-gate FAIL/);
  assert.match(err, / h old/);
  assert.match(err, /fighters=2/);
});

test("newest set with 0 fighters fails loudly", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  makeSet(root, "2026-10-09", { notes: "Orbit dome plus red boxes, 0 fighters on screen.", pngs: 2 });
  const { code, err } = runGate(["--dir", root]);
  assert.equal(code, 1, "0 fighters is exit 1");
  assert.match(err, /holds 0 fighters/);
  assert.match(err, /fighters=0/);
});


test("empty showcase dir fails loudly with no capture set", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  const { code, err } = runGate(["--dir", root]);
  assert.equal(code, 1, "no set is exit 1");
  assert.match(err, /no capture set yet/);
});

test("NOTES fighter rules: staged count wins, look_stage gives 2, else 0", () => {
  assert.equal(parseFightersFromNotes("3 staged fighters in motion"), 3);
  assert.equal(parseFightersFromNotes("V3 fight, look_stage fighters, 0 red-box stand-ins"), 2);
  assert.equal(parseFightersFromNotes("dome plus red boxes, no fighters on screen"), 0);
});

test("manifest fighters carry the count when NOTES.md is missing", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  makeSet(root, "2026-10-09", { notes: null, manifestFighters: 4, pngs: 1 });
  const r = checkShowcase(root, {});
  assert.equal(r.ok, true);
  assert.equal(r.fighters, 4);
  assert.equal(r.source, "manifest.json");
});

test("--json prints machine JSON with age hours and fighters", () => {
  const root = mkdtempSync(join(tmpdir(), "gate-"));
  makeSet(root, "2026-10-09", { notes: "Clip with 2 staged fighters in motion.", pngs: 1 });
  const { code, out } = runGate(["--dir", root, "--json"]);
  assert.equal(code, 0);
  const o = JSON.parse(out);
  assert.equal(o.ok, true);
  assert.equal(o.fighters, 2);
  assert.ok(typeof o.ageHours === "number");
});

