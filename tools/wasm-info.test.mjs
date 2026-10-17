// wasm-info drills: a hand-assembled module with known sections is read back exactly.
//   node --test tools/wasm-info.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { readWasmInfo } from "./wasm-info.mjs";

const leb = (n) => { const out = []; do { let b = n & 0x7f; n >>>= 7; if (n) b |= 0x80; out.push(b); } while (n); return out; };
const str = (s) => [...leb(s.length), ...Buffer.from(s)];
const vec = (items) => [...leb(items.length), ...items.flat()];
const section = (id, body) => [id, ...leb(body.length), ...body];
const MAGIC = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
const module = (...sections) => Buffer.from([...MAGIC, ...sections.flat()]);

test("tables, memories, imports, exports and features are read from their sections", () => {
  const wasm = module(
    section(2, vec([[...str("env"), ...str("f"), 0, ...leb(0)], [...str("env"), ...str("mem"), 2, 1, ...leb(17), ...leb(65536)]])),
    section(4, vec([[0x6f, 0, ...leb(128)], [0x70, 1, ...leb(3), ...leb(3)]])),
    section(5, vec([[0, ...leb(258)]])),
    section(7, vec([[...str("__wbindgen_externrefs"), 1, 0], [...str("memory"), 2, 0], [...str("run"), 0, 0]])),
    section(0, [...str("target_features"), ...vec([[0x2b, ...str("bulk-memory")], [0x2b, ...str("sign-ext")]])]),
  );
  const info = readWasmInfo(wasm);
  assert.deepEqual(info.imports, { func: 1, table: 0, memory: 1, global: 0, tag: 0 });
  assert.deepEqual(info.tables, [{ imported: false, type: "externref", min: 128, max: null, shared: false, memory64: false }, { imported: false, type: "funcref", min: 3, max: 3, shared: false, memory64: false }]);
  assert.equal(info.memories.length, 2);
  assert.deepEqual(info.memories[0], { imported: true, min: 17, max: 65536, shared: false, memory64: false });
  assert.deepEqual(info.memories[1], { imported: false, min: 258, max: null, shared: false, memory64: false });
  assert.deepEqual(info.exports, { tables: ["__wbindgen_externrefs"], memories: ["memory"] });
  assert.deepEqual(info.features, ["+bulk-memory", "+sign-ext"]);
});

test("an exported table with max equal to min is visible as one that cannot grow", () => {
  const info = readWasmInfo(module(section(4, vec([[0x6f, 1, ...leb(10), ...leb(10)]])), section(7, vec([[...str("t"), 1, 0]]))));
  const [t] = info.tables;
  assert.equal(t.max, t.min);
  assert.deepEqual(info.exports.tables, ["t"]);
});

test("not a module and a cut-off section are refused", () => {
  assert.throws(() => readWasmInfo(Buffer.from("hello world, not wasm")), /not a wasm module/);
  assert.throws(() => readWasmInfo(module([4, 5, 1, 0x6f, 0])), /ends early/);
});
