// One command, one character. Two lanes, the same gates.
//
// --from mpfb (the lane the game uses now): a real CC0 body, our proportions,
// our style, the 30 m gate, our 22-bone rig, and the page's budget check.
//
//   node tools/asset-gen/character.mjs --from mpfb --name probe_recruit --faction ashkin \
//     [--height 1.9] [--bulk 1.0] [--tris 2000] [--out target/chargen/probe_recruit]
//     [--proxy proxy741] [--clothes male_worksuit01] [--hair short02] [--shoes shoes01]
//   (MPFB asset names; "none" leaves one off. The proxy is the low-poly body.)
//
//   tools/blender/mpfb_body.py        MakeHuman body, headless (CC0)
//   tools/blender/proportion_pass.py  our caricature bands, per faction
//   tools/blender/style_cook.py       our palette, our tri budget, LODs, atlas
//   tools/blender/mesh_silhouette.py  the 30 m read test (fill band)
//   tools/blender/rig22.py            our 22-bone template
//
// --from rocketbox (realistic people, Microsoft Rocketbox avatars, MIT):
//
//   node tools/asset-gen/character.mjs --from rocketbox --avatar Adults/Male_Adult_08 --name rb_male08 \
//     [--home C:\forge-media\rocketbox] [--tris0 7400] [--out target/chargen/rb_male08]
//   (Group is Adults, Children or Professions. The FBX and textures are fetched
//   once into --home, resumable; nothing downloaded goes into the repo.)
//
//   tools/asset-gen/rocketbox.mjs     fetch the avatar (MIT, resumable, 3 MB/s cap)
//   tools/blender/rocketbox_cook.py   bone names, PNG maps, LOD0-3 GLBs, hero.png
//   tools/blender/mesh_silhouette.py  the 30 m read test (fill band)
//   the page's budget check           the same budgetCheck as the MPFB lane
//
// --concept (the older image lane, unchanged):
//
//   node tools/asset-gen/character.mjs --concept content/generated/concepts/ashkin_gunner_02.jpg \
//     --name ashkin_gunner --faction ashkin --out content/generated/characters/ashkin_gunner
//
//   tools/asset-gen/sf3d.mjs            the one local 3D lane
//   tools/blender/style_cook.py         our palette, our budget, our LODs
//   tools/blender/mesh_silhouette.py    the 30 m read test
//   tools/blender/rig22.py              our 22-bone template
//
// The gate has teeth: a silhouette that does not read, or a cook over the
// page's budget, stops the chain and says why. A step that fails names the
// step. The receipt is written whenever the chain gets to the gate.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseAvatar, DEFAULT_HOME } from "./rocketbox.mjs";

export const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));

// The gate band, calibrated on the six blockouts: the two heavies that read
// as rectangles scored 0.7443 and 0.7519, the readable ones 0.5283 to 0.6604.
export const FILL_MIN = 0.40;
export const FILL_MAX = 0.70;

// The Rocketbox lane has its own band. A realistic avatar is measured as its
// FBX stands it, arms out in a T-pose, so the bounding box is wide and the fill
// is low: the two proven avatars measure 0.30 (Male_Adult_08) and 0.35
// (Military_Male_01), both under the blockout floor. The floor sits 0.05 under
// the sparser one. The ceiling stays at 0.70, so a rectangle (0.7443) still fails.
export const ROCKETBOX_BAND = Object.freeze({ min: 0.25, max: 0.70 });

export const FACTIONS = ["ashkin", "wardens"];

// The page's budgets, as the Rust side states them:
//   atlas edge:   crates/assets/src/rig_cook.rs ATLAS_MAX_DIM (256)
//   first bundle: crates/assets/tests/it/rig_cook.rs `total < 2_040_000`
//   memory:       commit d16b5bab6, 19,988,480 B of linear memory
export const PAGE = Object.freeze({
  atlasMaxDim: 256,
  firstBundleBytes: 2040000,
  memoryBytes: 19988480,
});

