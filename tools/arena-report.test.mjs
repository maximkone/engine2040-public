// Run: node --test tools/arena-report.test.mjs
// CT82 manifest truth, CT45 VRAM caps, CT77 crate+torch receipt. Fixture trees
// prove each gate goes red; the real tree pins the structure that must hold.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { genBlock, readBlock, syncBlocks } from "./content-lib.mjs";
import {
  ARENA, arenaBlocks, checkManifest, checkPbrCaps, checkProofReport, checkSoak, checkSrcPair, LOCAL_HDRI, MATERIALS, pbrBody, soakBody, soakStats, srcReceiptBody,
} from "./arena-report.mjs";

function png(w, h) {
  const b = Buffer.alloc(32);
  b.write("\x89PNG\r\n\x1a\n", 0, "latin1");
  b.writeUInt32BE(13, 8);
  b.write("IHDR", 12);
  b.writeUInt32BE(w, 16);
  b.writeUInt32BE(h, 20);
  return b;
}

/** A tiny arena tree: 8 src files, the 16 required maps, one HDRI, a clean proof report. */
function fixture({ mapDim = 256 } = {}) {
  const base = mkdtempSync(join(tmpdir(), "arena-report-"));
  const root = join(base, "content", "arena");
  const local = join(root, "local-hdri");
  for (const d of ["src", "textures", "hdri", "local-hdri"]) mkdirSync(join(root, d), { recursive: true });
  mkdirSync(join(base, "content", "fighters", "lod"), { recursive: true });
  const mesh = { lods: [0, 1, 2, 3].map(() => ({ tris: 10, bytes: 1000 })) };
  writeFileSync(join(base, "content", "fighters", "lod", "hero.json"), JSON.stringify({ meshes: [mesh, mesh] }));
  writeFileSync(join(base, "content", "fighters", "manifest.json"), JSON.stringify({ variants: [{ texture: "1024x1024mips=11" }, { texture: "512x512mips=10" }] }));
  const files = {};
  const put = (rel, bytes) => {
    writeFileSync(join(root, rel), bytes);
    files[rel] = { sha256: createHash("sha256").update(bytes).digest("hex"), bytes: bytes.length };
  };
  for (const n of ["sand_floor", "stands", "walls", "gate_tunnel", "torch", "banner", "crate", "debris"]) put(`src/${n}.gltf`, Buffer.from(`{"asset":"${n}"}`));
  for (const m of MATERIALS) {
    for (const k of ["albedo_2k.jpg", "normal_2k.jpg", "roughness_2k.jpg", "orm.png"]) put(`textures/${m}_${k}`, png(mapDim, mapDim));
  }
  put("hdri/arena_day.hdr", Buffer.from("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2048 +X 4096\n"));
  writeFileSync(join(root, "manifest.json"), JSON.stringify({ files }));
  const report = {
    total_lod0_tris: 1920,
    bundle: { identical_twice: true },
    spatial: { actors: 11, findings: 0, verdict: "clean", hash: "cbf29ce484222325", identical_twice: true },
    assets: [{ asset: "crate", lod0_tris: 1536 }, { asset: "torch", lod0_tris: 384 }],
  };
  writeFileSync(join(root, "report.json"), JSON.stringify(report));
  return { root, local, files, report };
}

test("a clean fixture has no manifest problems", () => {
  const f = fixture();
  assert.deepEqual(checkManifest(f.root, f.local), []);
});

test("CT82: the stale pre-kit manifest is flagged", () => {
  const f = fixture();
  writeFileSync(join(f.root, "manifest_before.json"), "{}");
  assert.match(checkManifest(f.root, f.local).join("\n"), /manifest_before\.json: stale pre-kit manifest/);
});

test("CT82: bytes, hash, missing, unlisted and unlisted-required rows are named", () => {
  const f = fixture();
  writeFileSync(join(f.root, "src", "crate.gltf"), "{}");
  writeFileSync(join(f.root, "src", "torch.gltf"), Buffer.from(`{"asset":"XXXXX"}`));
  rmSync(join(f.root, "src", "banner.gltf"));
  writeFileSync(join(f.root, "src", "extra.gltf"), "{}");
  const man = JSON.parse(readFileSync(join(f.root, "manifest.json"), "utf8"));
  delete man.files["textures/cloth_orm.png"];
  writeFileSync(join(f.root, "manifest.json"), JSON.stringify(man));
  const out = checkManifest(f.root, f.local).join("\n");
  assert.match(out, /src\/crate\.gltf: 2 bytes on disk/);
  assert.match(out, /src\/torch\.gltf: sha256 differs/);
  assert.match(out, /src\/banner\.gltf: missing on disk/);
  assert.match(out, /src\/extra\.gltf: on disk but not in the manifest/);
  assert.match(out, /textures\/cloth_orm\.png: required map is not in the manifest/);
});

