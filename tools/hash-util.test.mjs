// Run: node --test tools/hash-util.test.mjs
// sha256 is the shared integrity digest for tool manifests: published
// SHA-256 vectors (checked against node:crypto), output shape,
// determinism, flip sensitivity and Buffer/Uint8Array parity. Pure
// function, so no repo file is touched.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { sha256 } from "./hash-util.mjs";

const enc = (s) => new TextEncoder().encode(s);
const oracle = (buf) => createHash("sha256").update(buf).digest("hex");

test("empty input is the SHA-256 empty digest, for both input kinds", () => {
  const empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
  assert.equal(sha256(new Uint8Array(0)), empty);
  assert.equal(sha256(Buffer.alloc(0)), empty);
});

test("matches the published SHA-256 vectors", () => {
  assert.equal(sha256(enc("a")), "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb");
  assert.equal(sha256(enc("abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  assert.equal(sha256(enc("hello")), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
  assert.equal(sha256(new Uint8Array([0])), "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d");
});

test("agrees with node:crypto on varied inputs", () => {
  for (const s of ["", "a", "arena 16-slot canonical", "seed 2040, 600 ticks"]) {
    assert.equal(sha256(enc(s)), oracle(enc(s)));
  }
  assert.equal(sha256(Buffer.from([10, 20, 30])), oracle(Buffer.from([10, 20, 30])));
});

test("output is 64 lowercase hex chars and deterministic", () => {
  assert.match(sha256(enc("arena")), /^[0-9a-f]{64}$/);
  assert.equal(sha256(enc("seed 2040")), sha256(enc("seed 2040")));
  assert.notEqual(sha256(enc("seed 2040")), sha256(enc("seed 2041")));
});

test("Buffer and Uint8Array views of the same bytes agree", () => {
  const raw = [10, 20, 30, 40, 250];
  assert.equal(sha256(Buffer.from(raw)), sha256(new Uint8Array(raw)));
  assert.equal(sha256(Buffer.from("foobar")), sha256(enc("foobar")));
});