export const NEXT_STEP_ROCKETBOX =
  "this lane is the look study (LODs, the 30 m gate, the budget); it does not reach the game. " +
  "The game's realistic fighter comes from node tools/asset-gen/fighter.mjs --avatar <Group>/<Name> " +
  "--name <variant> --prove (body, Mixamo clips, Roman kit, manifest row, the window proof).";

export const NEXT_STEP =
  "not wired into the game: nothing puts a cooked MPFB body into content/fighters " +
  "(no glb/ file, no manifest.json row), so fighters.e2bundle is never rebuilt from it. " +
  "The fighter bin (crates/assets/src/bin/fighter_bundle.rs) reads manifest.json, and the " +
  "rig cook (crates/assets/src/rig_cook.rs) expects QUADRANTS = 4 parts, while style_cook " +
  "writes 3 materials (skin, cloth, hair). Missing: the step that makes the 4-part body " +
  "and writes its manifest row.";

// ------------------------------------------------------------------ pure parts

export function argValue(argv, name, fallback = null) {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
}

// Which lane: --from wins; without it, a --concept means the concept lane.
export function chooseLane(argv) {
  const from = argValue(argv, "from");
  if (from === null) return argValue(argv, "concept") !== null ? "concept" : null;
  if (from === "concept" || from === "mpfb" || from === "rocketbox") return from;
  throw new Error(`--from takes concept, mpfb or rocketbox, not ${from}`);
}

export function gateVerdict(fill, band = { min: FILL_MIN, max: FILL_MAX }) {
  if (fill >= band.min && fill <= band.max) return "pass";
  return fill > band.max ? "fail_too_solid" : "fail_too_sparse";
}

// The steps of the MPFB lane, in order, as commands. Each names the file it
// must leave behind. Pure: no file access.
export function mpfbPlan({ root = ROOT, blender, out, faction, height, bulk, tris, assets = {} }) {
  const tools = join(root, "tools", "blender");
  const body = join(out, "body", "body.glb");
  const prop = join(out, "proportion", `${faction}.glb`);
  const cook = join(out, "cook");
  const gate = join(out, "gate");
  const rig = join(out, "rig", "rigged.glb");
  const headless = ["-b", "--factory-startup", "--python-exit-code", "1"];
  return [
    {
      label: "body (MPFB, headless)",
      command: blender,
      // No --factory-startup: the MPFB extension lives in the user's
      // extensions folder, the same way the proven body was made.
      args: ["-b", "--python-exit-code", "1", "--python", join(tools, "mpfb_body.py"),
        "--", body, "--height", String(height), "--bulk", String(bulk),
        ...Object.entries(assets).filter(([, v]) => v).flatMap(([k, v]) => [`--${k}`, String(v)])],
      writes: body,
    },
    {
      label: "proportion pass",
      command: blender,
      args: [...headless, "-P", join(tools, "proportion_pass.py"),
        "--", body, prop, "--faction", faction],
      writes: prop,
    },
    {
      label: "style cook",
      command: blender,
      args: [...headless, "-P", join(tools, "style_cook.py"),
        "--", prop, cook, "--faction", faction, "--tris", String(tris)],
      writes: join(cook, "cook.json"),
    },
    {
      label: "30 m gate",
      command: blender,
      args: [...headless, "-P", join(tools, "mesh_silhouette.py"),
        "--", join(cook, "lod0.glb"), gate],
      writes: join(gate, "silhouette.json"),
    },
    {
      label: "22-bone rig",
      command: blender,
      args: [...headless, "-P", join(tools, "rig22.py"),
        "--", join(cook, "lod0.glb"), rig],
      writes: rig,
    },
  ];
}

