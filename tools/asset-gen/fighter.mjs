// A realistic fighter, into the game, in one command: a Microsoft Rocketbox
// avatar (MIT) with Mixamo sword-and-shield mocap and the Roman kit, cooked
// into content/fighters/rocketbox where the arena binds it (E2040_BODIES=rocketbox).
//
//   node tools/asset-gen/fighter.mjs --avatar Professions/Military_Male_01 --name rb_legionary
//       [--kit roman|none] [--pack C:\forge-media\mixamo\sword_and_shield]
//       [--clip attack1=<file.fbx>]... [--home C:\forge-media\rocketbox]
//       [--out content/fighters/rocketbox] [--render <strip.png>] [--prove] [--dry-run]
//
// Steps, in order; the first that fails stops the chain and names itself:
//   1. fetch   tools/asset-gen/rocketbox.mjs (skipped when the FBX is home)
//   2. kit     tools/blender/roman_kit.py -> content/generated/roman_kit (skipped when built)
//   3. cook    tools/blender/rocketbox_fighter.py: body + six clips on one skeleton,
//              the manifest row merged in (the manifest is the game's variant list)
//   4. check   the manifest row against the files: body sha, joints, clips, the name
//   5. prove   (--prove) the real window: target/debug/arena.exe with
//              E2040_BODIES=rocketbox captures frames; the run must exit 0 and its
//              "arena capture:" line must say view=ok. A body at the wrong scale,
//              a camera inside a body or staged boxes instead of the soldier fail.
//
// Look at the captured frames afterwards. The checks catch the broken cases;
// they do not say the fighter looks good.
// Exit 0 done, 1 a step failed, 2 usage, 3 the check or the proof said no.
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, unlinkSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { parseAvatar, DEFAULT_HOME } from "./rocketbox.mjs";
import { auditDir, render } from "../frame-audit.mjs";

export const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
export const ROLES = ["idle", "run", "attack1", "attack2", "attack3", "death"];
export const MIN_JOINTS = 45; // crates/game/src/fighter_bind.rs ROCKETBOX_MIN_JOINTS
export const DEFAULT_PACK = "C:\\forge-media\\mixamo\\sword_and_shield";
export const DEFAULT_OUT = join("content", "fighters", "rocketbox");
export const KIT_DIR = join("content", "generated", "roman_kit");

// The proven role -> Mixamo file map (rb_legionary, 2026-10-22): weight in
// the slash, a shield-led attack, a second slash, a real death.
export const SWORD_AND_SHIELD = Object.freeze({
  idle: "sword and shield idle.fbx",
  run: "sword and shield run.fbx",
  attack1: "sword and shield slash.fbx",
  attack2: "sword and shield attack.fbx",
  attack3: "sword and shield slash (2).fbx",
  death: "sword and shield death.fbx",
});

const USAGE = `usage: node tools/asset-gen/fighter.mjs --avatar <Group>/<Name> --name <variant>
  [--kit roman|none] [--pack <mixamo pack dir>] [--clip <role>=<fbx>]...
  [--home <rocketbox home>] [--out <family dir>] [--render <png>] [--prove] [--dry-run]
  roles: ${ROLES.join(", ")}; variant: lowercase letters, digits and _, a letter first`;

// ------------------------------------------------------------------ pure parts

export function argValue(argv, name, fallback = null) {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
}

export function argValues(argv, name) {
  const out = [];
  argv.forEach((a, i) => { if (a === `--${name}` && argv[i + 1]) out.push(argv[i + 1]); });
  return out;
}

// The game's binder takes only these names (fighter_bind.rs rocketbox_variants).
export function checkName(name) {
  if (!/^[a-z][a-z0-9_]*$/.test(name || "")) {
    throw new Error(`--name takes lowercase letters, digits and _, a letter first, not ${name}`);
  }
  return name;
}

// role -> absolute FBX path: the pack's default file per role, each --clip
// role=file overriding (a bare file name is looked up in the pack).
export function clipMap(pack, overrides = []) {
  const map = Object.fromEntries(ROLES.map((r) => [r, join(pack, SWORD_AND_SHIELD[r])]));
  for (const spec of overrides) {
    const [role, ...rest] = spec.split("=");
    const file = rest.join("=");
    if (!ROLES.includes(role) || !file) throw new Error(`--clip takes <role>=<fbx> with role one of ${ROLES.join(", ")}, not ${spec}`);
    map[role] = /[\\/]/.test(file) ? file : join(pack, file);
  }
  return map;
}

