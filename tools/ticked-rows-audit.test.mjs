// Run: node --test tools/ticked-rows-audit.test.mjs
// Pins the static ticked-row audit: command parsing, filter matching, every reason code, and the
// CLI (exit codes, --json, removed-by evidence) against a throwaway git repo.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { auditRows, cargoTestCommands, closureEvidence, closureNotes, filterMatches, sourcePaths, tickedRows } from "./ticked-rows-audit.mjs";

const TOOL = join(dirname(fileURLToPath(import.meta.url)), "ticked-rows-audit.mjs");

// ---- command parsing --------------------------------------------------------------------

test("cargoTestCommands: crate, lib flag and filter", () => {
  const [c] = cargoTestCommands("Accept: `cargo test -p engine-net --lib snapshot`.");
  assert.deepEqual(c.crates, ["engine-net"]);
  assert.equal(c.lib, true);
  assert.deepEqual(c.filters, ["snapshot"]);
  assert.deepEqual(c.targets, []);
});

test("cargoTestCommands: wrapped rows, env prefix and gh-check wrapper", () => {
  const text = "x `cargo test -p   engine-playtest\n --lib   mirror_gdb` y `E2040_REQUIRE_GPU=1 cargo test -p engine-render --lib require_or_skip` z `node tools/gh-check.mjs cargo test -p engine-ai --lib melee`";
  const cmds = cargoTestCommands(text.replace(/\n/g, " "));
  assert.deepEqual(cmds.map((c) => [c.crates[0], c.filters[0]]), [["engine-playtest", "mirror_gdb"], ["engine-render", "require_or_skip"], ["engine-ai", "melee"]]);
  assert.equal(cmds[0].text, "cargo test -p engine-playtest --lib mirror_gdb");
});

test("cargoTestCommands: flag values are never filters", () => {
  const [c] = cargoTestCommands("`cargo test --locked --release -p engine-geo --features all,uv --no-default-features --test kit recipe`");
  assert.deepEqual(c.crates, ["engine-geo"]);
  assert.deepEqual(c.targets, ["kit"]);
  assert.deepEqual(c.filters, ["recipe"]);
});

test("cargoTestCommands: --test it with a module filter, -p twice, package= form", () => {
  const [a] = cargoTestCommands("`cargo test -p engine-endpoint --test it rules_door::`");
  assert.deepEqual([a.targets, a.filters], [["it"], ["rules_door::"]]);
  const [b] = cargoTestCommands("`cargo test -p engine-ecs -p engine-app --package=engine-geo`");
  assert.deepEqual(b.crates, ["engine-ecs", "engine-app", "engine-geo"]);
  assert.deepEqual(b.filters, []);
});

test("cargoTestCommands: harness args after --", () => {
  const [a] = cargoTestCommands("`cargo test -p engine-perf latency -- --nocapture --test-threads 1`");
  assert.deepEqual(a.filters, ["latency"]);
  const [b] = cargoTestCommands("`cargo test -p engine-perf -- --skip slow --exact exact_name`");
  assert.deepEqual(b.filters, ["exact_name"]);
  assert.equal(b.exact, true);
});

test("cargoTestCommands: templates, prose and non-cargo spans give nothing", () => {
  assert.deepEqual(cargoTestCommands("`cargo test -p <crate>` and `node tools/x.mjs` and `cargo build -p engine-net`"), []);
  const [c] = cargoTestCommands("`cargo test -p engine-net --lib snapshot (7 tests)`");
  assert.deepEqual(c.filters, ["snapshot"], '"(7" and "tests)" are prose, not filters');
});

test("cargoTestCommands: chained commands split and repeats collapse", () => {
  const cmds = cargoTestCommands("`cargo test -p engine-a --lib x && cargo test -p engine-b --lib y` again `cargo test -p engine-a --lib x`");
  assert.deepEqual(cmds.map((c) => c.crates[0]), ["engine-a", "engine-b"]);
});

test("sourcePaths: crate paths only, not donors or globs", () => {
  const text = "Owns `crates/net/src/lagcomp.rs` and crates/net/tests/it/wire.rs:12, not donor/crates/x/src/y.rs or `crates/render/src/present/**` or crates/*/src/z.rs";
  assert.deepEqual(sourcePaths(text), ["crates/net/src/lagcomp.rs", "crates/net/tests/it/wire.rs"]);
});