// The steps of the Rocketbox lane, in order. The fetch is the one step that
// touches the network, and it is resumable: a file already on disk at its
// listed size is not downloaded again.
export function rocketboxPlan({ root = ROOT, blender, node = process.execPath, home, out, avatar, tris0 }) {
  const tools = join(root, "tools", "blender");
  const fbx = join(home, avatar.name, "Export", `${avatar.name}.fbx`);
  const cook = join(out, "cook");
  const gate = join(out, "gate");
  const headless = ["-b", "--factory-startup", "--python-exit-code", "1"];
  return [
    {
      label: "fetch (rocketbox.mjs)",
      command: node,
      args: [join(root, "tools", "asset-gen", "rocketbox.mjs"),
        "--avatar", `${avatar.group}/${avatar.name}`, "--home", home],
      writes: fbx,
      inherit: true,
    },
    {
      label: "cook (Blender, bones and maps)",
      command: blender,
      args: [...headless, "-P", join(tools, "rocketbox_cook.py"),
        "--", fbx, cook, "--tris0", String(tris0)],
      writes: join(cook, "cook.json"),
    },
    {
      label: "30 m gate",
      command: blender,
      args: [...headless, "-P", join(tools, "mesh_silhouette.py"),
        "--", join(cook, "lod0.glb"), gate],
      writes: join(gate, "silhouette.json"),
    },
  ];
}

