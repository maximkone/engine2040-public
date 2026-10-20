import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { diffSurface, rustdocSurface, surfaceOf } from "./api-break.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "api-break.mjs");
const FIXTURES = join(HERE, "fixtures", "api-break");
const hasCargo = spawnSync("cargo", ["--version"], { encoding: "utf8", shell: false }).status === 0;

// One tiny rustdoc-JSON crate: `add()`, a public struct with one public field,
// an inherent impl with one method. Shapes match rustdoc JSON format 57.
function demoJson() {
  const fn = (inputs, output) => ({
    sig: { inputs, output, is_c_variadic: false },
    generics: { params: [], where_predicates: [] },
    header: { is_const: false, is_unsafe: false, is_async: false, abi: "Rust" },
    has_body: true,
  });
  const item = (id, name, visibility, inner, docs = "") => ({ id, name, visibility, docs, attrs: [], inner });
  const json = {
    format_version: 57,
    root: 0,
    index: {
      0: item(0, "crate", "public", { module: { is_crate: true, items: [1, 2], is_stripped: false } }),
      1: item(1, "add", "public", { function: fn([["a", { primitive: "u32" }], ["b", { primitive: "u32" }]], { primitive: "u32" }) }, "Adds two numbers."),
      2: item(2, "Counter", "public", {
        struct: {
          kind: { plain: { fields: [3], has_stripped_fields: false } },
          generics: { params: [], where_predicates: [] },
          impls: [4],
        },
      }),
      3: item(3, "value", "public", { struct_field: { primitive: "u32" } }),
      4: item(4, null, "default", {
        impl: {
          is_unsafe: false,
          generics: { params: [], where_predicates: [] },
          provided_trait_methods: [],
          trait: null,
          for: { resolved_path: { path: "Counter", id: 2, args: null } },
          items: [5],
          is_negative: false,
          is_synthetic: false,
          blanket_impl: null,
        },
      }),
      5: item(5, "new", "public", { function: fn([], { generic: "Self" }) }),
    },
    paths: {
      0: { crate_id: 0, path: ["demo"], kind: "module" },
      1: { crate_id: 0, path: ["demo", "add"], kind: "function" },
      2: { crate_id: 0, path: ["demo", "Counter"], kind: "struct" },
      5: { crate_id: 0, path: ["demo", "Counter", "new"], kind: "assoc_fn" },
    },
  };
  return json;
}

test("--help exits 0 and names the gate", () => {
  const r = spawnSync(process.execPath, [TOOL, "--help"], { encoding: "utf8" });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /OPT23 API-break gate/);
  assert.match(r.stdout, /--tag/);
});

test("an unknown argument exits 2 and points at --help", () => {
  const r = spawnSync(process.execPath, [TOOL, "--no-such-flag"], { encoding: "utf8" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /unknown argument --no-such-flag/);
});

test("--write with --crate is refused: a narrowed write would drop the other crates' contract", () => {
  const r = spawnSync(process.execPath, [TOOL, "--write", "--crate", "core"], { encoding: "utf8" });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /--write records the whole tree/);
});

test("surface keeps the impl method and public field, and ignores docs and spans", () => {
  const json = demoJson();
  const before = surfaceOf(json, "demo");
  assert.deepEqual(Object.keys(before.entries), ["Counter", "Counter::new", "add"]);
  const docOnly = demoJson();
  docOnly.index[1].docs = "Adds both numbers, wrapping on overflow.";
  docOnly.index["2"].docs = "A counter of steps.";
  docOnly.index[5].span = { filename: "other.rs", begin: [1, 1], end: [1, 2] };
  const after = surfaceOf(docOnly, "demo");
  assert.deepEqual(diffSurface(before.entries, after.entries), { breaks: [], added: [] });
});