// ---- filter matching --------------------------------------------------------------------

function pool(fns, mods) {
  return { fns: new Set(fns), mods: new Set(mods), fnText: ` ${fns.join(" ")} `, modText: ` ${mods.join(" ")} ` };
}

test("filterMatches: substring like cargo, module path, trailing ::, exact", () => {
  const p = pool(["snapshot_roundtrip", "frame_ok"], ["replay", "wire", "tests"]);
  assert.equal(filterMatches(p, "snapshot"), true);
  assert.equal(filterMatches(p, "roundtrip"), true);
  assert.equal(filterMatches(p, "wire::frame"), true);
  assert.equal(filterMatches(p, "wire::snapshot_roundtrip"), true);
  assert.equal(filterMatches(p, "wire::"), true);
  assert.equal(filterMatches(p, "frame_ok::"), false, "a trailing :: names a module, not a fn");
  assert.equal(filterMatches(p, "cloth"), false);
  assert.equal(filterMatches(p, "cloth::frame"), false);
  assert.equal(filterMatches(p, "snapshot", true), false, "--exact wants the whole name");
  assert.equal(filterMatches(p, "snapshot_roundtrip", true), true);
});

// ---- audit on an in-memory tree ---------------------------------------------------------

function memoryCtx(sources, extra = {}) {
  const files = new Set(Object.keys(sources));
  return {
    files,
    packages: new Map([["engine-net", "crates/net"], ["engine-save", "crates/save"], ["demo", "."]]),
    read: (p) => sources[p] ?? null,
    ...extra,
  };
}

const SOURCES = {
  "crates/net/Cargo.toml": "[package]\nname = \"engine-net\"\n",
  "crates/net/src/lib.rs": "pub mod replay;\n// fn commented_out() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn snapshot_roundtrip() {}\n}\n",
  "crates/net/src/replay/mod.rs": "#[test]\nfn replay_exact() {}\n",
  "crates/net/tests/it/main.rs": "mod wire;\n",
  "crates/net/tests/it/wire.rs": "#[test]\nfn frame_ok() {}\n",
  "crates/net/tests/solo.rs": "#[test]\nfn solo_case() {}\n",
  "crates/save/src/lib.rs": "pub fn save() {}\n",
  "src/main.rs": "fn main() {}\n#[test]\nfn root_case() {}\n",
};

const row = (id, text) => ({ id, source: "board", text });

test("auditRows: rows whose crate, target, filter and paths exist are clean", () => {
  const rows = [
    row("OK1", "Accept: `cargo test -p engine-net --lib snapshot`. Owns `crates/net/src/lib.rs`."),
    row("OK2", "`cargo test -p engine-net --test it wire::frame`"),
    row("OK3", "`cargo test -p engine-net --test solo solo_case` and `cargo test -p engine-net --lib replay::`"),
    row("OK4", "`cargo test -p demo root_case` and path `crates/net/src/replay.rs` (replay/mod.rs counts)"),
    row("OK5", "`cargo test -p engine-net` runs everything, `cargo test frame_ok` has no -p and is skipped"),
  ];
  const r = auditRows(rows, memoryCtx(SOURCES));
  assert.deepEqual(r.suspects, []);
  assert.equal(r.ticked, 5);
  assert.equal(r.commands, 6);
});

test("auditRows: crate-missing names the package and the Cargo.toml removal", () => {
  const ctx = memoryCtx(SOURCES, { removedBy: (p) => (p === "crates/viewer/Cargo.toml" ? { sha: "1296f273", subject: "rot: drop viewer" } : false) });
  const r = auditRows([row("BV1", "`cargo test --locked -p engine-viewer --lib` and again `cargo test -p engine-viewer --lib x`")], ctx);
  assert.equal(r.suspects.length, 1);
  assert.deepEqual(r.suspects[0].findings.map((f) => f.reason), ["crate-missing"], "same package twice is one finding");
  assert.match(r.suspects[0].findings[0].evidence, /engine-viewer.*crates\/viewer\/Cargo\.toml removed by 1296f273 "rot: drop viewer"/);
});

