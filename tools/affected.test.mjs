import assert from "node:assert/strict";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { mkdtempSync } from "node:fs";
import { affectedToolTests, buildSnapshot, computeAffected, diffSnapshots, greenRecordPath, hashChangedFiles, laneCommand, manifestFingerprint, metadataCachePath, parseArgs, readCachedMetadata, readLastGreen, shouldSkip, toolCommand, writeCachedMetadata, writeLastGreen } from "./affected.mjs";

// Synthetic cargo-metadata shape: gamma <- beta <- alpha (beta deps gamma,
// alpha deps beta), manifests under /ws/crates/<name>/.
const meta = (pkgs) => ({
  workspace_members: pkgs.map((p) => `id:${p.name}`),
  packages: pkgs.map((p) => ({
    id: `id:${p.name}`,
    name: p.name,
    manifest_path: `/ws/crates/${p.dir ?? p.name}/Cargo.toml`,
    dependencies: (p.deps ?? []).map((d) => ({ name: d, path: `../${d}` })),
  })),
  resolve: {
    nodes: pkgs.map((p) => ({
      pkg: `id:${p.name}`,
      deps: (p.deps ?? []).map((d) => ({ pkg: `id:${d}` })),
    })),
  },
});
const chain = () => meta([
  { name: "engine-alpha", deps: ["engine-beta"] },
  { name: "engine-beta", deps: ["engine-gamma"] },
  { name: "engine-gamma" },
]);

test("a mid-chain edit closes over reverse dependents only", () => {
  const r = computeAffected(["crates/engine-beta/src/x.rs"], chain(), "/ws");
  assert.deepEqual(r.changedPackages, ["engine-beta"]);
  assert.deepEqual(r.affected, ["engine-alpha", "engine-beta"]);
  assert.equal(r.total, 3);
  assert.equal(r.workspaceWide, false);
  assert.equal(r.coreFoundation, false);
  assert.deepEqual(r.unmapped, []);
});

test("a leaf edit affects nobody else; a root edit affects nobody upstream", () => {
  assert.deepEqual(computeAffected(["crates/engine-gamma/src/y.rs"], chain(), "/ws").affected,
    ["engine-alpha", "engine-beta", "engine-gamma"]);
  assert.deepEqual(computeAffected(["crates/engine-alpha/src/z.rs"], chain(), "/ws").affected, ["engine-alpha"]);
});

test("a leaf change selects its dependents only, skipping unrelated crates", () => {
  const m = meta([
    { name: "engine-leaf" },
    { name: "engine-mid", deps: ["engine-leaf"] },
    { name: "engine-solo" },
  ]);
  const r = computeAffected(["crates/engine-leaf/src/lib.rs"], m, "/ws");
  assert.deepEqual(r.changedPackages, ["engine-leaf"]);
  assert.deepEqual(r.affected, ["engine-leaf", "engine-mid"]);
  assert.equal(r.total, 3);
  assert.deepEqual(r.unmapped, []);
});

test("a workspace-wide file resolves the whole workspace", () => {
  for (const f of ["Cargo.toml", "Cargo.lock", "deny.toml", ".cargo/config.toml"]) {
    const r = computeAffected([f], chain(), "/ws");
    assert.deepEqual(r.affected, ["engine-alpha", "engine-beta", "engine-gamma"], f);
    assert.equal(r.workspaceWide, true);
  }
});

test("a tools/ edit is not workspace-wide: it gates no crates", () => {
  const r = computeAffected(["tools/proof.mjs"], chain(), "/ws");
  assert.equal(r.workspaceWide, false);
  assert.deepEqual(r.affected, []);
  assert.deepEqual(r.changedPackages, []);
  assert.deepEqual(r.unmapped, []);
});

test("an engine-core edit resolves all: the foundation closure", () => {
  const m = meta([{ name: "engine-core" }, { name: "engine-foo", deps: ["engine-core"] }]);
  const r = computeAffected(["crates/engine-core/src/fixed.rs"], m, "/ws");
  assert.deepEqual(r.affected, ["engine-core", "engine-foo"]);
  assert.equal(r.coreFoundation, true);
});

test("docs-only files are unmapped and gate nothing", () => {
  const r = computeAffected(["docs/README.md", "notes/x.md"], chain(), "/ws");
  assert.deepEqual(r.affected, []);
  assert.deepEqual(r.changedPackages, []);
  assert.deepEqual(r.unmapped, ["docs/README.md", "notes/x.md"]);
});

