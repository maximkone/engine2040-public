// weekly-clip.mjs — ONE 20 s 16:9 arena duel clip per week, delivered to marketing-studio.
//
// Wraps the pieces that already exist (SHOW1 capture in arena.exe, stitching in
// tools/showcase.mjs) into the weekly wrapper marketing-studio consumes as
// "engine2040 replays": find arena.exe at HEAD, capture the fixed tape,
// stitch MP4+GIF, skip pictures that did not move, write clip.json, and with
// --deliver copy the three files into the studio's from-engine2040/<week>/
// folder plus one order-book row set to delivered. No posting, no network.
//
// Usage from the repo root:
//   node tools/weekly-clip.mjs --week 2026W40 [--deliver] [--clip <dir>]
//   --week <year>W<week>  e.g. 2026W40 (required, selects target/clips/<week>)
//   --deliver             copy + order + mark delivered (default: capture only)
//   --clip <dir>          fixture capture dir (12+ PNGs + manifest.json): skips
//                         the arena build + run; used by tests and proof runs
//   -h, --help            print help and exit 0
//
// Steps (one line each on stdout): ARENA, CAPTURE, STITCH, MOVED, CLIP-JSON,
// then DELIVERED / NOT DELIVERED / READY, or NOOP. Exit codes: 0 ok (NOOP and
// READY included) · 2 usage · 3 arena.exe missing or stale · 4 capture
// failure · 5 stitch failure · 6 deliver failure. This tool never runs cargo:
// a missing or stale arena.exe prints the exact build command and exits 3;
// the pilot-clip seat builds it.
//
// Env overrides (tests and seats; production uses the defaults):
//   WEEKLY_CLIP_FFMPEG     ffmpeg binary (default: "ffmpeg" on PATH)
//   WEEKLY_CLIP_EMPIRE     empire.mjs path (default: the center checkout)
//   WEEKLY_CLIP_STUDIO_DIR marketing-studio repo dir (default: the checkout)
//   WEEKLY_CLIP_ORDERS_FILE orders.csv path (default: <repoRoot>/orders.csv)
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";

const WEEK_RE = /^\d{4}W\d{2}$/;
const CLIPS = "target/clips";
const ROWS = ["SHOW1"];
const SEED = 2040;
const FRAMES = 600;
const EVERY = 1;
const FPS = 30;
const CAMERA_PATH = "arena600";
const DEADLINE_MS = 600000;
const BUILD_CMD = "cargo build -p engine-game --bin arena";
const DEFAULT_EMPIRE = "C:/empire/center/empire.mjs";
const DEFAULT_STUDIO = "C:/empire/autonomous-factory/marketing/campaign-studio";
const ORDERS_HEADER = "order_id,from_repo,product,brief,status,delivered_path,adopted,date,adopted_commit";

const HELP = `weekly-clip.mjs — one 20 s 16:9 arena duel clip per week for marketing-studio

Usage:
  node tools/weekly-clip.mjs --week <year>W<week> [--deliver] [--clip <dir>]

Steps (one stdout line each): ARENA, CAPTURE, STITCH, MOVED, CLIP-JSON, then
DELIVERED | NOT DELIVERED: <reason> | READY | NOOP picture unchanged since <week>.
Exit codes: 0 ok · 2 usage · 3 arena.exe missing/stale · 4 capture · 5 stitch · 6 deliver.
`;

const fail = (code, message) => {
  process.stderr.write(`weekly-clip: error: ${message}\n`);
  process.exit(code);
};

const say = (line) => process.stdout.write(`${line}\n`);

function parseArgs(argv) {
  const opts = { week: null, deliver: false, clip: null };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--week") {
      opts.week = argv[i + 1] ?? null;
      i += 1;
    } else if (a === "--deliver") opts.deliver = true;
    else if (a === "--clip") {
      opts.clip = argv[i + 1] ?? null;
      i += 1;
    } else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else fail(2, `unknown flag ${a}\n\n${HELP}`);
  }
  if (!opts.week) fail(2, `--week <year>W<week> is required (e.g. --week 2026W40)\n\n${HELP}`);
  if (!WEEK_RE.test(opts.week)) fail(2, `bad --week ${JSON.stringify(opts.week)}: want <year>W<week> like 2026W40`);
  return opts;
}