test("auditRows: filter-missing, with the commit that last changed it", () => {
  const ctx = memoryCtx(SOURCES, { lastTouch: (needle) => (needle === "cloth_gap" ? { sha: "3d6ee4a1", subject: "auto: backup" } : false) });
  const r = auditRows([
    row("PX27", "`cargo test -p engine-net --lib cloth_gap`"),
    row("PX28", "`cargo test -p engine-net --lib replay::cloth_gap`"),
    row("EG31", "`cargo test -p engine-save --lib fsync_order`"),
    row("PX29", "`cargo test -p engine-net --lib commented_out`"),
  ], ctx);
  const byId = Object.fromEntries(r.suspects.map((s) => [s.id, s.findings]));
  assert.equal(byId.PX27[0].reason, "filter-missing");
  assert.match(byId.PX27[0].evidence, /"cloth_gap" in crates\/net\/src .*last changed in 3d6ee4a1 "auto: backup"/);
  assert.match(byId.PX28[0].evidence, /holds "cloth_gap"/, "the first failing segment is named");
  assert.match(byId.EG31[0].evidence, /no commit under crates\/save ever held it/);
  assert.equal(byId.PX29[0].reason, "filter-missing", "a commented-out fn is not a test");
});

test("auditRows: --lib looks in src only, --test in that target only", () => {
  const r = auditRows([
    row("A1", "`cargo test -p engine-net --lib frame_ok`"),
    row("A2", "`cargo test -p engine-net --test it snapshot_roundtrip`"),
    row("A3", "`cargo test -p engine-net frame_ok`"),
  ], memoryCtx(SOURCES));
  assert.deepEqual(r.suspects.map((s) => s.id), ["A1", "A2"], "A3 has no --lib/--test, so the whole crate is searched");
});

test("auditRows: test-target-missing vs test-target-moved", () => {
  const r = auditRows([
    row("T1", "`cargo test -p engine-net --test nothing_here`"),
    row("T2", "`cargo test -p engine-net --test wire`"),
  ], memoryCtx(SOURCES));
  assert.deepEqual(r.suspects.map((s) => [s.id, s.findings[0].reason]), [["T1", "test-target-missing"], ["T2", "test-target-moved"]]);
  assert.match(r.suspects[1].findings[0].evidence, /now crates\/net\/tests\/it\/wire\.rs/);
});

test("auditRows: a [[test]] block with a path is a target; one without a file is not", () => {
  const sources = {
    ...SOURCES,
    "crates/net/Cargo.toml": "[package]\nname = \"engine-net\"\n\n[[test]]\nname = \"vectors\"\npath = \"checks/vectors.rs\"\n\n[[test]]\nname = \"orphan\"\n",
    "crates/net/checks/vectors.rs": "#[test]\nfn vec_case() {}\n",
  };
  const r = auditRows([
    row("V1", "`cargo test -p engine-net --test vectors vec_case`"),
    row("V2", "`cargo test -p engine-net --test orphan`"),
  ], memoryCtx(sources));
  assert.deepEqual(r.suspects.map((s) => s.id), ["V2"]);
  assert.equal(r.suspects[0].findings[0].reason, "test-target-missing");
});

test("auditRows: path-missing says who removed it; path-moved when the crate still has the name", () => {
  const ctx = memoryCtx(SOURCES, {
    removedBy: (p) => (p === "crates/net/src/lagcomp.rs" ? { sha: "3151bd46", subject: "auto: backup 2026-10-16" } : false),
    deleted: new Set(["crates/net/src/ghost.rs"]),
  });
  const r = auditRows([
    row("MP5", "Owns `crates/net/src/lagcomp.rs`. Accept: fair hits."),
    row("MP6", "Owns `crates/net/src/wire.rs`."),
    row("MP7", "Owns `crates/net/src/never.rs`."),
    row("MP8", "Owns `crates/net/src/ghost.rs`."),
    row("MP9", "Owns `crates/save/src/lib.rs` and `crates/net/src/lib.rs`."),
  ], ctx);
  const byId = Object.fromEntries(r.suspects.map((s) => [s.id, s.findings[0]]));
  assert.equal(byId.MP5.reason, "path-missing");
  assert.match(byId.MP5.evidence, /crates\/net\/src\/lagcomp\.rs: removed by 3151bd46 "auto: backup 2026-10-16"/);
  assert.equal(byId.MP6.reason, "path-moved");
  assert.match(byId.MP6.evidence, /now crates\/net\/tests\/it\/wire\.rs/);
  assert.match(byId.MP7.evidence, /never in git history/);
  assert.match(byId.MP8.evidence, /deleted in the working tree, not committed yet/);
  assert.equal(byId.MP9, undefined);
});