test("DIR_OWNERS maps tests/, fixtures/ and content/ to their crates", () => {
  const m = meta([{ name: "engine-proof" }, { name: "engine-core" }, { name: "engine-assets" }]);
  assert.deepEqual(computeAffected(["tests/redteam/basic.rs"], m, "/ws").changedPackages, ["engine-proof"]);
  assert.deepEqual(computeAffected(["fixtures/games/x/rules.json"], m, "/ws").changedPackages, ["engine-proof"]);
  assert.deepEqual(computeAffected(["content/sprites/y.png"], m, "/ws").changedPackages, ["engine-assets"]);
});

test("laneCommand carries -j 2, --locked and every affected crate, never --release", () => {
  const cmd = laneCommand(["engine-alpha", "engine-beta"]);
  assert.equal(cmd, "cargo test -j 2 --locked -p engine-alpha -p engine-beta --all-targets");
  assert.ok(!cmd.includes("--release"));
});

test("parseArgs: base/files/json forms", () => {
  assert.deepEqual(parseArgs([]), { base: "HEAD", files: null, json: false, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--json"]), { base: "HEAD", files: null, json: true, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--base", "main"]), { base: "main", files: null, json: false, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--base=abc123"]), { base: "abc123", files: null, json: false, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--files", "a.rs", "b.rs", "--json"]), { base: "HEAD", files: ["a.rs", "b.rs"], json: true, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--mark-green"]), { base: "HEAD", files: null, json: false, markGreen: true, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--files", "a.rs", "--mark-green"]), { base: "HEAD", files: ["a.rs"], json: false, markGreen: true, tiers: false, snapshotWrite: null, snapshotDiff: null });
  assert.deepEqual(parseArgs(["--tiers"]), { base: "HEAD", files: null, json: false, markGreen: false, tiers: true, snapshotWrite: null, snapshotDiff: null });
});

const toolList = ["tools/affected.test.mjs", "tools/proof.test.mjs"];

test("a tools/ edit gates only its own stem test", () => {
  assert.deepEqual(affectedToolTests(["tools/affected.mjs"], toolList), ["tools/affected.test.mjs"]);
  assert.deepEqual(affectedToolTests(["tools/proof.mjs"], toolList), ["tools/proof.test.mjs"]);
});

test("a tool test-file edit gates itself; a helper with no test gates nothing", () => {
  assert.deepEqual(affectedToolTests(["tools/proof.test.mjs"], toolList), ["tools/proof.test.mjs"]);
  assert.deepEqual(affectedToolTests(["tools/no-such-test-helper.mjs"], toolList), []);
});

test("crates-only and docs-only diffs gate no tool tests; workspace-wide gates all", () => {
  assert.deepEqual(affectedToolTests(["crates/engine-beta/src/x.rs"], toolList), []);
  assert.deepEqual(affectedToolTests(["docs/README.md"], toolList), []);
  assert.deepEqual(affectedToolTests(["Cargo.lock"], toolList), toolList);
});

test("unmappable tools/ changes keep the run-all-tool-tests fallback", () => {
  for (const f of ["tools", "tools/vendor/x.mjs", "tools/archive/y.mjs", "tools/sub/z.mjs", "tools/rust-quality.baseline.json", "tools/sim-clippy.toml"]) {
    assert.deepEqual(affectedToolTests([f], toolList), toolList, f);
  }
});

test("toolCommand runs the subset only, never the full list", () => {
  assert.equal(toolCommand(["tools/affected.test.mjs"]), "node --test tools/affected.test.mjs");
  assert.equal(toolCommand([]), "(no tool changes: tool gate skips tool tests)");
});

// Fresh fixture workspace (never the real repo): root manifests plus one member.
const fixtureWorkspace = () => {
  const root = mkdtempSync(join(tmpdir(), "affected-fp-"));
  mkdirSync(join(root, "crates", "foo"), { recursive: true });
  writeFileSync(join(root, "Cargo.toml"), "[workspace]\n");
  writeFileSync(join(root, "Cargo.lock"), "lock\n");
  writeFileSync(join(root, "crates", "foo", "Cargo.toml"), "[package]\nname = 'foo'\n");
  return root;
};