// The PNG's pixel size from its IHDR chunk, or null if it is not a PNG.
export function pngSize(buf) {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (!buf || buf.length < 24 || sig.some((b, i) => buf[i] !== b)) return null;
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

// The images a GLB carries, with their bytes (the cooked LODs embed their atlas).
// `normal` is true for an image a material uses as its normal map, which the
// rig cook writes as BC5 instead of BC1.
export function glbImages(buf) {
  if (buf.length < 20 || buf.readUInt32LE(0) !== 0x46546c67) throw new Error("not a GLB file");
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
  const binStart = 20 + jsonLen + 8;
  const views = json.bufferViews || [];
  const textures = json.textures || [];
  const normalImages = new Set((json.materials || [])
    .filter((m) => m.normalTexture)
    .map((m) => textures[m.normalTexture.index]?.source)
    .filter((source) => source !== undefined));
  return (json.images || [])
    .map((image, index) => ({ image, index }))
    .filter(({ image }) => image.bufferView !== undefined)
    .map(({ image, index }) => {
      const view = views[image.bufferView];
      const start = binStart + (view.byteOffset || 0);
      return {
        mimeType: image.mimeType || "",
        bytes: buf.subarray(start, start + view.byteLength),
        normal: normalImages.has(index),
      };
    });
}

// A map's size on the page: its long edge held to the atlas cap, the way the
// rig cook shrinks it (`cook_body_at`, ATLAS_MAX_DIM).
export function cappedDims(a, page = PAGE) {
  const k = Math.min(1, page.atlasMaxDim / Math.max(a.width, a.height, 1));
  return { width: Math.max(1, Math.round(a.width * k)), height: Math.max(1, Math.round(a.height * k)) };
}

// The KTX2 the rig cook writes for one map (crates/assets/src/ktx2_cook.rs):
// an 80 B header, a 24 B index entry per mip level, the DFD (28 B plus 16 per
// channel), then every level down to 1x1 in 4x4 blocks, each level 16-byte
// aligned. Colour is BC1 (8 B a block); a normal map is BC5 (16 B a block).
export function ktx2Bytes(width, height, normal = false) {
  const align = (n) => Math.ceil(n / 16) * 16;
  const levels = 32 - Math.clz32(Math.max(width, height, 1));
  const block = normal ? 16 : 8;
  let total = align(80 + 24 * levels + 28 + 16 * (normal ? 2 : 1));
  for (let level = 0; level < levels; level++) {
    const w = Math.max(width >> level, 1);
    const h = Math.max(height >> level, 1);
    total += align(Math.ceil(w / 4) * Math.ceil(h / 4) * block);
  }
  return total;
}

// The bytes a Rocketbox body ships as: the LOD0 GLB without its embedded PNGs
// (the rig cook drops every image and keeps the rest of the rig), plus the KTX2
// it writes for each map at the atlas cap. `atlases` is [{ width, height, bytes, normal }],
// where bytes is the embedded PNG's size.
export function rocketboxShippedBytes({ lodBytes, atlases }, page = PAGE) {
  const embedded = atlases.reduce((sum, a) => sum + a.bytes, 0);
  const ktx2 = atlases.reduce((sum, a) => {
    const d = cappedDims(a, page);
    return sum + ktx2Bytes(d.width, d.height, Boolean(a.normal));
  }, 0);
  return lodBytes - embedded + ktx2;
}

// The page's budget for one cooked body, with the numbers the Rust rig cook
// measures: the lane is the bytes the body ships as (for the MPFB lane, the
// LOD0 GLB with its atlas inside), and the decoded RGBA of the atlases has to
// fit. The atlas edge is not a failure: the rig cook shrinks every atlas to the
// cap on its way to the page, so memory is counted at the edge the page will
// hold. `atlases` is [{ width, height, bytes }].
export function budgetCheck({ lod0Triangles, trisBudget, laneBytes, atlases }, page = PAGE) {
  const findings = [];
  const maxEdge = atlases.reduce((m, a) => Math.max(m, a.width, a.height), 0);
  const capped = (a) => {
    const d = cappedDims(a, page);
    return d.width * d.height;
  };
  const decoded = atlases.reduce((sum, a) => sum + capped(a) * 4, 0);
  const cookedEdge = Math.min(maxEdge, page.atlasMaxDim);
  if (lod0Triangles > trisBudget) {
    findings.push(`lod0 ${lod0Triangles} tris over the ${trisBudget} tri budget`);
  }
  if (laneBytes >= page.firstBundleBytes) {
    findings.push(`lane ${laneBytes} B at or over the first-bundle budget ${page.firstBundleBytes} B`);
  }
  if (decoded >= page.memoryBytes) {
    findings.push(`decoded atlases ${decoded} B at or over the memory budget ${page.memoryBytes} B`);
  }
  return {
    verdict: findings.length ? "fail" : "pass",
    findings,
    numbers: {
      lod0Triangles, trisBudget, maxEdge, cookedEdge, laneBytes, decodedBytes: decoded,
      atlasCapPx: page.atlasMaxDim, firstBundleBytes: page.firstBundleBytes, memoryBytes: page.memoryBytes,
    },
  };
}

// The receipt, as the one line the chain prints. Pure given a receipt object.
export function receiptLine(r) {
  const tris = r.lods.map((l) => `lod${l.lod} ${l.triangles}`).join(" ");
  const budget = r.budget.verdict === "pass"
    ? "budget pass"
    : `budget fail: ${r.budget.findings.join("; ")}`;
  return `RECEIPT ${r.name} ${r.faction} | tris ${tris} | atlas ${r.budget.numbers.maxEdge} px (page ${r.budget.numbers.cookedEdge}) ` +
    `| fill ${r.gate.bbox_fill} (30 m ${r.gate.fill_30m}) | gate ${r.gate.verdict} ` +
    `| ${budget} | ${r.seconds.total} s`;
}

// The receipt object of an MPFB chain. Pure given its inputs; the file
// sizes of the LODs are passed in so the builder does no I/O.
export function buildReceipt({
  name, faction, tris, seconds, cookJson, lodBytes, silhouette, rig, budget, verdict, stopped,
  band = { min: FILL_MIN, max: FILL_MAX },
}) {
  return {
    name,
    faction,
    tris_budget: tris,
    verdict,
    stopped_at: stopped,
    lods: cookJson ? cookJson.lods.map((l) => ({ lod: l.lod, triangles: l.triangles, bytes: lodBytes[l.lod] })) : [],
    gate: silhouette
      ? {
        verdict: gateVerdict(silhouette.bbox_fill, band),
        bbox_fill: silhouette.bbox_fill,
        fill_30m: silhouette.fill_30m,
        height_30m_px: silhouette.height_30m_px,
        band: [band.min, band.max],
      }
      : null,
    rig: rig ? { bones: rig.bones, weighted_vertices: rig.weighted_vertices, vertices: rig.vertices } : null,
    budget,
    seconds,
    generated_at: new Date().toISOString(),
    next_step: NEXT_STEP,
  };
}

// ------------------------------------------------------------- side effects

function fail(msg) {
  console.error(`character: ${msg}`);
  process.exit(1);
}

function blender() {
  if (process.env.BLENDER && existsSync(process.env.BLENDER)) return process.env.BLENDER;
  const roots = [
    "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe",
    "C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe",
    "C:\\Program Files\\Blender Foundation\\Blender 4.2\\blender.exe",
  ];
  for (const candidate of roots) if (existsSync(candidate)) return candidate;
  fail("no Blender found: set BLENDER to the blender.exe path");
}

function lastLine(text) {
  const lines = String(text || "").split("\n").map((l) => l.trim()).filter(Boolean);
  return (lines[lines.length - 1] || "no output").slice(0, 300);
}

// Blender prints its own noise after the script, so the metrics line is the
// last one that parses as JSON, not the last line there is.
function lastJson(text) {
  const lines = String(text || "").split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (!line.startsWith("{")) continue;
    try { return JSON.parse(line); } catch { /* keep looking */ }
  }
  return null;
}

