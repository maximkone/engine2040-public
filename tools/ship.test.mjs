import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync as mkdtempRaw, readFileSync, readdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { CHUNK_SLOT_MS, archiveExistingDir, chunkLegRow, chunkPlan, chunkSlotReport, cleanFingerprint, cleanLedgerRow, cleanNormalize, createWallEstimator, describePresentMode, joinedChunkProof, nextestArchiveArgs, nextestRunArgs, parseSmokeCounts, projectProbeWall, resolvePresentFlags, stagedFileReceipt, wallBudgetMs, writeArchiveAndJunit, writeArchiveReceipt, writeJunitReceipt, writeStagedReceipt } from "./ship.mjs";
import { fnv1a64Hex } from "./fnv1a64.mjs";

// Remove every temp folder this file makes when it exits (test temp folders piled up in %TEMP%).
const madeDirs = [];
const mkdtempSync = (prefix, options) => { const dir = mkdtempRaw(prefix, options); madeDirs.push(dir); return dir; };
process.on("exit", () => { for (const dir of madeDirs) try { rmSync(dir, { recursive: true, force: true }); } catch { /* still in use: leave it */ } });

const SHIP = fileURLToPath(new URL("./ship.mjs", import.meta.url));
// A dry run without --work keeps its e2040-clean-run-* folder (the log lives there), so the child's temp is one folder
// this file removes.
const shipTemp = mkdtempSync(join(tmpdir(), "e2040-ship-tmp-"));

const runShip = (args, { work, env } = {}) => {
  try {
    execFileSync(process.execPath, [SHIP, ...args], {
      encoding: "utf8",
      timeout: 120_000,
      cwd: work ?? process.cwd(),
      env: { ...process.env, TEMP: shipTemp, TMP: shipTemp, ...env },
    });
    return { status: 0, out: "" };
  } catch (error) {
    return { status: error.status ?? 1, out: `${error.stdout ?? ""}${error.stderr ?? ""}` };
  }
};

// LF-normalized copy of fixtures/clean-run: fresh git worktrees check text
// fixtures out as CRLF and the manifest parser reads LF.
const stageFixture = (work) => {
  const repoRoot = join(fileURLToPath(new URL(".", import.meta.url)), "..");
  const stage = (rel, text) => {
    const dest = join(work, "fixture", ...rel.split("/"));
    mkdirSync(dirname(dest), { recursive: true });
    if (text) writeFileSync(dest, readFileSync(join(repoRoot, "fixtures", "clean-run", ...rel.split("/")), "utf8").replace(/\r\n/g, "\n"), "utf8");
    else copyFileSync(join(repoRoot, "fixtures", "clean-run", ...rel.split("/")), dest);
  };
  for (const rel of ["package-manifest.txt", "launch.txt", "content/contents.txt", "saves/progress-schema-v1.txt", "bin/arena-smoke.mjs", "bin/arena.exe"]) stage(rel, true);
  for (const rel of ["content/arena-bundle.bin"]) stage(rel, false);
  return join(work, "fixture");
};

test("cleanFingerprint is stable per failure class, blind to volatile numbers/paths", () => {
  const a = cleanFingerprint("PLAY_TIMEOUT", "no exit within 20000ms at C:\\e2040-clean\\bin\\arena.exe");
  const b = cleanFingerprint("PLAY_TIMEOUT", "no exit within 45000ms at D:\\other\\bin\\arena.exe");
  assert.equal(a, b, "same class, different ms/paths: one fingerprint");
  assert.match(a, /^[0-9a-f]{16}$/);
  assert.notEqual(
    cleanFingerprint("PLAY_FAILED", "no exit within 20000ms at C:\\e2040-clean\\bin\\arena.exe"),
    a,
    "different reason: different fingerprint",
  );
  assert.equal(cleanFingerprint("PLAY_TIMEOUT", "x"), cleanFingerprint("PLAY_TIMEOUT", "x"));
  assert.ok(cleanNormalize("took 1,234 ms over 5 frames").includes("<n>"));
});