test("auditRows: rows naming nothing checkable are counted, not flagged", () => {
  const r = auditRows([row("Z1", "net: just words."), row("Z2", "`cargo build -p engine-net`")], memoryCtx(SOURCES));
  assert.deepEqual([r.suspects.length, r.unchecked], [0, 2]);
});

// ---- closure notes ----------------------------------------------------------------------

test("closureNotes: only a closure marker starts a note, and the next marker ends it", () => {
  const text = "Owns `crates/net/src/gone.rs`. Progress 2026-10-19: the new home is `crates/net/src/lib.rs`. Tick audit 2026-10-19: closed by 3ba9bfd. Landed 1d9e19f76.";
  const notes = closureNotes(text);
  assert.equal(notes.length, 3);
  assert.match(notes[0], /^Tick audit 2026-10-19:$/);
  assert.match(notes[1], /^closed by 3ba9bfd\.$/);
  assert.match(notes[2], /^Landed 1d9e19f76\.$/);
  assert.ok(!notes.some((n) => n.includes("Progress")), "the running record is not a closure note");
  assert.deepEqual(closureNotes("no marker here, just `crates/net/src/lib.rs`"), []);
});

test("closureEvidence: a real commit, a path, a run and a ticked row clear; no evidence does not", () => {
  const ctx = {
    files: new Set(["crates/net/src/lib.rs", "docs/plan/q.md"]),
    commitExists: (sha) => sha === "3ba9bfd",
  };
  const tickedIds = new Set(["OK2"]);
  const ev = (note) => closureEvidence(note, ctx, { crateDirs: ["crates/net"], tickedIds });
  assert.deepEqual(ev("Tick audit 2026-10-19: closed by 3ba9bfd.").map((e) => [e.kind, e.value]), [["commit", "3ba9bfd"]]);
  assert.deepEqual(ev("Tick audit 2026-10-19: closed by deadbee."), [], "a bogus sha is not a close");
  assert.deepEqual(ev("Tick audit 2026-10-19: the old file is gone."), [], "a note with no evidence keeps flagging");
  assert.deepEqual(ev("Tick audit 2026-10-19: LATER per docs/plan/q.md."), [], "LATER is a park, not a close");
  assert.deepEqual(ev("Tick audit 2026-10-19: replaced by OK2.").map((e) => e.value), ["OK2"]);
  assert.deepEqual(ev("Tick audit 2026-10-19: replaced by OPEN2."), [], "the replacement must be ticked");
  assert.deepEqual(ev("Tick audit 2026-10-19: the new home is `crates/net/src/lib.rs`.").map((e) => e.value), ["crates/net/src/lib.rs"]);
  assert.deepEqual(ev("Tick audit 2026-10-19: `crates/net/src/missing.rs` is gone."), [], "a gone path is not proof");
  assert.deepEqual(ev("Landed 39057462548."), [], "a bare number is not a run id");
  assert.deepEqual(ev("Landed run 39057462548.").map((e) => e.kind), ["run"]);
});

