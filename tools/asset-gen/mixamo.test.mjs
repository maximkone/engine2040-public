// node --test tools/asset-gen/mixamo.test.mjs
// No browser and no network: the pure parts, plus two child runs that stop before any browser starts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  DEFAULT_OUT, LICENCE_NOTE, countFbx, exportBody, findPack, folderStatus, hostAllowed, isComplete,
  packSlug, parseArgs, parseProducts, planLines, productsPath, provenanceRecord, recordedAnimations,
  withRetry, UsageError,
} from "./mixamo.mjs";

const HERE = fileURLToPath(new URL(".", import.meta.url));
const SCRIPT = resolve(HERE, "mixamo.mjs");

function tmp() {
  return mkdtempSync(join(tmpdir(), "mixamo-test-"));
}

test("--help is accepted and parses without a command", () => {
  assert.equal(parseArgs(["--help"]).help, true);
});

test("--login, --list and --pack pick their command", () => {
  assert.equal(parseArgs(["--login"]).command, "login");
  const l = parseArgs(["--list", "sword"]);
  assert.equal(l.command, "list");
  assert.equal(l.query, "sword");
  const p = parseArgs(["--pack", "Sword and Shield Pack", "--fps", "30", "--out", "D:\\x", "--dry-run", "--trace"]);
  assert.equal(p.command, "pack");
  assert.equal(p.pack, "Sword and Shield Pack");
  assert.equal(p.fps, 30);
  assert.equal(p.out, "D:\\x");
  assert.equal(p.dryRun, true);
  assert.equal(p.trace, true);
});

test("defaults: fps 60, output root C:\\forge-media\\mixamo", () => {
  const p = parseArgs(["--pack", "Idle"]);
  assert.equal(p.fps, 60);
  assert.equal(p.out, DEFAULT_OUT);
  assert.equal(DEFAULT_OUT, "C:\\forge-media\\mixamo");
});

test("bad arguments are usage errors", () => {
  assert.throws(() => parseArgs([]), UsageError);
  assert.throws(() => parseArgs(["--pack"]), UsageError);
  assert.throws(() => parseArgs(["--pack", "--dry-run"]), UsageError);
  assert.throws(() => parseArgs(["--pack", "Idle", "--fps", "50"]), UsageError);
  assert.throws(() => parseArgs(["--login", "--list", "x"]), UsageError);
  assert.throws(() => parseArgs(["--list", "   "]), UsageError);
  assert.throws(() => parseArgs(["--bogus"]), UsageError);
});

test("packSlug lowers case and turns runs of other characters into one underscore", () => {
  assert.equal(packSlug("Sword and Shield Pack"), "sword_and_shield_pack");
  assert.equal(packSlug("  Ybot / Pack!!  "), "ybot_pack");
  assert.equal(packSlug("Pro Sword & Shield"), "pro_sword_shield");
  assert.throws(() => packSlug("!!!"), UsageError);
});

test("isComplete: one FBX per animation plus the character; unknown count is never complete", () => {
  assert.equal(isComplete(8, 7), true);
  assert.equal(isComplete(7, 7), false);
  assert.equal(isComplete(0, 0), false);
  assert.equal(isComplete(40, null), false);
  assert.equal(isComplete(40, undefined), false);
});