test("manifestFingerprint is stable, then moves when a manifest changes", () => {
  const root = fixtureWorkspace();
  const before = manifestFingerprint(root);
  assert.ok(typeof before === "string" && before.length > 0);
  assert.equal(manifestFingerprint(root), before);
  appendFileSync(join(root, "crates", "foo", "Cargo.toml"), "# touch\n");
  assert.notEqual(manifestFingerprint(root), before);
});

test("manifestFingerprint is null when the tree is unreadable", () => {
  assert.equal(manifestFingerprint(join(tmpdir(), "affected-fp-missing-xyz")), null);
});

test("metadata cache round-trips on fingerprint match, misses otherwise", () => {
  const root = fixtureWorkspace();
  const fp = manifestFingerprint(root);
  const fake = { workspace_members: [], packages: [] };
  assert.equal(readCachedMetadata(root, fp), null);
  writeCachedMetadata(root, fp, JSON.stringify(fake));
  assert.deepEqual(readCachedMetadata(root, fp), fake);
  assert.equal(readCachedMetadata(root, `${fp}0`), null);
});

test("metadataCachePath is stable per root and distinct per root", () => {
  const a = fixtureWorkspace();
  assert.equal(metadataCachePath(a), metadataCachePath(a));
  assert.notEqual(metadataCachePath(a), metadataCachePath(fixtureWorkspace()));
});

test("green-skip: identical files plus closure skips, any change runs", () => {
  const files = ["crates/foo/src/lib.rs"];
  const affected = ["engine-foo"];
  const green = { filesHash: hashChangedFiles(files), affectedHash: hashChangedFiles(affected), verdict: "green" };
  assert.equal(shouldSkip(files, affected, green).skip, true);
  assert.equal(shouldSkip(["crates/foo/src/other.rs"], affected, green).skip, false);
  assert.equal(shouldSkip(files, ["engine-bar"], green).skip, false);
  assert.equal(shouldSkip(files, affected, null).skip, false);
  assert.equal(shouldSkip(files, affected, { ...green, verdict: "red" }).skip, false);
});

test("green record round-trips per root and ignores corrupt records", () => {
  const root = fixtureWorkspace();
  assert.equal(readLastGreen(root), null);
  assert.equal(writeLastGreen(root, { filesHash: "aa", affectedHash: "bb" }), true);
  assert.deepEqual(readLastGreen(root), { filesHash: "aa", affectedHash: "bb", verdict: "green" });
  assert.notEqual(greenRecordPath(root), greenRecordPath(fixtureWorkspace()));
});

test("required tier adds build inputs beyond the affected closure", () => {
  const m = meta([
    { name: "engine-alpha", deps: ["engine-beta"] },
    { name: "engine-beta", deps: ["engine-gamma"] },
    { name: "engine-gamma" },
    { name: "engine-solo" },
  ]);
  const r = computeAffected(["crates/engine-beta/src/x.rs"], m, "/ws");
  assert.deepEqual(r.changedPackages, ["engine-beta"]);
  assert.deepEqual(r.affected, ["engine-alpha", "engine-beta"]);
  assert.deepEqual(r.required, ["engine-alpha", "engine-beta", "engine-gamma"]);
});

test("required tier is the whole workspace on workspace-wide edits", () => {
  const r = computeAffected(["Cargo.lock"], chain(), "/ws");
  assert.deepEqual(r.required, ["engine-alpha", "engine-beta", "engine-gamma"]);
});

test("snapshot round-trips tiers and diffs run-over-run", () => {
  const tiers = { modified: ["engine-beta"], affected: ["engine-alpha", "engine-beta"], required: ["engine-alpha", "engine-beta", "engine-gamma"] };
  const a = buildSnapshot({ base: "HEAD", changedFiles: ["crates/engine-beta/src/x.rs"], tiers, totalWorkspacePackages: 3 });
  assert.equal(a.v, 1);
  assert.deepEqual(a.tiers, tiers);
  const b = buildSnapshot({ base: "HEAD", changedFiles: ["crates/engine-beta/src/x.rs", "tools/affected.mjs"], tiers, totalWorkspacePackages: 3 });
  const d = diffSnapshots(a, b);
  assert.deepEqual(d.addedFiles, ["tools/affected.mjs"]);
  assert.deepEqual(d.removedFiles, []);
  assert.equal(d.same, false);
  assert.equal(diffSnapshots(a, a).same, true);
});