test("cleanLedgerRow is one JSON line with fingerprint, count and status", () => {
  const row = cleanLedgerRow({ fingerprint: "abc123", reason: "PLAY_FAILED", detail: "boom", frames: 5, logPath: "l", failed: true });
  assert.equal(row.split("\n").length, 1, "single ledger row per run");
  const parsed = JSON.parse(row);
  assert.equal(parsed.fingerprint, "abc123");
  assert.equal(parsed.count, 1);
  assert.equal(parsed.status, "new");
});

test("a failing dry-run log carries its fingerprint plus exactly 1 ledger row", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const logPath = join(work, "clean-run.log");
  const r = runShip(["run", "--dry-run", "--fixture", join(work, "missing-fixture"), "--log", logPath, "--frames", "1"]);
  assert.equal(r.status, 1, "missing fixture fails");
  const log = readFileSync(logPath, "utf8");
  assert.match(log, /CLEAN-RUN FAIL \[MISSING_FIXTURE\]/);
  const prints = log.match(/^fingerprint: [0-9a-f]{16}$/m);
  assert.ok(prints, "every FAIL carries its fingerprint");
  const ledgers = log.split("\n").filter((l) => l.startsWith("ledger: "));
  assert.equal(ledgers.length, 1, "0 ledger rows before -> 1 ledger row per run");
  const parsed = JSON.parse(ledgers[0].slice("ledger: ".length));
  assert.equal(parsed.fingerprint, prints[0].split(": ")[1], "ledger row carries the same fingerprint");
  assert.equal(parsed.count, 1);
});

test("stagedFileReceipt lists every staged rel with bytes plus fnv (tauri copy_binaries receipt pattern)", () => {
  const staged = {
    "launch.txt": Buffer.from("version:1\n"),
    "bin/arena.exe": Buffer.from([0x4d, 0x5a, 0x90, 0x00]),
  };
  const receipt = stagedFileReceipt(staged);
  assert.equal(receipt.length, 2, "one row per staged file");
  assert.deepEqual(receipt.map((r) => r.rel), ["bin/arena.exe", "launch.txt"], "sorted by rel");
  for (const row of receipt) {
    assert.equal(row.bytes, staged[row.rel].length);
    assert.equal(row.fnv, fnv1a64Hex(staged[row.rel]));
  }
});

test("writeStagedReceipt writes exactly 1 stage-receipt.json per staging act", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const dist = join(work, "dist");
  const staged = { "launch.txt": Buffer.from("version:1\n") };
  const { path, files } = writeStagedReceipt(dist, staged);
  assert.equal(path, join(dist, "stage-receipt.json"));
  const onDisk = JSON.parse(readFileSync(path, "utf8"));
  assert.equal(onDisk.files.length, 1, "0 staged-file receipts before -> 1 per pack");
  assert.deepEqual(onDisk.files, files);
});

test("archiveExistingDir keeps the prior tree aside instead of wiping it", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const target = join(work, "target");
  mkdirSync(target, { recursive: true });
  writeFileSync(join(target, "keep.txt"), "last good tree", "utf8");
  const archived = archiveExistingDir(target);
  assert.ok(archived && archived.startsWith(`${target}.archive-`), "archive sits next to the target");
  assert.equal(existsSync(target), false, "target path is free for the new stage");
  assert.equal(readFileSync(join(archived, "keep.txt"), "utf8"), "last good tree", "prior tree survives");
  assert.equal(archiveExistingDir(join(work, "missing-dir")), null, "missing dir archives to nothing");
  const empty = join(work, "empty");
  mkdirSync(empty, { recursive: true });
  assert.equal(archiveExistingDir(empty), null, "empty dir archives to nothing");
});