export function plan({ root = ROOT, blender, node = process.execPath, avatar, name, home, pack, clips, kit, out, render }) {
  const fbx = join(home, avatar.name, "Export", `${avatar.name}.fbx`);
  const headless = ["-b", "--factory-startup", "--python-exit-code", "1"];
  const kitDir = join(root, KIT_DIR);
  const steps = [{
    label: "fetch (rocketbox.mjs)", command: node,
    args: [join(root, "tools", "asset-gen", "rocketbox.mjs"), "--avatar", `${avatar.group}/${avatar.name}`, "--home", home],
    skipIf: fbx,
  }];
  if (kit === "roman") {
    steps.push({
      label: "kit (roman_kit.py)", command: blender,
      args: [...headless, "-P", join(root, "tools", "blender", "roman_kit.py"), "--", kitDir],
      skipIf: join(kitDir, "scutum.glb"),
    });
  }
  const cook = [...headless, "-P", join(root, "tools", "blender", "rocketbox_fighter.py"), "--",
    "--target", fbx, "--textures", join(home, avatar.name, "Textures"), "--name", name, "--out-dir", out];
  for (const role of ROLES) cook.push("--clip", `${role}=${clips[role]}`);
  if (kit === "roman") cook.push("--weapon", join(kitDir, "gladius.glb"), "--shield", join(kitDir, "scutum.glb"));
  if (render) cook.push("--render", render);
  steps.push({ label: "cook (rocketbox_fighter.py)", command: blender, args: cook });
  return steps;
}

// The manifest row against the files on disk; a list of what is wrong.
export function checkFamily(manifest, name, readFile) {
  const wrong = [];
  const rows = (manifest && manifest.variants) || [];
  const names = rows.map((r) => r.name);
  if (new Set(names).size !== names.length) wrong.push(`a variant is named twice: ${names.join(", ")}`);
  const row = rows.find((r) => r.name === name);
  if (!row) return [...wrong, `${name} has no manifest row`];
  if (!(row.joints >= MIN_JOINTS)) wrong.push(`${name} joints ${row.joints} < ${MIN_JOINTS}`);
  if (row.clips !== ROLES.length) wrong.push(`${name} clips ${row.clips} != ${ROLES.length}`);
  if (!(row.tris_lod0 > 0)) wrong.push(`${name} has no triangles`);
  const body = readFile(join("glb", `${name}_body.glb`));
  if (!body) wrong.push(`glb/${name}_body.glb is missing`);
  else if (createHash("sha256").update(body).digest("hex") !== row.body_glb_sha256) {
    wrong.push(`glb/${name}_body.glb does not match its manifest sha`);
  }
  for (const role of ROLES) {
    if (!readFile(join("glb", "clips", `${name}_${role}.glb`))) wrong.push(`glb/clips/${name}_${role}.glb is missing`);
  }
  return wrong;
}

// The verdict of an arena run: the exit code and every "arena capture:" line
// must say view=ok; a run that fell back to staged boxes says so.
export function proofVerdict(status, stderr) {
  const text = String(stderr || "");
  const lines = text.split(/\r?\n/).filter((l) => l.includes("arena capture:"));
  const wrong = [];
  if (status !== 0) wrong.push(`arena exit ${status}`);
  if (/staged boxes/.test(text)) wrong.push("the look fell back to staged boxes");
  if (lines.length === 0) wrong.push("no 'arena capture:' line (no frame was captured)");
  for (const line of lines) {
    const view = /view=(\S+)/.exec(line);
    if (!view) wrong.push(`capture line has no view verdict: ${line.trim()}`);
    else if (view[1] !== "ok") wrong.push(`view ${view[1]}`);
  }
  return { ok: wrong.length === 0, wrong, lines };
}

// ------------------------------------------------------------------ the run

function blender() {
  if (process.env.BLENDER && existsSync(process.env.BLENDER)) return process.env.BLENDER;
  for (const v of ["5.2", "5.0", "4.2"]) {
    const p = `C:\\Program Files\\Blender Foundation\\Blender ${v}\\blender.exe`;
    if (existsSync(p)) return p;
  }
  throw new Error("no Blender found: set BLENDER to the blender.exe path");
}