test("auditRows: a closure note drops the finding; a note with no evidence keeps it", () => {
  const ctx = memoryCtx(SOURCES, { commitExists: (sha) => sha === "3ba9bfd" });
  const text = "Owns `crates/net/src/gone.rs`. Accept: `cargo test -p engine-net --lib snapshot`.";
  const r = auditRows([
    row("CL1", `${text} Tick audit 2026-10-19: the old path went in 3ba9bfd.`),
    row("CL2", `${text} Tick audit 2026-10-19: the old path is gone.`),
    row("CL3", `${text} Tick audit 2026-10-19: replaced by CL5.`),
    row("CL4", `${text} Tick audit 2026-10-19: replaced by CL9.`),
    row("CL5", "net: the replacement. Accept: `cargo test -p engine-net --lib snapshot`."),
    row("CL6", `${text} The new home is \`crates/net/src/lib.rs\`.`),
  ], ctx);
  assert.deepEqual(r.suspects.map((s) => s.id), ["CL2", "CL4", "CL6"]);
  assert.deepEqual(r.closures.map((c) => [c.id, c.evidence[0].kind]), [["CL1", "commit"], ["CL3", "row"]]);
});

// ---- rows from the board ----------------------------------------------------------------

function tempDir(name) {
  return mkdtempSync(join(tmpdir(), `ticked-audit-${name}-`));
}

function put(dir, rel, text) {
  mkdirSync(dirname(join(dir, rel)), { recursive: true });
  writeFileSync(join(dir, rel), text);
}

const BOARD = `# Board

### Net (owner 2026-10-17)

- [x] OK1 net: snapshot. Accept: \`cargo test -p engine-net --lib snapshot\`.
- [x] MP5 net: lag compensation. Owns
  \`crates/net/src/lagcomp.rs\`. Accept: \`cargo test -p engine-net --lib window_bounded\`.
- [ ] OPEN1 net: not done. Owns \`crates/net/src/open.rs\`.
- [x] GONE1 viewer: dropped crate. Accept: \`cargo test -p engine-viewer --lib\`.
`;

const ARCHIVE = `# Done rows

- [x] ARC1 net: old row moved out
  Owns \`crates/net/src/replicate.rs\`.
- [x] OK1 net: duplicate of a live row. Owns \`crates/net/src/never_looked_at.rs\`.
- [x] OPEN1 net: ticked here, open on the live board.
  Owns \`crates/net/src/also_ignored.rs\`.
`;