test("ship.mjs performs 0 wipes on the --replace paths (archive, never delete)", () => {
  const src = readFileSync(SHIP, "utf8");
  const wipes = (src.match(/rmSync\s*\(/g) ?? []).length;
  assert.equal(wipes, 0, "1 wiped FAIL log (2026-09-29 full attempt) -> 0 wipes");
  assert.ok(src.includes("archiveExistingDir(dist)"), "pack dist restage archives");
  assert.ok(src.includes("archiveExistingDir(target)"), "run target restage archives");
});

test("a passing dry-run log closes with exactly 1 ledger row", () => {
  // Stage an LF-normalized copy of fixtures/clean-run: fresh git worktrees
  // check text fixtures out as CRLF and the manifest parser reads LF.
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const repoRoot = join(fileURLToPath(new URL(".", import.meta.url)), "..");
  const stage = (rel, text) => {
    const dest = join(work, "fixture", ...rel.split("/"));
    mkdirSync(dirname(dest), { recursive: true });
    if (text) writeFileSync(dest, readFileSync(join(repoRoot, "fixtures", "clean-run", ...rel.split("/")), "utf8").replace(/\r\n/g, "\n"), "utf8");
    else copyFileSync(join(repoRoot, "fixtures", "clean-run", ...rel.split("/")), dest);
  };
  for (const rel of ["package-manifest.txt", "launch.txt", "content/contents.txt", "saves/progress-schema-v1.txt", "bin/arena-smoke.mjs", "bin/arena.exe"]) stage(rel, true);
  for (const rel of ["content/arena-bundle.bin"]) stage(rel, false);
  const logPath = join(work, "clean-run.log");
  const r = runShip(["run", "--dry-run", "--fixture", join(work, "fixture"), "--log", logPath, "--frames", "1"]);
  assert.equal(r.status, 0, `dry-run passes: ${r.out.slice(0, 300)}`);
  const log = readFileSync(logPath, "utf8");
  assert.match(log, /CLEAN-RUN PASS/);
  const ledgers = log.split("\n").filter((l) => l.startsWith("ledger: "));
  assert.equal(ledgers.length, 1, "pass runs also close with 1 ledger row");
  const parsed = JSON.parse(ledgers[0].slice("ledger: ".length));
  assert.equal(parsed.count, 1, "pass ledger carries count 1 via writeLogAndExit");
  assert.equal(parsed.status, "pass");
  assert.equal(parsed.reason, "PASS");
  assert.equal(parsed.frames, 1);
});

test("wall estimator converges on a steady rate and paces the remainder", () => {
  const est = createWallEstimator();
  assert.equal(est.record(0, 0), 0, "one sample is a baseline, not a rate");
  let rate = 0;
  for (let i = 1; i <= 10; i++) rate = est.record(i * 60, i * 1000);
  assert.ok(rate > 59 && rate < 61, `steady 60 steps/sec converges (got ${rate})`);
  const eta = est.etaMs(6000);
  assert.ok(eta > 95000 && eta < 105000, `6000 steps at 60/sec pace ~100 s (got ${eta}ms)`);
});

test("wall estimator drags down gradually when the run slows", () => {
  const est = createWallEstimator();
  for (let i = 1; i <= 10; i++) est.record(i * 60, i * 1000);
  const fast = est.rate;
  const afterOneSlow = est.record(10 * 60 + 6, 11 * 1000);
  assert.ok(afterOneSlow < fast, "one slow sample bends the rate down");
  assert.ok(afterOneSlow > 6, `double smoothing trails instead of jumping (got ${afterOneSlow})`);
  assert.equal(est.etaMs(0), Number.POSITIVE_INFINITY, "nothing left means no wait");
});

test("projectProbeWall flags a slow probe past budget and clears a fast one", () => {
  // Slow: 600 frames in 700 s projects 18000 to ~5.8 h, past the 5.05 h budget.
  const slow = projectProbeWall({ probeFrames: 600, probeMs: 700_000, totalFrames: 18000, budgetMs: 18_180_000 });
  assert.equal(slow.overrun, true, `slow probe aborts (projected ${Math.round(slow.projectedMs)}ms)`);
  assert.ok(slow.projectedMs > 18_180_000);
  // Fast: the quiet-box reading, 600 frames in 83 s, projects to ~41 min.
  const fast = projectProbeWall({ probeFrames: 600, probeMs: 83_000, totalFrames: 18000, budgetMs: 18_180_000 });
  assert.equal(fast.overrun, false, `fast probe proceeds (projected ${Math.round(fast.projectedMs)}ms)`);
  assert.equal(wallBudgetMs(18_180_000), 18_180_000, "no env override: the play timeout is the budget");
});

test("a large dry-run logs its wall projection and still passes", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const fixture = stageFixture(work);
  const logPath = join(work, "clean-run.log");
  const r = runShip(["run", "--dry-run", "--fixture", fixture, "--log", logPath, "--frames", "1000"]);
  assert.equal(r.status, 0, `probed dry-run passes: ${r.out.slice(0, 300)}`);
  const log = readFileSync(logPath, "utf8");
  assert.match(log, /\$ wall-probe: 600 frames in \d+ms projects 1000 frames to \d+ms/, "0 projection lines before -> 1 probe line per large run");
  assert.match(log, /CLEAN-RUN PASS/);
  const ledgers = log.split("\n").filter((l) => l.startsWith("ledger: "));
  assert.equal(ledgers.length, 1, "probed pass still closes with 1 ledger row");
});