test("folderStatus reads the disk: absent, partial and complete", () => {
  const dir = tmp();
  try {
    assert.deepEqual(folderStatus(join(dir, "missing"), 2), { fbx: 0, complete: false });
    mkdirSync(join(dir, "pack"));
    writeFileSync(join(dir, "pack", "a.fbx"), "");
    writeFileSync(join(dir, "pack", "b.FBX"), "");
    writeFileSync(join(dir, "pack", "notes.txt"), "");
    assert.equal(countFbx(join(dir, "pack")), 2);
    // 2 FBX on disk: complete for a pack of 1 animation (1 + character), not for 2
    assert.equal(folderStatus(join(dir, "pack"), 1).complete, true);
    assert.equal(folderStatus(join(dir, "pack"), 2).complete, false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("recordedAnimations reads PROVENANCE.json when present", () => {
  const dir = tmp();
  try {
    assert.equal(recordedAnimations(dir), null);
    writeFileSync(join(dir, "PROVENANCE.json"), JSON.stringify({ animations: 12 }));
    assert.equal(recordedAnimations(dir), 12);
    writeFileSync(join(dir, "PROVENANCE.json"), "not json");
    assert.equal(recordedAnimations(dir), null);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("provenance record names the source, format, fps and the licence note", () => {
  const rec = provenanceRecord({
    pack: "Sword and Shield Pack", slug: "sword_and_shield_pack", animations: 30, fps: 60,
    character: "abc", fetchedAt: "2026-10-22T10:00:00.000Z",
  });
  assert.equal(rec.pack, "Sword and Shield Pack");
  assert.equal(rec.animations, 30);
  assert.equal(rec.fps, 60);
  assert.equal(rec.format, "fbx7_2019");
  assert.equal(rec.source, "Adobe Mixamo");
  assert.equal(rec.licence, "Mixamo: royalty-free for use in games; do not redistribute the raw files as a library");
  assert.equal(rec.licence, LICENCE_NOTE);
  assert.equal(rec.fetched_at, "2026-10-22T10:00:00.000Z");
  assert.equal(rec.keyframe_reduction, "none");
});

test("export body asks for the whole pack as FBX, T-pose with skin, no keyframe reduction", () => {
  const body = exportBody({ pack: "Idle", character: "c1", fps: 24 });
  assert.equal(body.character_id, "c1");
  assert.equal(body.type, "MotionPack");
  assert.equal(body.product_name, "Idle");
  assert.deepEqual(body.preferences, { format: "fbx7_2019", mesh: "t-pose", skin: "true", fps: "24", reducekf: "0" });
});

test("findPack matches the name ignoring case and spacing, else names the close ones", () => {
  const packs = [{ id: "1", name: "Sword and Shield Pack" }, { id: "2", name: "Sword Idle" }];
  assert.equal(findPack(packs, "  sword AND shield   pack ").id, "1");
  assert.throws(() => findPack(packs, "Sword"), /no pack named "Sword"; close names: "Sword and Shield Pack", "Sword Idle"/);
});

test("parseProducts reads id, name, type and any animation count, tolerant of shape", () => {
  const rows = parseProducts({ results: [
    { id: "a", name: "Pack A", type: "MotionPack", motions: [{}, {}, {}] },
    { id: "b", name: "Pack B", type: "MotionPack", motion_count: 9 },
    { id: "c", name: "Pack C" },
    { name: "no id" },
  ] });
  assert.deepEqual(rows, [
    { id: "a", name: "Pack A", type: "MotionPack", animations: 3 },
    { id: "b", name: "Pack B", type: "MotionPack", animations: 9 },
    { id: "c", name: "Pack C", type: null, animations: null },
  ]);
  assert.deepEqual(parseProducts(null), []);
});

test("productsPath encodes the query and asks for packs and motions", () => {
  assert.equal(productsPath("sword", 24), "/api/v1/products?page=1&limit=24&type=Motion%2CMotionPack&query=sword");
  assert.match(productsPath("a b&c"), /query=a%20b%26c/);
});

test("hostAllowed admits mixamo and Adobe sign-in hosts only", () => {
  assert.equal(hostAllowed("https://www.mixamo.com/#/"), true);
  assert.equal(hostAllowed("https://mixamo.com/api/v1/products"), true);
  assert.equal(hostAllowed("https://auth.services.adobe.com/en_US/index.html"), true);
  assert.equal(hostAllowed("https://ims-na1.adobelogin.com/ims/check"), true);
  assert.equal(hostAllowed("data:text/plain,hi"), true);
  assert.equal(hostAllowed("https://evil.example/mixamo.com"), false);
  assert.equal(hostAllowed("https://notmixamo.com/"), false);
  assert.equal(hostAllowed("https://mixamo.com.evil.example/"), false);
  assert.equal(hostAllowed("file:///C:/x"), false);
  assert.equal(hostAllowed("not a url"), false);
});

test("withRetry: at most 3 retries with a pause between, then the last error", async () => {
  let calls = 0;
  const pauses = [];
  const sleepFn = async (ms) => { pauses.push(ms); };
  await assert.rejects(withRetry(async () => { calls++; throw new Error("boom " + calls); }, { sleepFn, pauseMs: 20000 }), /boom 4/);
  assert.equal(calls, 4);
  assert.deepEqual(pauses, [20000, 20000, 20000]);
});

test("withRetry returns the first success without pausing", async () => {
  let calls = 0;
  const v = await withRetry(async () => { calls++; if (calls < 2) throw new Error("once"); return 7; }, { sleepFn: async () => {} });
  assert.equal(v, 7);
  assert.equal(calls, 2);
});

test("planLines shows the folder state and the steps, with no network", () => {
  const dir = tmp();
  try {
    const out = planLines({ pack: "Idle", slug: "idle", out: dir, fps: 60, character: null, folder: { fbx: 0, complete: false } });
    const text = out.join("\n");
    assert.match(text, /dry run \(no browser, no network\)/);
    assert.match(text, /state:\s+absent/);
    assert.match(text, /60 fps/);
    const partial = planLines({ pack: "Idle", slug: "idle", out: dir, fps: 60, character: "c", folder: { fbx: 3, complete: false } }).join("\n");
    assert.match(partial, /partial \(3 FBX\)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--help prints usage and exits 0 with no browser", () => {
  const r = spawnSync(process.execPath, [SCRIPT, "--help"], { encoding: "utf8", timeout: 20000 });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /--login/);
  assert.match(r.stdout, /--dry-run/);
});

test("--pack --dry-run prints the plan and exits 0 with no browser", () => {
  const dir = tmp();
  try {
    const r = spawnSync(process.execPath, [SCRIPT, "--pack", "Sword and Shield Pack", "--fps", "30", "--out", dir, "--dry-run"], { encoding: "utf8", timeout: 20000 });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /sword_and_shield_pack/);
    assert.match(r.stdout, /30 fps/);
    assert.match(r.stdout, /state:\s+absent/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a bad argument exits 2 before any browser starts", () => {
  const r = spawnSync(process.execPath, [SCRIPT, "--pack", "Idle", "--fps", "50"], { encoding: "utf8", timeout: 20000 });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--fps must be one of/);
});