function run(cmd, args, cwd, extraEnv) {
  const r = spawnSync(cmd, args, {
    encoding: "utf8",
    cwd,
    timeout: 660_000,
    env: { ...process.env, ...extraEnv },
  });
  return { ok: !r.error && r.status === 0, status: r.status, error: r.error ? String(r.error) : "", out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
}

function gitLine(root, args) {
  const r = run("git", args, root);
  return r.ok ? r.out.trim().split("\n")[0].trim() : null;
}

function listPngs(dir) {
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return [];
  }
  return entries.filter((n) => n.toLowerCase().endsWith(".png")).sort();
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

// Newest arena.exe at HEAD: target/debug plus the pool dirs lanes build into.
// Missing, or older than the HEAD commit, means a rebuild is due: print the
// exact build command and exit 3 (the pilot-clip seat builds it).
function findArena(root, headTime) {
  const exe = process.platform === "win32" ? "arena.exe" : "arena";
  const candidates = [join(root, "target", "debug", exe)];
  for (let pool = 1; pool <= 4; pool += 1) candidates.push(join(root, "target", `pool-${pool}`, "debug", exe));
  let best = null;
  for (const path of candidates) {
    try {
      const mtime = statSync(path).mtimeMs;
      if (!best || mtime > best.mtime) best = { path, mtime };
    } catch {
      /* absent candidate */
    }
  }
  if (!best) return { stale: true, reason: "no arena.exe in target/debug or target/pool-*/debug" };
  if (headTime !== null && best.mtime < headTime * 1000) {
    return { stale: true, reason: `${best.path} is older than HEAD` };
  }
  return { stale: false, path: best.path };
}

// Newest earlier week with PNGs: the "last delivered clip" whose last frame
// the new picture is compared against. Nothing earlier means first clip.
function prevWeekDir(root, week) {
  const base = join(root, CLIPS);
  let entries;
  try {
    entries = readdirSync(base, { withFileTypes: true });
  } catch {
    return null;
  }
  const weeks = entries
    .filter((e) => e.isDirectory() && WEEK_RE.test(e.name) && e.name < week)
    .map((e) => e.name)
    .sort();
  for (let i = weeks.length - 1; i >= 0; i -= 1) {
    const dir = join(base, weeks[i]);
    if (listPngs(dir).length > 0) return { week: weeks[i], dir };
  }
  return null;
}

// Frame comparison, preferring tools/frame-moved.mjs when it exists (Tool 4
// of 6 owns it); otherwise a byte-identical sha256 fallback. Returns
// { moved, lookStage, method }.
function compareFrames(root, fresh, prev) {
  const mover = join(root, "tools", "frame-moved.mjs");
  if (existsSync(mover)) {
    const r = run(process.execPath, [mover, "--new", fresh, "--old", prev], root);
    const moved = /MOVED:\s*(\S+)/i.exec(r.out)?.[1] ?? "";
    if (r.ok && moved) {
      const stage = /look_stage:\s*(\S+)/i.exec(r.out)?.[1] ?? moved;
      return { moved: moved.toLowerCase() !== "none", lookStage: stage, method: "frame-moved.mjs" };
    }
    // A broken mover must not block the weekly clip: fall through loudly.
    say(`MOVED: frame-moved.mjs failed (${r.error || `exit ${r.status}`}); internal compare instead`);
  }
  const same = sha256File(fresh) === sha256File(prev);
  return { moved: !same, lookStage: same ? "none" : "moved", method: "internal-sha256" };
}

// The maker's orders.csv: the file `empire.mjs order` appends to (the maker
// repo dir from empire.json), so the tool updates the same row it ordered.
function defaultOrdersFile(empireJs, root) {
  try {
    const repos = JSON.parse(readFileSync(join(dirname(empireJs), "empire.json"), "utf8")).repos ?? {};
    const maker = repos.engine2040;
    const dir = typeof maker === "string" ? maker : maker?.dir;
    if (dir) return join(dir, "orders.csv");
  } catch {
    /* fall through to the repo root */
  }
  return join(root, "orders.csv");
}
const normBrief = (s) => String(s ?? "").toLowerCase().replace(/\s+/g, " ").trim();
const briefHead = (b) => String(b ?? "").split(" | ")[0];

function readOrders(path) {
  if (!existsSync(path)) return { header: null, rows: [], eol: "\n" };
  const raw = readFileSync(path, "utf8");
  const eol = raw.includes("\r\n") ? "\r\n" : "\n";
  const lines = raw.split(/\r?\n/).filter((l) => l.trim());
  const header = (lines.shift() ?? "").trim();
  const rows = lines.map((l) => {
    const f = l.split(",");
    return { order_id: f[0], from_repo: f[1], product: f[2], brief: f[3], status: f[4], delivered_path: f[5] ?? "", adopted: f[6], date: f[7], adopted_commit: f[8] ?? "", fields: f.length };
  });
  return { header, rows, eol };
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const root = process.cwd();
  const ffmpegBin = process.env.WEEKLY_CLIP_FFMPEG || "ffmpeg";
  const empireJs = process.env.WEEKLY_CLIP_EMPIRE || DEFAULT_EMPIRE;
  const studioDir = process.env.WEEKLY_CLIP_STUDIO_DIR || DEFAULT_STUDIO;
  const ordersFile = process.env.WEEKLY_CLIP_ORDERS_FILE || defaultOrdersFile(empireJs, root);
  const brief = `weekly 20 s arena clip ${opts.week}: 16:9 mp4 + gif + clip.json`;

  // (1) newest arena.exe built at HEAD (skipped for fixture dirs).
  const clipDir = opts.clip ? resolve(opts.clip) : join(root, CLIPS, opts.week);
  let arenaPath = null;
  let headSha = gitLine(root, ["rev-parse", "HEAD"]) ?? "unknown";
  if (opts.clip) {
    say(`ARENA fixture: arena build+run skipped (--clip ${clipDir})`);
    const pngs = listPngs(clipDir);
    if (pngs.length === 0) fail(2, `fixture clip dir has no PNGs: ${clipDir}`);
  } else {
    const headTimeRaw = gitLine(root, ["log", "-1", "--format=%ct", "HEAD"]);
    const headTime = headTimeRaw !== null && /^\d+$/.test(headTimeRaw) ? Number(headTimeRaw) : null;
    const found = findArena(root, headTime);
    if (found.stale) fail(3, `${found.reason}; build it first, this tool never runs cargo: ${BUILD_CMD}`);
    arenaPath = found.path;
    say(`ARENA ${arenaPath} (HEAD ${headSha})`);

    // (2) capture with the fixed tape: 600 frames, every frame, 30 fps.
    mkdirSync(clipDir, { recursive: true });
    const cap = run(arenaPath, [
      "--frames", String(FRAMES), "--capture-every", String(EVERY), "--fps", String(FPS),
      "--camera-path", CAMERA_PATH, "--exit", "--mute", "--deadline-ms", String(DEADLINE_MS),
      "--capture-dir", clipDir, "--sha", headSha, "--rows", ROWS.join(","),
    ], root);
    if (!cap.ok) fail(4, `arena capture failed (${cap.error || `exit ${cap.status}`}): ${cap.out.slice(-2000)}`);
    const got = listPngs(clipDir).length;
    say(`CAPTURE ok: ${got} frames in ${clipDir} (tape fixed: ${CAMERA_PATH}, rows ${ROWS.join(",")})`);
  }
  if (!opts.clip) {
    // Fixture path prints its own CAPTURE line; keep one line per step.
  } else {
    say(`CAPTURE fixture: ${listPngs(clipDir).length} frames in ${clipDir}`);
  }

  const capManifest = readJson(join(clipDir, "manifest.json")) ?? {};
  const clipName = typeof capManifest.clip === "string" && capManifest.clip.trim()
    ? capManifest.clip.trim()
    : opts.week;
  const mp4Path = join(clipDir, `${clipName}.mp4`);
  const gifPath = join(clipDir, `${clipName}.gif`);

  // (3) stitch MP4 + GIF; without ffmpeg say so and continue with clip.json.
  let ffmpegVersion = null;
  let stitched = false;
  const probe = run(ffmpegBin, ["-version"], root);
  if (!probe.ok) {
    say(`STITCH skipped: ffmpeg not found (${ffmpegBin}); install it, then re-run (Windows: winget install Gyan.FFmpeg)`);
  } else {
    ffmpegVersion = probe.out.split("\n")[0].trim();
    const st = run(process.execPath, [join(root, "tools", "showcase.mjs"), "--clip", clipDir], root);
    if (!st.ok) fail(5, `showcase stitch failed (${st.error || `exit ${st.status}`}): ${st.out.slice(-2000)}`);
    stitched = true;
    say(`STITCH ok: ${mp4Path} + ${gifPath}`);
  }

  // (4) unchanged picture is a NOOP: never send a clip twice, never an old
  // clip under a new week.
  const pngs = listPngs(clipDir);
  const lastFrame = join(clipDir, pngs[pngs.length - 1]);
  const prev = prevWeekDir(root, opts.week);
  let lookStage = prev ? "moved" : "first";
  if (prev) {
    const prevPngs = listPngs(prev.dir);
    const cmp = compareFrames(root, lastFrame, join(prev.dir, prevPngs[prevPngs.length - 1]));
    lookStage = cmp.lookStage;
    say(`MOVED: ${cmp.moved ? "moved" : "none"} vs ${prev.week} (${cmp.method})`);
    if (!cmp.moved) {
      say(`NOOP picture unchanged since ${prev.week}`);
      process.exit(0);
    }
  } else {
    say(`MOVED: first clip (no earlier week under ${CLIPS})`);
  }

  // (5) clip.json: the delivery manifest marketing-studio reads.
  const manifest = capManifest;
  const frames = pngs.length;
  const fps = Number(manifest.fps ?? FPS);
  const clip = {
    week: opts.week,
    clip: clipName,
    build_sha: manifest.sha ?? manifest.buildSha ?? headSha,
    seed: manifest.seed ?? SEED,
    tape: manifest.tape ?? "live",
    rows: manifest.rows ?? ROWS,
    look_stage: lookStage,
    frames,
    fps,
    duration_s: Number((frames / fps).toFixed(1)),
    ffmpeg: ffmpegVersion,
    stitch: stitched ? "ok" : "skipped",
    mp4: stitched ? `${clipName}.mp4` : null,
    gif: stitched ? `${clipName}.gif` : null,
    mp4_sha256: stitched && existsSync(mp4Path) ? sha256File(mp4Path) : null,
  };
  const clipPath = join(clipDir, "clip.json");
  const prevClip = readJson(clipPath);
  if (prevClip?.delivered === true && prevClip.mp4_sha256 && prevClip.mp4_sha256 === clip.mp4_sha256 && clip.mp4_sha256) {
    say(`CLIP-JSON ${clipPath}: unchanged`);
    say(`NOOP already delivered ${opts.week} at from-engine2040/${opts.week}`);
    process.exit(0);
  }
  writeFileSync(clipPath, `${JSON.stringify(clip, null, 2)}\n`, "utf8");
  say(`CLIP-JSON ${clipPath}: ${frames} frames, ${clip.duration_s} s, look_stage ${lookStage}`);

  if (!opts.deliver) {
    say(`READY ${clipDir} (not delivered: pass --deliver)`);
    return;
  }

  // (6) deliver: three files into a new studio folder, one order-book row.
  if (!stitched || !existsSync(mp4Path) || !existsSync(gifPath)) {
    say(`NOT DELIVERED ${opts.week}: no ${clipName}.mp4/.gif (stitch skipped, ffmpeg absent)`);
    process.exit(6);
  }
  const destDir = join(studioDir, "from-engine2040", opts.week);
  try {
    mkdirSync(destDir, { recursive: true });
    copyFileSync(mp4Path, join(destDir, `${opts.week}.mp4`));
    copyFileSync(gifPath, join(destDir, `${opts.week}.gif`));
    copyFileSync(clipPath, join(destDir, "clip.json"));
  } catch (e) {
    say(`NOT DELIVERED ${opts.week}: copy to ${destDir} failed: ${e.message}`);
    process.exit(6);
  }
  const relPath = `from-engine2040/${opts.week}`;

  let book = readOrders(ordersFile);
  let row = book.rows.find((r) => normBrief(briefHead(r.brief)) === normBrief(brief));
  let orderId = row?.order_id ?? null;
  if (!row) {
    const od = run(process.execPath, [empireJs, "order", "engine2040", brief, "--for", "marketing-studio", "--by", "engine2040-pilot"], root, { WEEKLY_CLIP_ORDERS_FILE: ordersFile });
    if (!od.ok) {
      say(`NOT DELIVERED ${opts.week}: files copied to ${relPath} but the order failed: ${(od.error || od.out).slice(-500)}`);
      process.exit(6);
    }
    orderId = /O-\d+/.exec(od.out)?.[0] ?? null;
    book = readOrders(ordersFile);
    row = (orderId && book.rows.find((r) => r.order_id === orderId))
      ?? book.rows.find((r) => normBrief(briefHead(r.brief)) === normBrief(brief));
    if (!row) {
      say(`NOT DELIVERED ${opts.week}: order ran but no row for ${orderId ?? brief} in ${ordersFile}`);
      process.exit(6);
    }
    orderId = row.order_id;
  }
  row.status = "delivered";
  row.delivered_path = relPath;
  const lines = [book.header && book.header !== "null" ? book.header : ORDERS_HEADER];
  let touched = false;
  for (const r of book.rows) {
    if (r.order_id === orderId && !touched) {
      touched = true;
      lines.push([r.order_id, r.from_repo, r.product, r.brief, "delivered", relPath, r.adopted, r.date, r.adopted_commit].join(","));
    } else {
      lines.push([r.order_id, r.from_repo, r.product, r.brief, r.status, r.delivered_path, r.adopted, r.date, r.adopted_commit].join(","));
    }
  }
  try {
    writeFileSync(ordersFile, `${lines.join(book.eol)}${book.eol}`, "utf8");
  } catch (e) {
    say(`NOT DELIVERED ${opts.week}: cannot update ${ordersFile}: ${e.message}`);
    process.exit(6);
  }
  // The delivery dir holds exactly the three files; nothing else is touched.
  say(`DELIVERED ${relPath}`);
}

main();