test("a dry-run past a tiny wall budget aborts as WALL_OVERRUN with receipt", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const fixture = stageFixture(work);
  const logPath = join(work, "clean-run.log");
  const probeWork = join(work, "w");
  const r = runShip(
    ["run", "--dry-run", "--fixture", fixture, "--work", probeWork, "--log", logPath, "--frames", "1000"],
    { env: { E2040_SHIP_WALL_BUDGET_MS: "1" } },
  );
  assert.equal(r.status, 1, "over-budget probe aborts");
  const log = readFileSync(logPath, "utf8");
  assert.match(log, /CLEAN-RUN FAIL \[WALL_OVERRUN\]/, "0 projection aborts before -> 1 per over-budget run");
  const prints = log.match(/^fingerprint: [0-9a-f]{16}$/m);
  assert.ok(prints, "the overrun carries its fingerprint");
  const ledgers = log.split("\n").filter((l) => l.startsWith("ledger: "));
  assert.equal(ledgers.length, 1, "overrun still closes with exactly 1 ledger row");
  assert.equal(JSON.parse(ledgers[0].slice("ledger: ".length)).fingerprint, prints[0].split(": ")[1]);
  const receipt = JSON.parse(readFileSync(join(probeWork, "clean-run-receipt.json"), "utf8"));
  assert.equal(receipt.overrun, true, "abort leaves a receipt behind");
  assert.ok(receipt.projectedMs > receipt.budgetMs);
});

test("present flags default to today's behavior and forward verbatim when set", () => {
  assert.deepEqual(resolvePresentFlags({}), [], "unset: no extra arg to the game binary");
  assert.deepEqual(resolvePresentFlags({ presentEvery: "1" }), [], "every-1 is today's behavior");
  assert.deepEqual(resolvePresentFlags({ presentEvery: "4" }), ["--present-every", "4"]);
  assert.deepEqual(resolvePresentFlags({ presentSkip: true }), ["--present-skip"]);
  assert.throws(() => resolvePresentFlags({ presentEvery: "0" }), /positive integer/);
  assert.throws(() => resolvePresentFlags({ presentEvery: "nope" }), /positive integer/);
  assert.throws(() => resolvePresentFlags({ presentEvery: "2", presentSkip: true }), /conflicts/);
  assert.deepEqual(describePresentMode({}), { mode: "every-1", every: 1, skip: false });
  assert.deepEqual(describePresentMode({ presentEvery: "4" }), { mode: "every-4", every: 4, skip: false });
  assert.deepEqual(describePresentMode({ presentSkip: true }), { mode: "skip", every: null, skip: true });
});