test("tickedRows: live ticked rows, then archived rows read whole, live board first", () => {
  const dir = tempDir("rows");
  try {
    put(dir, "docs/SPRINT.md", BOARD);
    put(dir, "docs/archive/SPRINT-done-2026-10-01.md", ARCHIVE);
    put(dir, "docs/archive/SPRINT-2026-10-05.md", "- [x] OLD1 net: older snapshot. Owns `crates/net/src/old.rs`.\n");
    put(dir, "docs/SPRINT-ARCHIVE.md", "- [x] OLD2 net: oldest.\n");
    const ids = (rows) => rows.map((r) => `${r.id}@${r.source}`);
    assert.deepEqual(ids(tickedRows(dir, { history: false })), ["OK1@board", "MP5@board", "GONE1@board", "ARC1@docs/archive/SPRINT-done-2026-10-01.md"]);
    const all = tickedRows(dir);
    assert.deepEqual(ids(all).slice(4), ["OLD1@docs/archive/SPRINT-2026-10-05.md", "OLD2@docs/SPRINT-ARCHIVE.md"]);
    assert.match(all.find((r) => r.id === "MP5").text, /crates\/net\/src\/lagcomp\.rs.*window_bounded/, "continuation lines are part of the row");
    assert.match(all.find((r) => r.id === "ARC1").text, /replicate\.rs/, "an archived row keeps its continuation lines");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("tickedRows: the real MP5 row (archived) still carries its lagcomp.rs path", () => {
  const mp5 = tickedRows().find((r) => r.id === "MP5");
  assert.ok(mp5, "MP5 is a ticked row somewhere in docs/");
  assert.ok(sourcePaths(mp5.text).includes("crates/net/src/lagcomp.rs"));
});

// ---- the CLI against a throwaway git repo -----------------------------------------------

function git(dir, ...args) {
  const env = { ...process.env };
  for (const k of ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"]) delete env[k];
  const r = spawnSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8", env });
  assert.equal(r.status, 0, `git ${args.join(" ")}: ${r.stderr}`);
  return r.stdout.trim();
}

function audit(dir, ...flags) {
  // The ceiling keeps git from finding a repo above the temp dir when the dir itself has none.
  const env = { ...process.env, GIT_CEILING_DIRECTORIES: tmpdir() };
  const r = spawnSync(process.execPath, [TOOL, "--root", dir, ...flags], { encoding: "utf8", env });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

function makeRepo() {
  const dir = tempDir("repo");
  put(dir, "docs/SPRINT.md", BOARD);
  put(dir, "docs/archive/SPRINT-done-2026-10-01.md", ARCHIVE);
  put(dir, "crates/net/Cargo.toml", "[package]\nname = \"engine-net\"\n");
  put(dir, "crates/net/src/lib.rs", "//! net\npub mod lagcomp;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn snapshot_roundtrip() {}\n}\n");
  put(dir, "crates/net/src/lagcomp.rs", "#[test]\nfn window_bounded() {}\n");
  put(dir, "crates/net/src/replicate.rs", "pub fn replicate() {}\n");
  git(dir, "init", "-q");
  git(dir, "add", "-A");
  git(dir, "commit", "-q", "-m", "net: everything lands");
  return dir;
}

// A repo whose row names crates/net/src/old.rs, a file that was committed and then deleted.
function makeClosureRepo() {
  const dir = tempDir("closure");
  put(dir, "docs/SPRINT.md", "# Board\n");
  put(dir, "crates/net/Cargo.toml", "[package]\nname = \"engine-net\"\n");
  put(dir, "crates/net/src/lib.rs", "#[cfg(test)]\nmod tests {\n    #[test]\n    fn snapshot_roundtrip() {}\n}\n");
  put(dir, "crates/net/src/old.rs", "pub fn old() {}\n");
  git(dir, "init", "-q");
  git(dir, "add", "-A");
  git(dir, "commit", "-q", "-m", "net: the old seam");
  rmSync(join(dir, "crates/net/src/old.rs"));
  git(dir, "add", "-A");
  git(dir, "commit", "-q", "-m", "rot: drop the old seam");
  return dir;
}

test("CLI: a clean repo exits 0; an untracked file that is on disk counts as there", () => {
  const dir = makeRepo();
  try {
    const first = audit(dir, "--no-history");
    assert.equal(first.code, 1, "GONE1 names a crate this repo does not have");
    assert.match(first.out, /^SUMMARY 1 suspect row\(s\) of 4 ticked \[crate-missing 1\]/m);
    put(dir, "crates/net/src/fresh.rs", "pub fn fresh() {}\n");
    put(dir, "docs/SPRINT.md", BOARD.replace(/- \[x\] GONE1.*\n/, "- [x] FRESH1 net: not committed yet. Owns `crates/net/src/fresh.rs`.\n"));
    const ok = audit(dir);
    assert.equal(ok.code, 0, ok.out + ok.err);
    assert.match(ok.out, /^SUMMARY 0 suspect row\(s\) of 4 ticked/m);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI: a swept file makes its row suspect, with the removing commit as evidence", () => {
  const dir = makeRepo();
  try {
    rmSync(join(dir, "crates/net/src/lagcomp.rs"));
    git(dir, "add", "-A");
    git(dir, "commit", "-q", "-m", "auto: backup 2026-10-16 22:30 (27 files) [skip ci]");
    const sha = git(dir, "rev-parse", "--short", "HEAD");
    const r = audit(dir);
    assert.equal(r.code, 1, r.out + r.err);
    const line = r.out.split("\n").find((l) => l.startsWith("MP5"));
    assert.ok(line, r.out);
    assert.match(line, /^MP5 +filter-missing,path-missing +/);
    assert.ok(line.includes(`crates/net/src/lagcomp.rs: removed by ${sha} "auto: backup 2026-10-16 22:30 (27 files) [skip ci]"`), line);
    assert.ok(line.includes("window_bounded"), line);
    assert.match(r.out, /^GONE1 +crate-missing +-p engine-viewer: no such package/m);
    assert.doesNotMatch(r.out, /^ARC1|^OK1|^OPEN1/m, "replicate.rs still exists, so the archived row is clean");
    assert.match(r.out, /^SUMMARY 2 suspect row\(s\) of 4 ticked \[crate-missing 1, filter-missing 1, path-missing 1\]/m);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI: a file deleted on disk but not yet committed is caught", () => {
  const dir = makeRepo();
  try {
    rmSync(join(dir, "crates/net/src/replicate.rs"));
    const r = audit(dir);
    assert.equal(r.code, 1);
    assert.match(r.out, /^ARC1 +path-missing +crates\/net\/src\/replicate\.rs: deleted in the working tree, not committed yet/m);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI: --json carries the same suspects; one run is read-only", () => {
  const dir = makeRepo();
  try {
    const before = git(dir, "status", "--porcelain");
    const r = audit(dir, "--json");
    assert.equal(r.code, 1);
    const j = JSON.parse(r.out);
    assert.equal(j.ticked, 4);
    assert.deepEqual(j.suspects.map((s) => s.id), ["GONE1"]);
    assert.equal(j.suspects[0].findings[0].reason, "crate-missing");
    assert.equal(j.suspects[0].source, "board");
    assert.equal(git(dir, "status", "--porcelain"), before, "the audit edits nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI: a closure note clears only evidence that exists", () => {
  const dir = makeClosureRepo();
  try {
    const sha = git(dir, "rev-parse", "--short", "HEAD");
    const run = (note, ...flags) => {
      put(dir, "docs/SPRINT.md", `# Board\n\n- [x] LOST1 net: the old seam. Owns \`crates/net/src/old.rs\`. ${note}\n- [x] OK2 net: the replacement. Accept: \`cargo test -p engine-net --lib snapshot\`.\n- [ ] OPEN2 net: not done.\n`);
      return audit(dir, "--no-history", ...flags);
    };

    // A commit git knows clears the row; the JSON names the evidence.
    const real = run(`Tick audit 2026-10-19: the seam went in ${sha}.`, "--json");
    assert.equal(real.code, 0, real.out + real.err);
    const j = JSON.parse(real.out);
    assert.deepEqual(j.suspects, []);
    assert.equal(j.closures.length, 1);
    assert.deepEqual(j.closures[0].evidence, [{ kind: "commit", value: sha }]);
    assert.deepEqual(j.closures[0].reasons, ["path-missing"]);

    // The same note with a bogus sha keeps the row flagged.
    const bogus = run("Tick audit 2026-10-19: the seam went in deadbee.");
    assert.equal(bogus.code, 1, bogus.out + bogus.err);
    assert.match(bogus.out, /^LOST1 +path-missing +crates\/net\/src\/old\.rs/m);
    assert.doesNotMatch(bogus.out, /^OK2/m);

    // A note that names no evidence keeps flagging; so does a LATER park note.
    assert.equal(run("Tick audit 2026-10-19: the old seam is gone.").code, 1);
    assert.equal(run("Tick audit 2026-10-19: LATER per docs/plan/q.md.").code, 1);

    // A replacement row clears only when it exists and is ticked.
    assert.equal(run("Tick audit 2026-10-19: replaced by OK2.").code, 0);
    assert.equal(run("Tick audit 2026-10-19: replaced by OPEN2.").code, 1, "the open row is not a ticked replacement");
    assert.equal(run("Tick audit 2026-10-19: replaced by NOPE2.").code, 1, "an absent row is no replacement");

    // A path in the row body is not a closure note: the note has to say it closed the row.
    const body = run("The new home is `crates/net/src/lib.rs`.");
    assert.equal(body.code, 1);
    assert.match(body.out, /^LOST1 +path-missing/m);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI: usage errors and non-git roots exit 2, --help exits 0", () => {
  const bad = spawnSync(process.execPath, [TOOL, "--nope"], { encoding: "utf8" });
  assert.equal(bad.status, 2);
  assert.match(bad.stderr, /unknown argument --nope/);
  const help = spawnSync(process.execPath, [TOOL, "--help"], { encoding: "utf8" });
  assert.equal(help.status, 0);
  assert.match(help.stdout, /Exit codes: 0 no suspect row/);
  const dir = tempDir("nogit");
  try {
    put(dir, "docs/SPRINT.md", BOARD);
    const r = audit(dir);
    assert.equal(r.code, 2, r.out + r.err);
    assert.match(r.err, /git ls-files failed/);
    const none = audit(join(dir, "missing"));
    assert.equal(none.code, 2);
    assert.match(none.err, /board missing/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