test("CT82: an HDRI moved to the local cache still counts, a missing one does not", () => {
  const f = fixture();
  const man = JSON.parse(readFileSync(join(f.root, "manifest.json"), "utf8"));
  const bytes = readFileSync(join(f.root, "hdri", "arena_day.hdr"));
  man.files["hdri/arena_night.hdr"] = { sha256: createHash("sha256").update(bytes).digest("hex"), bytes: bytes.length };
  writeFileSync(join(f.root, "manifest.json"), JSON.stringify(man));
  assert.match(checkManifest(f.root, f.local).join("\n"), /arena_night\.hdr: missing on disk/);
  writeFileSync(join(f.local, "arena_night.hdr"), bytes);
  assert.deepEqual(checkManifest(f.root, f.local), []);
});

test("CT45: the VRAM table reports dims and the bound set, and an oversize set fails the potato cap", () => {
  const f = fixture({ mapDim: 2048 });
  assert.match(pbrBody(f.root, f.local), /2048x2048 \| 22369621 \| yes/);
  assert.match(pbrBody(f.root, f.local), /potato tier[^\n]*FAIL/);
  assert.match(checkPbrCaps(f.root, f.local).join("\n"), /over the potato cap/);
  const small = fixture({ mapDim: 256 });
  assert.deepEqual(checkPbrCaps(small.root, small.local), []);
  assert.match(pbrBody(small.root, small.local), /potato tier[^\n]*PASS/);
});

test("CT77: the proof report must show 0 findings and cook x2", () => {
  const f = fixture();
  assert.deepEqual(checkProofReport(f.root), []);
  const bad = { ...f.report, spatial: { ...f.report.spatial, findings: 2 }, bundle: { identical_twice: false } };
  writeFileSync(join(f.root, "report.json"), JSON.stringify(bad));
  const out = checkProofReport(f.root).join("\n");
  assert.match(out, /spatial findings 2/);
  assert.match(out, /bundle cook is not identical twice/);
});

test("CT77: a pair that moved or went over budget is refused, and the receipt names the SHAs", () => {
  const f = fixture();
  assert.deepEqual(checkSrcPair(f.root), []);
  assert.match(srcReceiptBody(f.root), /\| `src\/crate\.gltf` \| 17 \| `[0-9a-f]{64}` \| match \| 1536 \| 5000 \| 8388608 \|/);
  writeFileSync(join(f.root, "src", "crate.gltf"), "{}");
  assert.match(checkSrcPair(f.root).join("\n"), /src\/crate\.gltf: bytes differ from manifest\.json/);
  assert.match(srcReceiptBody(f.root), /DRIFT/);
  const big = { ...f.report, assets: [{ asset: "crate", lod0_tris: 9000 }, { asset: "torch", lod0_tris: 384 }] };
  writeFileSync(join(f.root, "report.json"), JSON.stringify(big));
  assert.match(checkSrcPair(f.root).join("\n"), /9000 tris over the 5000 budget/);
});

test("CT61: the soak baseline sums kit, atlases and 8 hero buffers, keeps ms PENDING, and fails over the potato cap", () => {
  const f = fixture();
  const s = soakStats(f.root, f.local);
  assert.equal(s.atlases, 5592405 + 1398101);
  assert.equal(s.meshBytes, 8 * 2000);
  assert.equal(s.total, s.active + s.atlases + s.meshBytes);
  assert.match(soakBody(f.root, f.local), /CPU ms \/ GPU ms \| PENDING/);
  assert.deepEqual(checkSoak(f.root, f.local), []);
  const big = fixture({ mapDim: 2048 });
  assert.match(checkSoak(big.root, big.local).join(" "), /over the potato cap/);
});

test("generated blocks are written once and then read back identical", () => {
  const f = fixture();
  const specs = arenaBlocks(f.root, f.local).map((s) => ({ ...s, seed: `# t\n\n${genBlock(s.name, "")}\n` }));
  const root = mkdtempSync(join(tmpdir(), "arena-blocks-"));
  for (const d of ["content", "content/arena", "content/arena/pbr"]) mkdirSync(join(root, d));
  assert.equal(syncBlocks(specs, { root }).length, 3);
  assert.equal(syncBlocks(specs, { root, write: true }).length, 3);
  assert.deepEqual(syncBlocks(specs, { root }), []);
  assert.equal(readBlock(readFileSync(join(root, specs[1].file), "utf8"), "src-receipt"), specs[1].body);
  assert.equal(readBlock(readFileSync(join(root, specs[2].file), "utf8"), "soak-baseline"), specs[2].body);
});

test("real tree: stale manifest deleted, the 16 maps listed, caps met, blocks current", () => {
  assert.equal(existsSync(join(ARENA, "manifest_before.json")), false);
  const files = JSON.parse(readFileSync(join(ARENA, "manifest.json"), "utf8")).files;
  for (const m of MATERIALS) {
    for (const k of ["albedo_2k.jpg", "normal_2k.jpg", "roughness_2k.jpg", "orm.png"]) assert.ok(files[`textures/${m}_${k}`], `${m}_${k}`);
  }
  assert.deepEqual(checkPbrCaps(ARENA, LOCAL_HDRI), []);
  assert.deepEqual(syncBlocks(arenaBlocks()), []);
});