test("parseSmokeCounts quotes ticks played plus frames presented", () => {
  const got = parseSmokeCounts("arena smoke: ticks=30 frames=30 alpha=0.990 hud_cells=12");
  assert.equal(got.ticks, 30, "ticks played quoted");
  assert.equal(got.framesPresented, 30, "frames presented quoted");
  const skipped = parseSmokeCounts("arena smoke: ticks=600 frames=150 alpha=0.500");
  assert.equal(skipped.ticks, 600);
  assert.equal(skipped.framesPresented, 150, "skipped presentation shows fewer frames than ticks");
  assert.deepEqual(parseSmokeCounts("(no output)"), { ticks: null, framesPresented: null });
});

test("a dry-run with --present-every writes a receipt with ticks, frames, wall and mode", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const fixture = stageFixture(work);
  const logPath = join(work, "clean-run.log");
  const receiptPath = join(work, "clean-run-receipt.json");
  const r = runShip(["run", "--dry-run", "--fixture", fixture, "--log", logPath, "--frames", "5", "--present-every", "4", "--receipt", receiptPath]);
  assert.equal(r.status, 0, `flagged dry-run passes (launcher ignores the forwarded flag): ${r.out.slice(0, 300)}`);
  const receipt = JSON.parse(readFileSync(receiptPath, "utf8"));
  assert.equal(receipt.ticks, 5, "receipt quotes ticks played");
  assert.equal(receipt.frames_presented, 5, "receipt quotes frames presented");
  assert.ok(typeof receipt.wall_ms === "number" && receipt.wall_ms >= 0, "receipt quotes play wall time");
  assert.deepEqual(receipt.present, { mode: "every-4", every: 4, skip: false });
  assert.equal(receipt.exit, 0);
  const log = readFileSync(logPath, "utf8");
  assert.match(log, /\$ counts: ticks=5 frames_presented=5 wall_ms=\d+ present=every-4/);
});

test("a dry-run with --present-skip records the skip mode in its receipt", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const fixture = stageFixture(work);
  const logPath = join(work, "clean-run.log");
  const receiptPath = join(work, "clean-run-receipt.json");
  const r = runShip(["run", "--dry-run", "--fixture", fixture, "--log", logPath, "--frames", "5", "--present-skip", "--receipt", receiptPath]);
  assert.equal(r.status, 0, `skip dry-run passes: ${r.out.slice(0, 300)}`);
  const receipt = JSON.parse(readFileSync(receiptPath, "utf8"));
  assert.deepEqual(receipt.present, { mode: "skip", every: null, skip: true });
  assert.equal(receipt.ticks, 5);
  assert.equal(receipt.frames_presented, 5);
});

test("a dry-run with conflicting present flags fails loudly", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const r = runShip(["run", "--dry-run", "--fixture", join(work, "missing"), "--present-every", "2", "--present-skip"]);
  assert.equal(r.status, 2, "flag conflict exits 2 with usage");
  assert.match(r.out, /conflicts with --present-every/);
});

test("nextest arg builders carry the archive/extract leg (A8 steal)", () => {
  assert.deepEqual(
    nextestArchiveArgs({ archiveFile: "nextest-archive.tar.zst" }),
    ["archive", "--archive-file", "nextest-archive.tar.zst"],
    "archive leg names its file",
  );
  assert.deepEqual(
    nextestRunArgs({ archiveFile: "a.tar.zst", extractTo: "T", workspaceRemap: "D", junitPath: "j.xml" }),
    ["run", "--archive-file", "a.tar.zst", "--extract-to", "T", "--workspace-remap", "D", "--junit", "j.xml"],
    "extract leg remaps the workspace and names the junit receipt",
  );
  assert.deepEqual(
    nextestRunArgs({ archiveFile: "a.tar.zst", extractTo: "T" }),
    ["run", "--archive-file", "a.tar.zst", "--extract-to", "T"],
    "optional remap/junit stay off when unset (cargo fallback)",
  );
});