function elapsed(since) {
  return Number(((Date.now() - since) / 1000).toFixed(1));
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function runMpfbStep(step) {
  console.log(`-> ${step.label}`);
  const started = Date.now();
  let output;
  try {
    output = execFileSync(step.command, step.args, {
      cwd: ROOT,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      // A download shows its progress meter; the Blender steps keep their output.
      stdio: step.inherit ? ["ignore", "inherit", "inherit"] : ["ignore", "pipe", "pipe"],
      timeout: 4 * 60 * 60 * 1000,
    });
  } catch (err) {
    const why = err.status !== undefined && err.status !== null
      ? `exit ${err.status}: ${lastLine(err.stderr || err.stdout)}`
      : err.message.split("\n")[0];
    fail(`${step.label} failed, the chain stops here (${why})`);
  }
  if (!existsSync(step.writes)) fail(`${step.label} exited clean but wrote no ${step.writes}`);
  const seconds = elapsed(started);
  console.log(`   done in ${seconds}s`);
  return { output, seconds };
}

function writeReceipt(out, receipt) {
  writeFileSync(join(out, "receipt.json"), JSON.stringify(receipt, null, 2) + "\n", "utf8");
}

function runMpfb(argv) {
  const name = argValue(argv, "name");
  const faction = argValue(argv, "faction", "ashkin");
  if (!name) fail("--from mpfb needs --name <id>");
  if (!FACTIONS.includes(faction)) fail(`--faction must be ${FACTIONS.join(" or ")}, not ${faction}`);
  const height = Number(argValue(argv, "height", "1.9"));
  const bulk = Number(argValue(argv, "bulk", "1.0"));
  const tris = Number(argValue(argv, "tris", "2000"));
  if (!(height > 0) || !(bulk > 0) || !(tris > 0)) fail("--height, --bulk and --tris must be positive numbers");

  const out = resolve(ROOT, argValue(argv, "out", `content/generated/characters/${name}`));
  for (const dir of ["body", "proportion", "cook", "gate", "rig"]) {
    mkdirSync(join(out, dir), { recursive: true });
  }
  const started = Date.now();
  const seconds = {};
  const outputs = {};
  const assets = {};
  for (const key of ["proxy", "clothes", "hair", "shoes"]) assets[key] = argValue(argv, key, null);
  const plan = mpfbPlan({ blender: blender(), out, faction, height, bulk, tris, assets });
  for (const step of plan) {
    const result = runMpfbStep(step);
    seconds[step.label] = result.seconds;
    outputs[step.label] = result.output;
    if (step.label === "30 m gate") {
      const silhouette = readJson(join(out, "gate", "silhouette.json"));
      const verdict = gateVerdict(silhouette.bbox_fill);
      if (verdict !== "pass") {
        // A silhouette that does not read is not worth a rig, and the
        // receipt still says what was measured.
        seconds.total = elapsed(started);
        const receipt = buildReceipt({
          name, faction, tris, seconds, cookJson: readJson(join(out, "cook", "cook.json")),
          lodBytes: lodByteSizes(out), silhouette, rig: null, budget: null,
          verdict: verdict, stopped: step.label,
        });
        writeReceipt(out, receipt);
        console.error(`character: the silhouette does not read (${verdict}). ` +
          `Look at ${join(out, "gate", "silhouette_30m.png")} before painting anything.`);
        process.exit(3);
      }
    }
  }

  const cookJson = readJson(join(out, "cook", "cook.json"));
  const silhouette = readJson(join(out, "gate", "silhouette.json"));
  const lod0 = readFileSync(join(out, "cook", "lod0.glb"));
  const atlases = glbImages(lod0).map((image) => {
    const size = pngSize(image.bytes);
    if (!size) fail(`lod0 carries a non-PNG image (${image.mimeType}), the budget cannot be read`);
    return { ...size, bytes: image.bytes.length };
  });
  const budget = budgetCheck({
    lod0Triangles: cookJson.lods[0].triangles,
    trisBudget: tris,
    laneBytes: lod0.length,
    atlases,
  });
  const rig = lastJson(outputs["22-bone rig"]);
  seconds.total = elapsed(started);
  const receipt = buildReceipt({
    name, faction, tris, seconds, cookJson, lodBytes: lodByteSizes(out), silhouette,
    rig: rig ? { bones: rig.bones, weighted_vertices: rig.weighted_vertices, vertices: rig.vertices } : null,
    budget, verdict: budget.verdict === "pass" ? "pass" : "budget_fail", stopped: null,
  });
  writeReceipt(out, receipt);
  console.log(receiptLine(receipt));
  console.log(`WROTE ${join(out, "receipt.json")}`);
  console.log(`next step: ${NEXT_STEP}`);
  if (budget.verdict !== "pass") {
    console.error(`character: the cook is over the page's budget: ${budget.findings.join("; ")}`);
    process.exit(3);
  }
}

// The page's budget for a cooked lod0, read from the GLB the cook wrote. The
// lane is what the page gets: the GLB without its 1024 px PNGs, plus the 256 px
// KTX2 the rig cook makes from each of them.
function rocketboxBudget(out, tris) {
  const cookJson = readJson(join(out, "cook", "cook.json"));
  const lod0 = readFileSync(join(out, "cook", "lod0.glb"));
  const atlases = glbImages(lod0).map((image) => {
    const size = pngSize(image.bytes);
    if (!size) fail(`lod0 carries a non-PNG image (${image.mimeType}), the budget cannot be read`);
    return { ...size, bytes: image.bytes.length, normal: image.normal };
  });
  return budgetCheck({
    lod0Triangles: cookJson.lods[0].triangles,
    trisBudget: tris,
    laneBytes: rocketboxShippedBytes({ lodBytes: lod0.length, atlases }),
    atlases,
  });
}

function runRocketbox(argv) {
  const name = argValue(argv, "name");
  const avatarSpec = argValue(argv, "avatar");
  if (!name) fail("--from rocketbox needs --name <id>");
  if (!avatarSpec) fail("--from rocketbox needs --avatar <Group>/<Name>, e.g. Adults/Male_Adult_08");
  let avatar;
  try {
    avatar = parseAvatar(avatarSpec);
  } catch (err) {
    fail(err.message);
  }
  const home = resolve(argValue(argv, "home", DEFAULT_HOME));
  const tris = Number(argValue(argv, "tris0", "7400"));
  if (!(tris > 0)) fail("--tris0 must be a positive number");

  const out = resolve(ROOT, argValue(argv, "out", `target/chargen/${name}`));
  mkdirSync(join(out, "cook"), { recursive: true });
  mkdirSync(join(out, "gate"), { recursive: true });
  const started = Date.now();
  const seconds = {};
  const outputs = {};
  const plan = rocketboxPlan({ blender: blender(), home, out, avatar, tris0: tris });
  for (const step of plan) {
    const result = runMpfbStep(step);
    seconds[step.label] = result.seconds;
    outputs[step.label] = result.output;
    if (step.label === "30 m gate") {
      const silhouette = readJson(join(out, "gate", "silhouette.json"));
      const verdict = gateVerdict(silhouette.bbox_fill, ROCKETBOX_BAND);
      if (verdict !== "pass") {
        // The receipt still carries the budget numbers, so the owner sees both.
        seconds.total = elapsed(started);
        const receipt = buildReceipt({
          name, faction: "rocketbox", tris, seconds, cookJson: readJson(join(out, "cook", "cook.json")),
          lodBytes: lodByteSizes(out), silhouette, rig: null, budget: rocketboxBudget(out, tris),
          verdict, stopped: step.label, band: ROCKETBOX_BAND,
        });
        receipt.next_step = NEXT_STEP_ROCKETBOX;
        writeReceipt(out, receipt);
        console.log(receiptLine(receipt));
        console.error(`character: the silhouette does not read (${verdict}). ` +
          `Look at ${join(out, "gate", "silhouette_30m.png")} before painting anything.`);
        process.exit(3);
      }
    }
  }

  const cookJson = readJson(join(out, "cook", "cook.json"));
  const silhouette = readJson(join(out, "gate", "silhouette.json"));
  const budget = rocketboxBudget(out, tris);
  seconds.total = elapsed(started);
  const receipt = buildReceipt({
    name, faction: "rocketbox", tris, seconds, cookJson, lodBytes: lodByteSizes(out), silhouette,
    rig: null, budget, verdict: budget.verdict === "pass" ? "pass" : "budget_fail", stopped: null,
    band: ROCKETBOX_BAND,
  });
  receipt.next_step = NEXT_STEP_ROCKETBOX;
  receipt.avatar = { group: avatar.group, name: avatar.name, licence: cookJson.licence, sha256: cookJson.source.sha256 };
  writeReceipt(out, receipt);
  console.log(receiptLine(receipt));
  console.log(`WROTE ${join(out, "receipt.json")}`);
  console.log(`hero: ${join(out, "cook", "hero.png")}`);
  console.log(`next step: ${NEXT_STEP_ROCKETBOX}`);
  if (budget.verdict !== "pass") {
    console.error(`character: the cook is over the page's budget: ${budget.findings.join("; ")}`);
    process.exit(3);
  }
}

// The byte size of each LOD file the cook wrote, keyed by LOD number.
function lodByteSizes(out) {
  const sizes = {};
  for (let lod = 0; lod < 4; lod++) {
    const path = join(out, "cook", `lod${lod}.glb`);
    if (existsSync(path)) sizes[lod] = statSync(path).size;
  }
  return sizes;
}

function runConcept(argv) {
  const concept = argValue(argv, "concept");
  const name = argValue(argv, "name");
  const faction = argValue(argv, "faction", "ashkin");
  const outDir = resolve(ROOT, argValue(argv, "out", `content/generated/characters/${name || "unnamed"}`));
  const tris = argValue(argv, "tris", "2000");
  const texture = argValue(argv, "texture", "256");
  if (!concept || !name) fail("--concept <image> and --name <id> are required");

  const conceptPath = resolve(ROOT, concept);
  if (!existsSync(conceptPath)) fail(`no concept image at ${conceptPath}`);

  const BLENDER = blender();

  mkdirSync(outDir, { recursive: true });
  const meshDir = join(outDir, "mesh");
  const cookDir = join(outDir, "cook");
  const gateDir = join(outDir, "gate");
  const rigDir = join(outDir, "rig");

  function run(label, command, args) {
    console.log(`-> ${label}`);
    const started = Date.now();
    const output = execFileSync(command, args, {
      cwd: ROOT,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 4 * 60 * 60 * 1000,
    });
    const seconds = ((Date.now() - started) / 1000).toFixed(1);
    console.log(`   done in ${seconds}s`);
    return { output, seconds: Number(seconds) };
  }

  // 1. the base mesh, from the one local 3D lane
  const mesh = run("base mesh (sf3d)", process.execPath, [
    join(ROOT, "tools", "asset-gen", "sf3d.mjs"),
    "--image", conceptPath, "--out", meshDir, "--texture", texture,
  ]);
  const baseMesh = join(meshDir, "mesh.glb");
  if (!existsSync(baseMesh)) fail(`sf3d wrote no mesh.glb in ${meshDir}`);

  // 2. our style: palette, budget, atlas, AO, LODs
  const cook = run("style cook", BLENDER, [
    "-b", "--factory-startup", "--python-exit-code", "1",
    "-P", join(ROOT, "tools", "blender", "style_cook.py"),
    "--", baseMesh, cookDir, "--faction", faction, "--tris", tris,
  ]);
  const cookJson = readJson(join(cookDir, "cook.json"));

  // 3. the gate
  const gate = run("30 m gate", BLENDER, [
    "-b", "--factory-startup", "--python-exit-code", "1",
    "-P", join(ROOT, "tools", "blender", "mesh_silhouette.py"),
    "--", join(cookDir, "lod0.glb"), gateDir,
  ]);
  const silhouette = readJson(join(gateDir, "silhouette.json"));

  const fill = silhouette.bbox_fill;
  const gateVerdictValue = gateVerdict(fill);

  // 4. our rig
  const rig = run("22-bone rig", BLENDER, [
    "-b", "--factory-startup", "--python-exit-code", "1",
    "-P", join(ROOT, "tools", "blender", "rig22.py"),
    "--", join(cookDir, "lod0.glb"), join(rigDir, "rigged.glb"),
  ]);
  const rigMetrics = lastJson(rig.output) ?? { bones: 0, weighted_vertices: 0, vertices: 0, distance_stats: {} };

  const character = {
    name,
    faction,
    concept: conceptPath,
    out: outDir,
    gate: {
      verdict: gateVerdictValue,
      bbox_fill: fill,
      fill_band: [FILL_MIN, FILL_MAX],
      height_30m_px: silhouette.height_30m_px,
      fill_30m: silhouette.fill_30m,
      triangles: silhouette.triangles,
      note: "a silhouette passes when it reads at 90 pixels tall: faction first, class second",
    },
    cook: {
      lods: cookJson.lods,
      atlas: cookJson.atlas,
      ao: cookJson.ao,
      scale_applied: cookJson.scale_applied,
      source_triangles: cookJson.source_triangles,
    },
    rig: {
      bones: rigMetrics.bones,
      weighted_vertices: rigMetrics.weighted_vertices,
      vertices: rigMetrics.vertices,
      distance_stats: rigMetrics.distance_stats,
      landmarks: rigMetrics.landmarks,
    },
    seconds: { mesh: mesh.seconds, cook: cook.seconds, gate: gate.seconds, rig: rig.seconds },
    generated_at: new Date().toISOString(),
  };
  writeFileSync(join(outDir, "character.json"), JSON.stringify(character, null, 2) + "\n", "utf8");

  console.log(`\nGATE ${gateVerdictValue}  fill ${fill} (band ${FILL_MIN}-${FILL_MAX})  ` +
    `${silhouette.height_30m_px} px at 30 m  ${cookJson.lods[0].triangles} tris at LOD0`);
  console.log(`WROTE ${join(outDir, "character.json")}`);
  if (gateVerdictValue !== "pass") {
    console.error(`character: the silhouette does not read (${gateVerdictValue}). ` +
      `Look at ${join(gateDir, "silhouette_30m.png")} before painting anything.`);
    process.exit(3);
  }
}

export function main(argv = process.argv) {
  const lane = chooseLane(argv);
  if (lane === "mpfb") return runMpfb(argv);
  if (lane === "rocketbox") return runRocketbox(argv);
  if (lane === "concept") return runConcept(argv);
  fail("--concept <image> (the image lane), --from mpfb --name <id> --faction <f> (the body lane) " +
    "or --from rocketbox --avatar <Group>/<Name> --name <id> (the realistic lane) is required");
}

// Run only as the command, so the tests can import the pure parts.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
