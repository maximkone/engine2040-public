// Run: node --test tools/blender/machine_gen.test.mjs
// The machine generator's numbers run in plain Python (no Blender): these tests
// call it through --stats, --list and the usage/budget exits, then check the
// geometry invariants with a short python snippet.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const script = join(dir, "machine_gen.py");
const propsDir = join(dir, "..", "..", "content", "generated", "props");
// One machine prop stands in the arena ring, so it holds the potato cap
// (`crates/assets/src/budget.rs::POTATO`, 1500 tris).
const POTATO_TRIS = 1500;
const KIT_MACHINES = [
  ["machine_ballista", "ballista"],
  ["machine_crane", "crane"],
  ["machine_cart", "cart"],
];

function run(args) {
  try {
    const out = execFileSync("python", [script, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { status: 0, out, err: "" };
  } catch (e) {
    return { status: e.status ?? 1, out: e.stdout ?? "", err: e.stderr ?? "" };
  }
}

function stats(preset, seed, extra = []) {
  const r = run(["--stats", "--preset", preset, "--seed", String(seed), ...extra]);
  assert.equal(r.status, 0, r.err);
  return JSON.parse(r.out);
}

const PRESETS = ["machine", "ballista", "crane", "cart"];
const MIRRORED = ["ballista", "crane", "cart"];

test("--list names the four presets with their budgets", () => {
  const r = run(["--list"]);
  assert.equal(r.status, 0, r.err);
  const listing = JSON.parse(r.out).presets;
  assert.deepEqual(Object.keys(listing).sort(), [...PRESETS].sort());
  for (const name of PRESETS) {
    assert.ok(listing[name].budget > 0, `${name} has a budget`);
    assert.ok(listing[name].title.length > 0, `${name} has a title`);
  }
});

test("same preset and seed give the same mesh (sha256 of the vertices, faces and slots)", () => {
  for (const preset of PRESETS) {
    const a = stats(preset, 7);
    const b = stats(preset, 7);
    assert.equal(a.sha256, b.sha256, preset);
    assert.equal(a.tris, b.tris, preset);
  }
});

test("a different seed gives a different mesh", () => {
  for (const preset of PRESETS) {
    assert.notEqual(stats(preset, 7).sha256, stats(preset, 8).sha256, preset);
  }
});

test("a text seed is hashed and stays deterministic", () => {
  const a = stats("cart", "bridge-3");
  const b = stats("cart", "bridge-3");
  assert.equal(a.sha256, b.sha256);
  assert.notEqual(a.sha256, stats("cart", "bridge-4").sha256);
});

test("every preset stays under its triangle budget and uses at most three material slots", () => {
  for (const preset of PRESETS) {
    for (const seed of [1, 2, 3, 4]) {
      const s = stats(preset, seed);
      assert.ok(s.tris > 0, `${preset} ${seed} has triangles`);
      assert.ok(s.tris <= s.budget, `${preset} ${seed}: ${s.tris} <= ${s.budget}`);
      assert.ok(s.materials.length <= 3, `${preset} ${seed} materials ${s.materials}`);
      for (const m of s.materials) assert.ok(["wood", "iron", "tarp"].includes(m), m);
    }
  }
});

test("mirrored presets are symmetric across Y and stand on the floor", () => {
  for (const preset of MIRRORED) {
    for (const seed of [1, 2, 3]) {
      const s = stats(preset, seed);
      const { min, max } = s.bounds_m;
      assert.ok(Math.abs(min[1] + max[1]) < 1e-3, `${preset} ${seed} y symmetric: ${min[1]} ${max[1]}`);
      assert.ok(Math.abs(min[2]) < 1e-3, `${preset} ${seed} on the floor: ${min[2]}`);
    }
  }
});

test("geometry invariants: faces point out, subdivision keeps area, text seeds hash", () => {
  const code = `
import sys, json
sys.path.insert(0, ${JSON.stringify(dir)})
import machine_gen as m
# a block's six faces point away from its centre
mesh = m.Mesh()
c = (0.3, -0.2, 0.5)
fs = mesh.block(c, ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.5, 0.4, 0.3), 0)
assert all(m.vdot(mesh.normal(f), m.vsub(mesh.centroid(f), c)) > 0 for f in fs)
# an extruded wall and its new top point out of the solid
top, sides = mesh.extrude(fs[4], 0.2)
ctr = (0.3, -0.2, 0.6)
assert all(m.vdot(mesh.normal(f), m.vsub(mesh.centroid(f), ctr)) > 0 for f in sides + [top])
# a prism: the cap faces +axis and the walls face out
p = m.Mesh()
p.prism((0, 0, 0), (0, 0, 1), 0.5, 1.0, 8, 0)
assert p.tri_count() == 22
# a 2 x 3 subdivision of a unit square keeps its area
q = m.Mesh()
quad = q.face([q.vert(v) for v in ((0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0))], 0)
area = 0.0
for f in q.subdivide(quad, 2, 3):
    pts = q.corners(f)
    area += m.vlen(m.vcross(m.vsub(pts[1], pts[0]), m.vsub(pts[3], pts[0])))
assert abs(area - 1.0) < 1e-9, area
# the rng is inclusive at both ends and repeats for a seed
r = m.Rng("x")
vals = {r.randint(2, 4) for _ in range(200)}
assert vals == {2, 3, 4}, vals
assert m.Rng("x").random() == m.Rng("x").random()
print("ok")
`;
  const out = execFileSync("python", ["-c", code], { encoding: "utf8" });
  assert.equal(out.trim(), "ok");
});

test("unknown preset exits 2 and names the choices", () => {
  const r = run(["--stats", "--preset", "trebuchet-9000"]);
  assert.equal(r.status, 2);
  assert.ok(r.err.includes("no preset named trebuchet-9000"));
  assert.ok(r.err.includes("ballista"));
});

test("a mesh over budget exits 3 with E_MACHINE and the budget", () => {
  const r = run(["--stats", "--preset", "cart", "--seed", "7", "--max-tris", "10"]);
  assert.equal(r.status, 3);
  assert.ok(r.err.includes("E_MACHINE"));
  assert.ok(r.err.includes("budget"));
});

test("an unknown flag or a bad --max-tris exits 2", () => {
  assert.equal(run(["--stats", "--spin"]).status, 2);
  assert.equal(run(["--stats", "--max-tris", "lots"]).status, 2);
});

test("writing a GLB without Blender says so and exits 3", () => {
  const work = mkdtempSync(join(tmpdir(), "machine-gen-"));
  try {
    const r = run([join(work, "cart.glb"), "--preset", "cart", "--seed", "7"]);
    assert.equal(r.status, 3);
    assert.ok(r.err.includes("needs Blender"), r.err);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
});

test("the_props_manifest_lists_the_three_machines_under_the_cap", () => {
  const manifest = JSON.parse(readFileSync(join(propsDir, "props_built.json"), "utf8"));
  assert.equal(manifest.props.length, manifest.count, "count matches the rows");
  const ids = manifest.props.map((p) => p.id);
  assert.deepEqual(
    ids.slice(-KIT_MACHINES.length),
    KIT_MACHINES.map(([id]) => id),
    "the three machines are appended, the 24 catalogue props keep their place",
  );
  const rows = new Map(manifest.props.map((p) => [p.id, p]));
  for (const [id, preset] of KIT_MACHINES) {
    const row = rows.get(id);
    assert.ok(row, `${id} is in the props manifest`);
    assert.equal(row.file, `${id}.glb`);
    assert.equal(row.build, `machine:${preset}`);
    assert.deepEqual(row.classes, [], `${id} belongs to no fighter class`);
    const stat = stats(preset, 7);
    assert.equal(row.triangles, stat.tris, `${id} carries the generator's tris`);
    assert.ok(row.triangles <= POTATO_TRIS, `${id}: ${row.triangles} <= ${POTATO_TRIS}`);
    const file = join(propsDir, row.file);
    assert.ok(statSync(file).size > 0, `${row.file} has bytes in the props kit`);
    const sidecar = JSON.parse(readFileSync(join(propsDir, `${id}.machine.json`), "utf8"));
    assert.equal(sidecar.seed, "7", `${id} was cooked at the pinned seed`);
    assert.equal(sidecar.sha256, stat.sha256, `${id} sidecar is the generator's bake`);
  }
});