test("writeJunitReceipt writes one junit receipt, pass or fail", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const passPath = join(work, "clean-run-junit.xml");
  writeJunitReceipt(passPath, { wallMs: 1500, failed: false });
  const passXml = readFileSync(passPath, "utf8");
  assert.match(passXml, /failures="0"/, "pass receipt records 0 failures");
  assert.match(passXml, /time="1\.5"/, "pass receipt quotes the play wall");
  const failPath = join(work, "fail-junit.xml");
  writeJunitReceipt(failPath, { wallMs: 2000, failed: true, detail: "boom <tag> & quote" });
  const failXml = readFileSync(failPath, "utf8");
  assert.match(failXml, /failures="1"/, "fail receipt records 1 failure");
  assert.ok(!failXml.includes("<tag>"), "failure detail is XML-escaped");
});

test("writeArchiveReceipt writes one archive receipt per run", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const files = [{ rel: "bin/arena.exe", bytes: 4, fnv: "abc" }];
  const { path } = writeArchiveReceipt(work, { files, wallMs: 2500, rebuildMs: 2595, exit: 0, archiveFile: "a.tar.zst", extractTo: "T", workspaceRemap: "D" });
  assert.equal(path, join(work, "clean-run-archive.json"), "archive receipt sits beside the run receipt");
  const onDisk = JSON.parse(readFileSync(path, "utf8"));
  assert.equal(onDisk.tool, "nextest archive/extract");
  assert.deepEqual(onDisk.files, files);
  assert.equal(onDisk.wall_ms, 2500);
  assert.equal(onDisk.rebuild_ms, 2595, "receipt quotes the full-rebuild wall for the archive-vs-rebuild bar");
  assert.equal(onDisk.exit, 0);
});

test("writeArchiveAndJunit leaves 1 archive plus 1 junit receipt beside the run receipt", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const receiptPath = join(work, "clean-run-receipt.json");
  writeFileSync(receiptPath, "{}\n", "utf8");
  const { archivePath, junitPath } = writeArchiveAndJunit(receiptPath, {
    manifestRows: [{ name: "launch.txt", bytes: 10, digest: "ff" }],
    wallMs: 3000,
    failed: false,
    dist: "D",
    target: "T",
  });
  assert.equal(archivePath, join(work, "clean-run-archive.json"));
  assert.equal(junitPath, join(work, "clean-run-junit.xml"));
  const archive = JSON.parse(readFileSync(archivePath, "utf8"));
  assert.equal(archive.files.length, 1, "0 archive rows before -> 1 manifest row per run");
  assert.equal(archive.exit, 0);
  assert.match(readFileSync(junitPath, "utf8"), /failures="0"/);
  const failReceipt = join(work, "fail-receipt.json");
  writeFileSync(failReceipt, "{}\n", "utf8");
  writeArchiveAndJunit(failReceipt, { manifestRows: [], wallMs: 1, failed: true, detail: "WALL_OVERRUN" });
  assert.equal(JSON.parse(readFileSync(join(work, "clean-run-archive.json"), "utf8")).exit, 1, "failed run records exit 1");
  assert.match(readFileSync(join(work, "clean-run-junit.xml"), "utf8"), /failures="1"/);
});

test("a passing dry-run leaves its archive plus junit receipt beside the receipt (A8 accept)", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
  const fixture = stageFixture(work);
  const receiptPath = join(work, "clean-run-receipt.json");
  const r = runShip(["run", "--dry-run", "--fixture", fixture, "--log", join(work, "clean-run.log"), "--frames", "2", "--receipt", receiptPath]);
  assert.equal(r.status, 0, `dry-run passes: ${r.out.slice(0, 300)}`);
  assert.ok(existsSync(join(work, "clean-run-archive.json")), "0 archive receipts before -> 1 per GM3 run");
  assert.ok(existsSync(join(work, "clean-run-junit.xml")), "0 junit receipts before -> 1 per GM3 run");
  const log = readFileSync(join(work, "clean-run.log"), "utf8");
  assert.match(log, /\$ archive: .*clean-run-archive\.json/);
  assert.match(log, /\$ junit: .*clean-run-junit\.xml/);
});

