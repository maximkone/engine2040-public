// OPT36 unify test: real tree passes, seeded extra feature fails.
import assert from "node:assert/strict";
import test from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkUnify, findViolations, parseDeps, resolveViolations } from "./feature-unify.mjs";
const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
test("real tree passes", () => {
  assert.deepEqual(checkUnify(repoDir), []);
});
test("seeded extra feature fails", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png"] }');
  const member = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png", "avif"] }');
  const bad = findViolations("seed", member, union);
  assert.ok(bad.length === 1 && bad[0].includes("avif"), "seeded avif must fail");
});
test("subset passes", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png", "hdr"] }');
  const member = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png"] }');
  assert.deepEqual(findViolations("seed", member, union), []);
});
test("workspace-inherited no-defaults follows the workspace root", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", default-features = false, features = ["png", "hdr"] }');
  const member = parseDeps('[dependencies]\nimage = { workspace = true, features = ["png"] }');
  const root = parseDeps('[workspace.dependencies]\nimage = { version = "0.25", default-features = false }', "workspace.dependencies");
  assert.deepEqual(findViolations("seed", member, union, root), []);
});
test("a workspace member that re-enables defaults still fails", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", default-features = false, features = ["png"] }');
  const member = parseDeps('[dependencies]\nimage = { workspace = true, features = ["png"] }');
  const root = parseDeps('[workspace.dependencies]\nimage = { version = "0.25" }', "workspace.dependencies");
  assert.deepEqual(findViolations("seed", member, union, root), ["seed: image needs default-features = false"]);
});

// SW12: the verify step is the metadata twin of findViolations. cargo resolves
// inheritance, renames, default features and target gating, so the feature set
// a member really builds with is what the union must hold.
test("metadata resolve: a host feature outside the union fails, a wasm-only one passes", () => {
  const member = (id, name, dep) => ({ id, name, dependencies: [dep] });
  const metadata = {
    workspace_members: ["h", "m1", "m2"],
    packages: [
      { id: "h", name: "engine-workspace-hack", dependencies: [{ name: "uuid", features: ["serde", "v4"], uses_default_features: true, path: null, target: null }] },
      member("m1", "engine-scene", { name: "uuid", features: ["serde", "v4"], uses_default_features: true, path: null, target: null }),
      // A target-gated dependency is a separate resolve: uuid's `js` is wasm32's.
      member("m2", "engine-web", { name: "uuid", features: ["js"], uses_default_features: true, path: null, target: 'cfg(target_arch = "wasm32")' }),
    ],
  };
  assert.deepEqual(resolveViolations(metadata), [], "the live shape passes");

  // The same request on the host target is outside the union.
  metadata.packages[1].dependencies[0].features = ["serde", "v7"];
  const bad = resolveViolations(metadata);
  assert.equal(bad.length, 1);
  assert.match(bad[0], /engine-scene: uuid feature "v7" is outside the workspace-hack union/);
  assert.match(bad[0], /crates\/workspace-hack\/Cargo\.toml/);
  assert.match(bad[0], /fix: /);
});

test("metadata resolve: defaults the union turned off, a path dep and an unlisted external behave", () => {
  const metadata = {
    workspace_members: ["h", "m1", "m2", "m3"],
    packages: [
      { id: "h", name: "engine-workspace-hack", dependencies: [{ name: "image", features: ["png"], uses_default_features: false, path: null, target: null }] },
      { id: "m1", name: "engine-render", dependencies: [{ name: "image", features: [], uses_default_features: true, path: null, target: null }] },
      // A path dependency is a workspace edge, not part of the external union.
      { id: "m2", name: "engine-game", dependencies: [{ name: "engine-core", features: ["x"], uses_default_features: true, path: "/x", target: null }] },
      // An external the union does not list is out of scope, like feature-unify.
      { id: "m3", name: "engine-net", dependencies: [{ name: "tokio", features: ["full"], uses_default_features: true, path: null, target: null }] },
    ],
  };
  assert.deepEqual(resolveViolations(metadata), ["engine-render: image needs default-features = false, the union turns them off fix: set default-features = false on image in the engine-render manifest"]);
  metadata.packages[1].dependencies[0].uses_default_features = false;
  assert.deepEqual(resolveViolations(metadata), [], "defaults off matches the union");
  assert.deepEqual(resolveViolations({ workspace_members: [], packages: [] }), ["engine-workspace-hack: the union crate is missing from the workspace fix: restore crates/workspace-hack with its externals"]);
});
