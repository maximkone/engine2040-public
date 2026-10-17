import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { ACCEPTED_SPDX, COMBAT_TARGET, COOK_SCRIPT, DIGEST_RE, GLB_BYTES_MAX, GLB_BYTES_MIN, GLB_SHA_RE, KEYS_ALLOWED, REFUSED_MARKS, checkClip, checkCookInputs, checkManifest, cookSpawnPlan, findExecutable, parseCookArgs, runCook } from "./cook-char.mjs";

const hex16 = (n) => n.toString(16).padStart(16, "0");

const goodClip = (i) => ({
  short: `clip${i}`,
  source_action: `act${i}`,
  glb_bytes: 60000,
  cook_digest: hex16(0x1000 + i),
  glb_sha16: hex16(0x2000 + i),
  keys: 432,
  loop: true,
  root_motion: false,
});

const goodManifest = () => {
  const canon = Array.from({ length: 14 }, (_, i) => goodClip(i));
  const batchA = Array.from({ length: 16 }, (_, i) => goodClip(100 + i));
  return {
    locomotion: {
      record_100style: { spdx: "CC-BY-4.0", fetched: false },
      cmu: { fetched: false },
      synthetic_bake: { db_digest: `0x${hex16(7)}`, clips: 8, frames: 360, entries: 360 },
      first_16: { fetched: false },
    },
    combat: { target: COMBAT_TARGET, canonical_14: canon, batch_a_16: batchA, landed: 30, remaining: { open_count: 50 } },
    refused: ["geno footage", "lafan1 set", "mixamo pack", "qal rig", "unreal asset"],
  };
};

test("gates stay pinned: target, licences, keys, byte envelope", () => {
  assert.equal(COMBAT_TARGET, 80);
  assert.ok(ACCEPTED_SPDX.has("CC0-1.0") && ACCEPTED_SPDX.has("CC-BY-4.0"));
  for (const m of ["geno", "lafan1", "mixamo", "qal", "unreal"]) assert.ok(REFUSED_MARKS.includes(m));
  for (const k of [432, 475, 819, 862]) assert.ok(KEYS_ALLOWED.has(k));
  assert.ok(GLB_BYTES_MIN < 60000 && 60000 < GLB_BYTES_MAX);
  assert.match(hex16(1), DIGEST_RE);
  assert.doesNotMatch("short", DIGEST_RE);
  assert.match(hex16(2), GLB_SHA_RE);
});

test("checkClip accepts a clean clip and records its digest", () => {
  const seen = new Set();
  assert.deepEqual(checkClip(goodClip(1), seen), []);
  assert.ok(seen.has(hex16(0x1001)));
});

test("checkClip flags duplicate digest, bad bytes, bad keys, bad flags", () => {
  const seen = new Set([hex16(0x1001)]);
  const dup = { ...goodClip(1) };
  assert.match(checkClip(dup, seen).join("\n"), /duplicate cook_digest/);
  const badBytes = { ...goodClip(2), cook_digest: hex16(0x9999), glb_bytes: 10 };
  assert.match(checkClip(badBytes, new Set()).join("\n"), /glb_bytes/);
  const badKeys = { ...goodClip(3), cook_digest: hex16(0x999a), keys: 12 };
  assert.match(checkClip(badKeys, new Set()).join("\n"), /keys 12/);
  const badLoop = { ...goodClip(4), cook_digest: hex16(0x999b), loop: "yes" };
  assert.match(checkClip(badLoop, new Set()).join("\n"), /loop is not a boolean/);
});

test("checkManifest accepts the 30-row Tier A shape", () => {
  assert.deepEqual(checkManifest(goodManifest()), []);
});

test("checkManifest flags drift: target, counts, landed, refused", () => {
  const m = goodManifest();
  m.combat.target = 99;
  m.combat.canonical_14.pop();
  m.combat.landed = 1;
  m.refused = ["nothing refused"];
  const errs = checkManifest(m).join("\n");
  assert.match(errs, /combat\.target 99/);
  assert.match(errs, /canonical_14 holds 13 rows/);
  assert.match(errs, /combat\.landed/);
  assert.match(errs, /refused list must name geno/);
});