test("the GM3 chunk plan splits 18000 frames into legs that sum to the full proof", () => {
  const plan = chunkPlan(18000, 3);
  assert.equal(plan.totalFrames, 18000);
  assert.deepEqual(plan.legs.map((l) => l.frames), [6000, 6000, 6000], "the plan's 3 legs are the 6000-frame slots");
  assert.equal(plan.legs.reduce((n, l) => n + l.frames, 0), 18000, "chunks sum to the full proof");
  assert.deepEqual(plan.legs.map((l) => [l.firstFrame, l.lastFrame]), [[1, 6000], [6001, 12000], [12001, 18000]], "legs cover contiguous frames");
  assert.equal(plan.legs.reduce((n, l) => n + (l.lastFrame - l.firstFrame + 1), 0), 18000);
  const uneven = chunkPlan(18000, 7);
  const sizes = uneven.legs.map((l) => l.frames);
  assert.equal(sizes.reduce((n, f) => n + f, 0), 18000, "an uneven split still sums to the full proof");
  assert.ok(Math.max(...sizes) - Math.min(...sizes) <= 1, "leg sizes stay 1 apart at most");
  assert.deepEqual(chunkPlan(18000, 1).legs.map((l) => l.frames), [18000], "one leg is today's single run");
  assert.throws(() => chunkPlan(18000, 0), /--chunks/);
  assert.throws(() => chunkPlan(5, 6), /--chunks 6 exceeds --frames 5/);
});

test("each chunk hash is stable across runs and blind to wall time", () => {
  const counts = { ticks: 6000, framesPresented: 6000 };
  const rowsOf = (plan) => plan.legs.map((leg) => chunkLegRow(plan, leg, counts));
  const first = rowsOf(chunkPlan(18000, 3));
  const again = rowsOf(chunkPlan(18000, 3));
  assert.deepEqual(first.map((r) => r.hash), again.map((r) => r.hash), "same plan twice: same leg hashes");
  assert.equal(new Set(first.map((r) => r.hash)).size, 3, "one hash per leg");
  assert.match(first[0].hash, /^[0-9a-f]{16}$/);
  assert.deepEqual(first[0], { index: 1, frames: 6000, first: 1, last: 6000, ticks: 6000, frames_presented: 6000, hash: first[0].hash });
  const shifted = chunkLegRow(chunkPlan(18000, 3), chunkPlan(18000, 3).legs[0], { ticks: 5999, framesPresented: 5999 });
  assert.notEqual(shifted.hash, first[0].hash, "a different tick count changes the hash");
  const otherPlan = chunkLegRow(chunkPlan(12000, 2), chunkPlan(12000, 2).legs[0], counts);
  assert.notEqual(otherPlan.hash, first[0].hash, "a different plan changes the hash");
});

test("the joined proof sums the legs and reports a missing leg as a mismatch", () => {
  const plan = chunkPlan(18000, 3);
  const rows = plan.legs.map((leg) => chunkLegRow(plan, leg, { ticks: leg.frames, framesPresented: leg.frames }));
  const joined = joinedChunkProof(plan, rows);
  assert.equal(joined.frames, 18000, "chunks sum to the full proof");
  assert.equal(joined.expected, 18000);
  assert.equal(joined.matches, true);
  assert.match(joined.digest, /^[0-9a-f]{16}$/);
  assert.equal(joinedChunkProof(plan, rows).digest, joined.digest, "the joined hash is stable");
  const short = joinedChunkProof(plan, rows.slice(0, 2));
  assert.equal(short.frames, 12000);
  assert.equal(short.matches, false, "2 of 3 legs never counts as the full proof");
});

