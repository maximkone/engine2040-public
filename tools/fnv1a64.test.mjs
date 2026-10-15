// Run: node --test tools/fnv1a64.test.mjs
// fnv1a64Hex is the shared integrity digest behind ship.mjs pack manifests
// and desync-diff.mjs dump hashes: published FNV-1a 64 vectors (checked
// against an independent Python implementation of the spec), output shape,
// determinism, flip sensitivity and Buffer/Uint8Array parity. Pure function,
// so no repo file is touched.
import assert from "node:assert/strict";
import test from "node:test";
import { fnv1a64Hex } from "./fnv1a64.mjs";

const enc = (s) => new TextEncoder().encode(s);

test("empty input is the FNV offset basis, for both input kinds", () => {
  assert.equal(fnv1a64Hex(new Uint8Array(0)), "cbf29ce484222325");
  assert.equal(fnv1a64Hex(Buffer.alloc(0)), "cbf29ce484222325");
});

test("matches the published FNV-1a 64 vectors", () => {
  assert.equal(fnv1a64Hex(enc("a")), "af63dc4c8601ec8c");
  assert.equal(fnv1a64Hex(enc("foobar")), "85944171f73967e8");
  assert.equal(fnv1a64Hex(new Uint8Array([0])), "af63bd4c8601b7df");
  assert.equal(fnv1a64Hex(enc("hello")), "a430d84680aabd0b");
});

test("output is always 16 lowercase hex chars", () => {
  for (let byte = 0; byte < 256; byte += 1) {
    assert.match(fnv1a64Hex(new Uint8Array([byte])), /^[0-9a-f]{16}$/);
  }
  assert.match(fnv1a64Hex(enc("arena 16-slot canonical")), /^[0-9a-f]{16}$/);
  assert.match(fnv1a64Hex(new Uint8Array(1024).fill(0xff)), /^[0-9a-f]{16}$/);
});

test("same bytes hash identically on repeat runs", () => {
  const bytes = enc("seed 2040, 600 ticks");
  assert.equal(fnv1a64Hex(bytes), fnv1a64Hex(enc("seed 2040, 600 ticks")));
  const blob = new Uint8Array(256).map((_, i) => i);
  assert.equal(fnv1a64Hex(blob), fnv1a64Hex(new Uint8Array(256).map((_, i) => i)));
});

test("a one-bit flip, reorder or length change moves the digest", () => {
  const base = new Uint8Array([1, 2, 3, 4]);
  const flipped = new Uint8Array([1, 2, 3, 5]);
  assert.notEqual(fnv1a64Hex(base), fnv1a64Hex(flipped));
  assert.notEqual(fnv1a64Hex(base), fnv1a64Hex(new Uint8Array([4, 3, 2, 1])));
  assert.notEqual(fnv1a64Hex(base), fnv1a64Hex(new Uint8Array([1, 2, 3])));
  assert.notEqual(fnv1a64Hex(base), fnv1a64Hex(new Uint8Array([1, 2, 3, 4, 0])));
});

test("Buffer and Uint8Array views of the same bytes agree", () => {
  const raw = [10, 20, 30, 40, 250];
  assert.equal(fnv1a64Hex(Buffer.from(raw)), fnv1a64Hex(new Uint8Array(raw)));
  assert.equal(fnv1a64Hex(Buffer.from("foobar")), fnv1a64Hex(enc("foobar")));
});

test("all 256 single-byte digests are distinct and none is the empty hash", () => {
  const seen = new Set();
  for (let byte = 0; byte < 256; byte += 1) {
    seen.add(fnv1a64Hex(new Uint8Array([byte])));
  }
  assert.equal(seen.size, 256);
  assert.ok(!seen.has(fnv1a64Hex(new Uint8Array(0))));
});