test("checkManifest refuses fetched bytes and bad bake digest", () => {
  const m = goodManifest();
  m.locomotion.record_100style.fetched = true;
  m.locomotion.synthetic_bake.db_digest = "bad";
  const errs = checkManifest(m).join("\n");
  assert.match(errs, /must not claim fetched bytes/);
  assert.match(errs, /db_digest malformed/);
});

test("checkClip and checkManifest reject empty shapes", () => {
  assert.match(checkClip({}, new Set()).join("\n"), /missing short/);
  assert.deepEqual(checkManifest(null), ["manifest is not an object"]);
  assert.match(checkManifest({}).join("\n"), /locomotion section missing/);
});

test("stub_blender_receives_exact_argv_and_reports_peak", () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-stub-"));
  const script = join(dir, "cook_char.py"), input = join(dir, "in.fbx"), output = join(dir, "out.glb");
  const argvFile = join(dir, "argv.json"), stubJs = join(dir, "blender-stub.js");
  writeFileSync(script, "# stub\n");
  writeFileSync(input, "in");
  const js = `import { writeFileSync } from "node:fs";\nwriteFileSync(process.env.COOK_STUB_OUT, JSON.stringify(process.argv.slice(2)), "utf8");\n`;
  writeFileSync(stubJs, js);
  const sh = join(dir, "blender");
  writeFileSync(sh, `#!/usr/bin/env node\n${js}`);
  try { chmodSync(sh, 0o755); } catch {}
  writeFileSync(join(dir, "blender.cmd"), `@echo off\r\nnode "${stubJs}" %*\r\n`);
  const tool = fileURLToPath(new URL("./cook-char.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [tool, "--cook", input, output, "--script", script], { encoding: "utf8", env: { ...process.env, PATH: dir + delimiter + process.env.PATH, COOK_STUB_OUT: argvFile } });
  assert.equal(r.status, 0, `stub cook exits 0, got ${r.status} stderr=${r.stderr} stdout=${r.stdout}`);
  assert.match(String(r.stdout), /cook: peak_ws_mb=/);
  assert.deepEqual(JSON.parse(readFileSync(argvFile, "utf8")), ["-b", "-P", script, "--", input, output]);
});

test("no_blender_exits_2_with_hint", () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-empty-"));
  const input = join(dir, "in.fbx"), output = join(dir, "out.glb"), script = join(dir, "cook_char.py");
  writeFileSync(input, "in");
  writeFileSync(script, "# stub\n");
  const tool = fileURLToPath(new URL("./cook-char.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [tool, "--cook", input, output, "--script", script], { encoding: "utf8", env: { ...process.env, PATH: dir } });
  assert.equal(r.status, 2, `missing blender exits 2, got ${r.status} stderr=${r.stderr} stdout=${r.stdout}`);
  assert.match(String(r.stderr), /blender/i);
  assert.match(String(r.stderr), /install/i);
});

// ---- GS24 runner: pure helpers, fake child process, then real Blender legs.
test("findExecutable follows PATHEXT order on Windows and skips bare files", () => {
  // NTFS is case-insensitive and PATHEXT is upper case, so compare lower case.
  const present = new Set(["d:\\tools\\blender.cmd", "c:\\bin\\blender", "c:\\bin\\blender.bat"]);
  const exists = (p) => present.has(p.toLowerCase());
  const low = (v) => (v === null ? v : v.toLowerCase());
  const got = findExecutable("blender", { platform: "win32", env: { Path: "C:\\bin;D:\\tools" }, exists });
  assert.equal(low(got), "c:\\bin\\blender.bat");
  present.delete("c:\\bin\\blender.bat");
  assert.equal(low(findExecutable("blender", { platform: "win32", env: { PATH: "C:\\bin;D:\\tools" }, exists })), "d:\\tools\\blender.cmd");
  present.clear();
  present.add("c:\\bin\\blender");
  assert.equal(findExecutable("blender", { platform: "win32", env: { Path: "C:\\bin" }, exists }), null);
});

test("findExecutable on posix needs the exact name and null when absent", () => {
  const exists = (p) => p === "/opt/b/blender";
  assert.equal(findExecutable("blender", { platform: "linux", env: { PATH: "/usr/bin:/opt/b" }, exists }), "/opt/b/blender");
  assert.equal(findExecutable("blender", { platform: "linux", env: { PATH: "/usr/bin" }, exists }), null);
  assert.equal(findExecutable("blender", { platform: "linux", env: {}, exists }), null);
  assert.equal(findExecutable("/opt/b/blender", { platform: "linux", env: {}, exists: (p) => p === "/opt/b/blender" }), "/opt/b/blender");
  const explicit = findExecutable("D:\\apps\\blender", { platform: "win32", env: {}, exists: (p) => p.toLowerCase() === "d:\\apps\\blender.exe" });
  assert.equal(explicit && explicit.toLowerCase(), "d:\\apps\\blender.exe");
});

test("cookSpawnPlan starts an exe directly and quotes a cmd launcher", () => {
  const args = ["-b", "-P", "C:\\my dir\\cook_char.py", "--", "in 1.bvh", "out.glb"];
  assert.deepEqual(cookSpawnPlan("C:\\Program Files\\Blender\\blender.exe", args, "win32"), { cmd: "C:\\Program Files\\Blender\\blender.exe", args, shell: false });
  const cmd = cookSpawnPlan("C:\\b\\blender.CMD", args, "win32");
  assert.equal(cmd.shell, true);
  assert.equal(cmd.cmd, '"C:\\b\\blender.CMD"');
  assert.deepEqual(cmd.args, ['"-b"', '"-P"', '"C:\\my dir\\cook_char.py"', '"--"', '"in 1.bvh"', '"out.glb"']);
  assert.deepEqual(cookSpawnPlan("/usr/bin/blender", args, "linux"), { cmd: "/usr/bin/blender", args, shell: false });
});

test("checkCookInputs names each defect before Blender starts", () => {
  const yes = () => true;
  assert.equal(checkCookInputs({ input: "a.bvh", output: "b.glb", script: "s.py" }, yes), null);
  assert.equal(checkCookInputs({ input: "A.FBX", output: "B.GLTF", script: "s.py" }, yes), null);
  assert.equal(checkCookInputs({ output: "b.glb", script: "s.py" }, yes).code, "E_COOK_USAGE");
  assert.equal(checkCookInputs({ input: "a.bvh", output: "b.glb", script: "s.py" }, (p) => p !== "a.bvh").code, "E_COOK_INPUT");
  assert.equal(checkCookInputs({ input: "a.obj", output: "b.glb", script: "s.py" }, yes).code, "E_COOK_FORMAT");
  assert.equal(checkCookInputs({ input: "a.bvh", output: "b.fbx", script: "s.py" }, yes).code, "E_COOK_FORMAT");
  assert.equal(checkCookInputs({ input: "a.bvh", output: "b.glb", script: "s.py" }, (p) => p !== "s.py").code, "E_COOK_SCRIPT");
});

test("parseCookArgs reads in, out, script and blender (env or flag)", () => {
  assert.deepEqual(parseCookArgs(["--cook", "a.bvh", "b.glb"], {}), { input: "a.bvh", output: "b.glb", script: COOK_SCRIPT, blender: "blender" });
  const full = parseCookArgs(["--cook", "a.bvh", "b.glb", "--script", "s.py", "--blender=X:/bl.exe"], { E2040_BLENDER: "env.exe" });
  assert.deepEqual(full, { input: "a.bvh", output: "b.glb", script: "s.py", blender: "X:/bl.exe" });
  assert.equal(parseCookArgs(["--cook", "a.bvh", "b.glb"], { E2040_BLENDER: "env.exe" }).blender, "env.exe");
  assert.match(parseCookArgs(["--cook", "a.bvh"], {}).error, /E_USAGE/);
  assert.match(parseCookArgs(["--cook", "--script", "x"], {}).error, /E_USAGE/);
  assert.ok(COOK_SCRIPT.endsWith(join("tools", "blender", "cook_char.py")));
  assert.ok(existsSync(COOK_SCRIPT), "the cook script the runner points at exists");
});

function fakeChild({ code = 0, errText = "", pid = 4242, error = null } = {}) {
  const child = new EventEmitter();
  child.pid = pid;
  child.stderr = new EventEmitter();
  setImmediate(() => {
    if (errText) child.stderr.emit("data", Buffer.from(errText));
    if (error) child.emit("error", error);
    else child.emit("close", code);
  });
  return child;
}
const sink = () => { const c = []; return { write: (s) => c.push(String(s)), text: () => c.join("") }; };

async function fakeCook(over = {}) {
  const calls = [];
  const samples = over.samples ?? [120, 340];
  const out = sink(), err = sink();
  const r = await runCook({
    input: "in.bvh", output: "out.glb", script: "cook_char.py",
    findFn: () => "C:\\Program Files\\Blender\\blender.exe",
    existsFn: () => true,
    sampleFn: () => samples.shift() ?? null,
    spawnFn: (cmd, args, opts) => { calls.push([cmd, args, opts]); return fakeChild(over.child); },
    stdout: out, stderr: err, ...over.run,
  });
  return { r, calls, out: out.text(), err: err.text() };
}

test("runCook hands Blender the exact argv directly and reports the sampled peak", async () => {
  const { r, calls, out } = await fakeCook();
  assert.equal(r.exit, 0);
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "C:\\Program Files\\Blender\\blender.exe");
  assert.deepEqual(calls[0][1], ["-b", "-P", "cook_char.py", "--", "in.bvh", "out.glb"]);
  assert.equal(calls[0][2].shell, false);
  assert.equal(out, "cook: peak_ws_mb=340\n");
  assert.equal(r.peakWsMb, 340);
});

test("runCook with no sample says UNKNOWN but still exits 0", async () => {
  const { r, out } = await fakeCook({ samples: [] });
  assert.equal(r.exit, 0);
  assert.equal(out, "cook: peak_ws_mb=UNKNOWN\n");
});

test("runCook passes a failing Blender code through and prints E_COOK_EXIT", async () => {
  const { r, out, err } = await fakeCook({ child: { code: 3, errText: "E_COOK_IMPORT bad file\n" } });
  assert.equal(r.exit, 3);
  assert.equal(out, "");
  assert.match(err, /E_COOK_IMPORT bad file/);
  assert.match(err, /E_COOK_EXIT 3/);
  assert.doesNotMatch(err, /E_BLENDER_MISSING/);
});

test("runCook does not mistake 'not found' text from a failed cook for a missing Blender", async () => {
  const { r, err } = await fakeCook({ child: { code: 3, errText: "E_COOK_IMPORT file not found or not recognized\n" } });
  assert.equal(r.exit, 3);
  assert.doesNotMatch(err, /E_BLENDER_MISSING/);
});

test("runCook treats a signal kill (null code) as a failure", async () => {
  const { r, out, err } = await fakeCook({ child: { code: null } });
  assert.equal(r.exit, 1);
  assert.equal(out, "");
  assert.match(err, /E_COOK_EXIT 1/);
});

test("runCook without Blender on PATH returns 2 with the hint and never spawns", async () => {
  const { r, calls, err } = await fakeCook({ run: { findFn: () => null } });
  assert.equal(r.exit, 2);
  assert.equal(calls.length, 0);
  assert.match(err, /^E_BLENDER_MISSING install Blender/);
});

test("runCook refuses a bad call before spawning", async () => {
  const cases = [
    [{ existsFn: (p) => p !== "in.bvh" }, "E_COOK_INPUT"],
    [{ input: "in.obj" }, "E_COOK_FORMAT"],
    [{ output: "out.fbx" }, "E_COOK_FORMAT"],
    [{ existsFn: (p) => p !== "cook_char.py" }, "E_COOK_SCRIPT"],
  ];
  for (const [run, code] of cases) {
    const { r, calls, err } = await fakeCook({ run });
    assert.equal(r.exit, 2, code);
    assert.equal(calls.length, 0, code);
    assert.match(err, new RegExp(`^${code} `), code);
  }
});

test("runCook maps a spawn ENOENT to the missing-Blender exit", async () => {
  const { r, err } = await fakeCook({ child: { error: Object.assign(new Error("spawn blender ENOENT"), { code: "ENOENT" }) } });
  assert.equal(r.exit, 2);
  assert.match(err, /E_BLENDER_MISSING/);
  const other = await fakeCook({ child: { error: Object.assign(new Error("boom"), { code: "EACCES" }) } });
  assert.equal(other.r.exit, 1);
  assert.match(other.err, /E_BLENDER_SPAWN boom/);
});

// Real Blender: one process per clip, nothing stubbed. Skipped without blender on PATH.
const HAS_BLENDER = spawnSync("blender", ["--version"], { encoding: "utf8", shell: false }).status === 0;
const BL_SKIP = HAS_BLENDER ? false : "blender not on PATH";
const BVH = [
  "HIERARCHY", "ROOT Hips", "{", "  OFFSET 0.0 0.0 0.0",
  "  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation",
  "  JOINT Spine", "  {", "    OFFSET 0.0 10.0 0.0", "    CHANNELS 3 Zrotation Xrotation Yrotation",
  "    End Site", "    {", "      OFFSET 0.0 10.0 0.0", "    }", "  }", "}",
  "MOTION", "Frames: 4", "Frame Time: 0.0333333",
  "0 0 0 0 0 0 0 0 0", "0 1 0 10 0 0 5 0 0", "0 2 0 20 0 0 10 0 0", "0 3 0 30 0 0 15 0 0", "",
].join("\n");

function glbJson(path) {
  const b = readFileSync(path);
  assert.equal(b.subarray(0, 4).toString("latin1"), "glTF");
  assert.equal(b.readUInt32LE(8), b.length, "GLB header length matches the file");
  return JSON.parse(b.subarray(20, 20 + b.readUInt32LE(12)).toString("utf8"));
}

function cookReal(input, output) {
  const tool = fileURLToPath(new URL("./cook-char.mjs", import.meta.url));
  return spawnSync(process.execPath, [tool, "--cook", input, output], { encoding: "utf8", timeout: 180000 });
}

test("real Blender cooks a BVH to a GLB with the skeleton, an animation and a measured peak", { skip: BL_SKIP, timeout: 200000 }, () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-real-"));
  try {
    const bvh = join(dir, "walk.bvh"), glb = join(dir, "sub", "walk.glb");
    writeFileSync(bvh, BVH);
    const r = cookReal(bvh, glb);
    assert.equal(r.status, 0, `stderr=${r.stderr}\nstdout=${r.stdout}`);
    assert.match(r.stdout, /cook_char: objects=1 actions=1 bytes=\d+ out=/);
    const peak = /cook: peak_ws_mb=(\d+)\n/.exec(r.stdout);
    assert.ok(peak && Number(peak[1]) >= 50, `a real Blender working set is sampled, got ${peak && peak[1]}`);
    const j = glbJson(glb);
    const names = j.nodes.map((n) => n.name);
    assert.ok(names.includes("Hips") && names.includes("Spine"), names.join(","));
    assert.equal(j.animations.length, 1);
    assert.ok(j.animations[0].channels.length >= 2);
    assert.equal(j.meshes, undefined, "no default cube rides along");
    assert.equal(j.cameras, undefined);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("real Blender: a corrupt clip exits 3, names the import, and leaves no output", { skip: BL_SKIP, timeout: 200000 }, () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-real-"));
  try {
    const bad = join(dir, "bad.bvh"), glb = join(dir, "bad.glb");
    writeFileSync(bad, "this is not a bvh\n");
    const r = cookReal(bad, glb);
    assert.equal(r.status, 3, `stderr=${r.stderr}`);
    assert.match(r.stderr, /E_COOK_IMPORT /);
    assert.match(r.stderr, /E_COOK_EXIT 3/);
    assert.doesNotMatch(r.stdout, /cook: peak_ws_mb/);
    assert.equal(existsSync(glb), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("real Blender: the glTF and FBX import branches both reach a GLB", { skip: BL_SKIP, timeout: 400000 }, () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-real-"));
  try {
    const bvh = join(dir, "walk.bvh"), first = join(dir, "first.glb"), again = join(dir, "again.glb");
    writeFileSync(bvh, BVH);
    assert.equal(cookReal(bvh, first).status, 0);
    // glTF in, glTF out.
    const r = cookReal(first, again);
    assert.equal(r.status, 0, `stderr=${r.stderr}`);
    assert.equal(glbJson(again).animations.length, 1);
    // FBX in: Blender itself writes the FBX from the BVH, then the runner cooks it.
    const fbx = join(dir, "walk.fbx"), viaFbx = join(dir, "via-fbx.glb");
    const expr = `import bpy; bpy.ops.wm.read_factory_settings(use_empty=True); bpy.ops.import_anim.bvh(filepath=r'${bvh}'); bpy.ops.export_scene.fbx(filepath=r'${fbx}')`;
    const mk = spawnSync("blender", ["-b", "--python-expr", expr], { encoding: "utf8", shell: false, timeout: 180000 });
    assert.equal(mk.status, 0, mk.stderr);
    assert.ok(existsSync(fbx), "the FBX fixture was written");
    const f = cookReal(fbx, viaFbx);
    assert.equal(f.status, 0, `stderr=${f.stderr}\nstdout=${f.stdout}`);
    assert.ok(glbJson(viaFbx).nodes.length >= 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// The script's own argument gate, without Blender: plain python has no bpy, so
// only the checks before the cook run. Skipped when python is absent.
const HAS_PYTHON = spawnSync("python", ["--version"], { encoding: "utf8" }).status === 0;
function pyGate(...args) {
  const script = fileURLToPath(new URL("./blender/cook_char.py", import.meta.url));
  return spawnSync("python", ["-I", script, "--", ...args], { encoding: "utf8" });
}

test("cook_char.py refuses bad arguments with exit 2 and a named code", { skip: HAS_PYTHON ? false : "python not on PATH" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-py-"));
  try {
    const bvh = join(dir, "a.bvh");
    writeFileSync(bvh, "x");
    const out = join(dir, "o.glb");
    const gate = (args, code) => {
      const r = pyGate(...args);
      assert.equal(r.status, 2, `${code}: ${r.stderr}`);
      assert.match(r.stderr, new RegExp(`^${code} `));
    };
    gate([], "E_COOK_USAGE");
    gate([bvh], "E_COOK_USAGE");
    gate([bvh, out, "extra"], "E_COOK_USAGE");
    gate([join(dir, "missing.bvh"), out], "E_COOK_INPUT");
    gate([bvh, join(dir, "o.fbx")], "E_COOK_FORMAT");
    const obj = join(dir, "a.obj");
    writeFileSync(obj, "x");
    gate([obj, out], "E_COOK_FORMAT");
    // Good arguments but no bpy in plain python: a cook failure, not a crash.
    const r = pyGate(bvh, out);
    assert.equal(r.status, 3);
    assert.match(r.stderr, /^E_COOK_NO_BPY /);
    assert.equal(existsSync(out), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