test("a removed item and a changed signature are breaks; a new item is not", () => {
  const before = surfaceOf(demoJson(), "demo").entries;
  const cut = demoJson();
  cut.index[0].inner.module.items = [2];
  const cutDiff = diffSurface(before, surfaceOf(cut, "demo").entries);
  assert.deepEqual(cutDiff.breaks, [{ key: "add", kind: "removed" }]);

  const reSig = demoJson();
  reSig.index[1].inner.function.sig.inputs.push(["c", { primitive: "u32" }]);
  const reSigDiff = diffSurface(before, surfaceOf(reSig, "demo").entries);
  assert.deepEqual(reSigDiff.breaks, [{ key: "add", kind: "changed" }]);

  const added = demoJson();
  added.index[0].inner.module.items.push(6);
  added.index[6] = { id: 6, name: "extra", visibility: "public", docs: "", attrs: [], inner: { constant: { type: { primitive: "u8" }, const: { expr: "1", value: "1u8", is_literal: true } } } };
  const addedDiff = diffSurface(before, surfaceOf(added, "demo").entries);
  assert.deepEqual(addedDiff.breaks, []);
  assert.deepEqual(addedDiff.added, ["extra"]);
});

test("a rustdoc format move is refused, not read as a break", () => {
  const json = demoJson();
  json.format_version = 58;
  assert.throws(() => surfaceOf(json, "demo"), /rustdoc JSON format 58/);
});

test("a trait method losing its default body is a break", () => {
  const traitJson = (hasBody) => ({
    format_version: 57,
    root: 0,
    index: {
      0: { id: 0, name: "crate", visibility: "public", docs: "", attrs: [], inner: { module: { is_crate: true, items: [1], is_stripped: false } } },
      1: { id: 1, name: "Greet", visibility: "public", docs: "", attrs: [], inner: { trait: { is_unsafe: false, generics: { params: [], where_predicates: [] }, bounds: [], items: [2] } } },
      2: {
        id: 2,
        name: "greet",
        visibility: "default",
        docs: "",
        attrs: [],
        inner: { function: { sig: { inputs: [], output: null, is_c_variadic: false }, generics: { params: [], where_predicates: [] }, header: { is_const: false, is_unsafe: false, is_async: false, abi: "Rust" }, has_body: hasBody } },
      },
    },
    paths: { 0: { crate_id: 0, path: ["demo"], kind: "module" }, 1: { crate_id: 0, path: ["demo", "Greet"], kind: "trait" } },
  });
  const before = surfaceOf(traitJson(true), "demo").entries;
  assert.deepEqual(diffSurface(before, surfaceOf(traitJson(true), "demo").entries), { breaks: [], added: [] });
  assert.deepEqual(diffSurface(before, surfaceOf(traitJson(false), "demo").entries).breaks, [{ key: "Greet", kind: "changed" }]);
});

// The row's Accept, on real rustdoc output: a seeded breaking change fails and
// a doc-only change passes. Skipped only where cargo itself is missing.
test("fixtures: v1 vs v2-doc passes, v1 vs v2-break fails", { skip: !hasCargo && "no cargo on this machine" }, () => {
  const render = (name) => {
    const base = join(FIXTURES, name);
    const targetDir = mkdtempSync(join(tmpdir(), `api-break-${name}-`));
    const got = rustdocSurface({ crate: "api-break-fixture", lib: "api_break_fixture", cwd: base, manifest: join(base, "Cargo.toml"), targetDir });
    assert.equal(got.ok, true, got.reason);
    return got.surface;
  };
  const v1 = render("v1");
  assert.deepEqual(Object.keys(v1.entries), ["Counter", "Counter::get", "Counter::new", "Counter::step", "Fault", "add"]);
  assert.deepEqual(diffSurface(v1.entries, v1.entries), { breaks: [], added: [] });
  const doc = diffSurface(v1.entries, render("v2-doc").entries);
  assert.deepEqual(doc, { breaks: [], added: [] }, "a doc-only change must pass");
  const breaks = diffSurface(v1.entries, render("v2-break").entries).breaks;
  assert.deepEqual(breaks, [
    { key: "Counter::step", kind: "changed" },
    { key: "Fault", kind: "changed" },
    { key: "add", kind: "removed" },
  ]);
});