function prove(root, name) {
  const exe = join(root, "target", "debug", process.platform === "win32" ? "arena.exe" : "arena");
  if (!existsSync(exe)) {
    return { ok: false, wrong: [`${exe} is not built: cargo build -p engine-game --features app --bin arena`], lines: [] };
  }
  let sha = "unknown";
  try { sha = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: root }).toString().trim(); } catch { /* no git */ }
  const dir = join(root, "target", "clips", name);
  mkdirSync(dir, { recursive: true });
  // Frames from an older run would sit beside this run's and be looked at
  // as if they were new: clear this folder's frames first, by exact name.
  for (const f of readdirSync(dir)) if (/^frame_\d+\.png$/.test(f)) unlinkSync(join(dir, f));
  const args = ["--scene", "arena", "--fighters", "2", "--frames", "240", "--exit",
    "--capture-dir", dir, "--capture-every", "60", "--fps", "30", "--sha", sha];
  console.log(`FIGHTER prove: E2040_BODIES=rocketbox ${exe} ${args.join(" ")}`);
  const run = spawnSync(exe, args, { cwd: root, env: { ...process.env, E2040_BODIES: "rocketbox" }, timeout: 300000, encoding: "utf8" });
  const verdict = proofVerdict(run.status, run.stderr);
  verdict.dir = dir;
  return verdict;
}

export function main(argv = process.argv) {
  if (argv.includes("--help") || argv.includes("-h")) { console.log(USAGE); return 0; }
  let avatar, name, clips, steps, out;
  const home = argValue(argv, "home", DEFAULT_HOME);
  const pack = argValue(argv, "pack", DEFAULT_PACK);
  const kit = argValue(argv, "kit", "roman");
  try {
    avatar = parseAvatar(argValue(argv, "avatar"));
    name = checkName(argValue(argv, "name"));
    if (!["roman", "none"].includes(kit)) throw new Error(`--kit takes roman or none, not ${kit}`);
    clips = clipMap(pack, argValues(argv, "clip"));
    out = resolve(ROOT, argValue(argv, "out", DEFAULT_OUT));
    const dry = argv.includes("--dry-run");
    steps = plan({ blender: dry ? "blender" : blender(), avatar, name, home, pack, clips, kit, out, render: argValue(argv, "render") && resolve(ROOT, argValue(argv, "render")) });
  } catch (err) {
    console.error(`fighter: ${err.message}\n${USAGE}`);
    return 2;
  }
  if (argv.includes("--dry-run")) {
    for (const s of steps) console.log(`${s.label}${s.skipIf && existsSync(s.skipIf) ? " (skip: built)" : ""}\n  ${s.command} ${s.args.join(" ")}`);
    return 0;
  }
  const missing = ROLES.filter((r) => !existsSync(clips[r])).map((r) => `${r}: ${clips[r]}`);
  if (missing.length) {
    console.error(`fighter: clip files missing (fetch the pack: node tools/asset-gen/mixamo.mjs --pack "Sword and Shield Pack"):\n  ${missing.join("\n  ")}`);
    return 1;
  }
  for (const step of steps) {
    if (step.skipIf && existsSync(step.skipIf)) { console.log(`FIGHTER ${step.label}: skip, built`); continue; }
    console.log(`FIGHTER ${step.label}`);
    const run = spawnSync(step.command, step.args, { cwd: ROOT, stdio: "inherit" });
    if (run.status !== 0) { console.error(`fighter: step "${step.label}" failed (exit ${run.status})`); return 1; }
  }
  const manifest = JSON.parse(readFileSync(join(out, "manifest.json"), "utf8"));
  const wrong = checkFamily(manifest, name, (rel) => (existsSync(join(out, rel)) ? readFileSync(join(out, rel)) : null));
  if (wrong.length) { console.error(`fighter: check said no:\n  ${wrong.join("\n  ")}`); return 3; }
  const row = manifest.variants.find((r) => r.name === name);
  console.log(`FIGHTER ${name} joints=${row.joints} tris=${row.tris_lod0} clips=${row.clips} variants=${manifest.variants.map((r) => r.name).join(",")}`);
  if (argv.includes("--prove")) {
    const verdict = prove(ROOT, name);
    for (const line of verdict.lines) console.log(line.trim());
    if (!verdict.ok) { console.error(`fighter: the window proof said no:\n  ${verdict.wrong.join("\n  ")}`); return 3; }
    console.log(`FIGHTER proof ok: frames in ${verdict.dir} - look at them`);
    // Advisory beside view=ok: the picture laws (shadow, material, scale, attach) over the same frames.
    try {
      console.log(`FIGHTER audit: ${render(auditDir(verdict.dir))}`);
    } catch (err) {
      console.log(`FIGHTER audit: not run (${err.message})`);
    }
  } else {
    console.log("FIGHTER next: --prove (the real window, view=ok), then node tools/gh-check.mjs cargo test -p engine-game --features app --lib rocketbox");
  }
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