test("chunk slots project each leg against the 30 min slot from one probe", () => {
  assert.equal(CHUNK_SLOT_MS, 1_800_000, "the slot is 30 min");
  const plan = chunkPlan(18000, 3);
  const fits = chunkSlotReport(plan, 138); // the RTX 3050 twin's 83 s per 600 frames
  assert.deepEqual(fits.projected, [828_000, 828_000, 828_000]);
  assert.deepEqual(fits.over, [], "a 6000-frame leg at 138 ms/frame fits the slot");
  const slow = chunkSlotReport(plan, 520); // the contended reading, 62 s per 120 frames
  assert.deepEqual(slow.over, [1, 2, 3], "a slow box puts every leg over the slot");
  assert.deepEqual(chunkSlotReport(plan, 0), { projected: [], over: [] }, "no rate: no claim");
});

test("an 18000-frame dry-run in 3 legs prints a joined proof whose hashes repeat", () => {
  const fixture = stageFixture(mkdtempSync(join(tmpdir(), "e2040-ship-test-")));
  const oneRun = () => {
    const work = mkdtempSync(join(tmpdir(), "e2040-ship-test-"));
    const logPath = join(work, "clean-run.log");
    const receiptPath = join(work, "clean-run-receipt.json");
    const r = runShip(["run", "--dry-run", "--fixture", fixture, "--work", work, "--log", logPath, "--receipt", receiptPath, "--frames", "18000", "--chunks", "3"]);
    return { r, log: readFileSync(logPath, "utf8"), receipt: JSON.parse(readFileSync(receiptPath, "utf8")) };
  };
  const first = oneRun();
  const second = oneRun();
  assert.equal(first.r.status, 0, `chunked dry-run passes: ${first.r.out.slice(0, 300)}`);
  assert.match(first.log, /\$ chunk plan: 3 legs \(6000\+6000\+6000\) = 18000 frames/);
  assert.match(first.log, /\$ chunk note: each leg boots a fresh bout/);
  for (const i of [1, 2, 3]) {
    assert.match(first.log, new RegExp(`\\$ chunk ${i}/3: frames ${(i - 1) * 6000 + 1}-${i * 6000} \\(6000\\) ticks=6000 presented=6000 wall_ms=\\d+ hash=[0-9a-f]{16}`));
  }
  assert.match(first.log, /\$ chunks: 3 legs, 18000 of 18000 frames played, sum matches, joined hash=[0-9a-f]{16}/);
  assert.match(first.log, /\$ chunk slots: legs project \d+\+\d+\+\d+ ms against the 1800000 ms slot \(fits\)/);
  assert.equal(first.receipt.chunks.length, 3, "one row per leg");
  assert.deepEqual(first.receipt.chunks.map((c) => c.frames), [6000, 6000, 6000]);
  assert.equal(first.receipt.frames, 18000);
  assert.equal(first.receipt.ticks, 18000, "leg ticks sum to the full proof");
  assert.equal(first.receipt.frames_presented, 18000);
  assert.equal(first.receipt.chunk_sum, true);
  assert.deepEqual(first.receipt.chunk_plan, { legs: 3, total_frames: 18000 });
  assert.deepEqual(first.receipt.chunks.map((c) => c.hash), second.receipt.chunks.map((c) => c.hash), "each leg hash is stable across runs");
  assert.equal(second.receipt.joined_hash, first.receipt.joined_hash, "the joined hash is stable across runs");
  assert.equal(second.log.match(/CLEAN-RUN PASS/g).length, 1, "a chunked pass closes with 1 PASS line");
});

test("--chunks past the frame count is a usage error", () => {
  const r = runShip(["run", "--dry-run", "--frames", "2", "--chunks", "3"]);
  assert.equal(r.status, 2, "usage errors exit 2");
  assert.match(r.out, /--chunks 3 exceeds --frames 2/);
});
