import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { KEY_NAMES, describeSpawn, ensureKeys, keyFile, sealKeyEnv } from "./seal-keys.mjs";

const fresh = () => keyFile(mkdtempSync(join(tmpdir(), "sealkeys-")));

const captureWarn = () => {
  const lines = [];
  const orig = console.warn;
  console.warn = (...a) => lines.push(a.join(" "));
  return { lines, restore: () => { console.warn = orig; } };
};

test("ensureKeys makes two 64-hex keys and keeps them on the second call", () => {
  const file = fresh();
  const a = ensureKeys(file);
  assert.deepEqual(Object.keys(a), KEY_NAMES);
  for (const name of KEY_NAMES) assert.match(a[name], /^[0-9a-f]{64}$/);
  assert.notEqual(a[KEY_NAMES[0]], a[KEY_NAMES[1]]);
  assert.deepEqual(ensureKeys(file), a);
});

test("a malformed key is replaced, a good one is kept", () => {
  const file = fresh();
  const a = ensureKeys(file);
  const broken = { ...JSON.parse(readFileSync(file, "utf8")), [KEY_NAMES[0]]: "short" };
  writeFileSync(file, JSON.stringify(broken));
  const b = ensureKeys(file);
  assert.match(b[KEY_NAMES[0]], /^[0-9a-f]{64}$/);
  assert.equal(b[KEY_NAMES[1]], a[KEY_NAMES[1]]);
});

test("sealKeyEnv lets an env value win", () => {
  const file = fresh();
  const env = sealKeyEnv(file, { [KEY_NAMES[0]]: "f".repeat(64) });
  assert.equal(env[KEY_NAMES[0]], "f".repeat(64));
  assert.match(env[KEY_NAMES[1]], /^[0-9a-f]{64}$/);
});

test("a corrupt key file warns loudly, then regenerates", () => {
  const file = fresh();
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, "{ not json");
  const cap = captureWarn();
  try {
    const keys = ensureKeys(file);
    for (const name of KEY_NAMES) assert.match(keys[name], /^[0-9a-f]{64}$/);
    assert.equal(cap.lines.length, 1, "one loud warning, not silence");
    assert.match(cap.lines[0], /WARN seal-keys.*unreadable/);
  } finally {
    cap.restore();
  }
});

test("a non-object key file warns loudly, then regenerates", () => {
  const file = fresh();
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, '"just a string"');
  const cap = captureWarn();
  try {
    const keys = ensureKeys(file);
    for (const name of KEY_NAMES) assert.match(keys[name], /^[0-9a-f]{64}$/);
    assert.equal(cap.lines.length, 1, "one loud warning, not silence");
    assert.match(cap.lines[0], /WARN seal-keys.*non-object/);
  } finally {
    cap.restore();
  }
});

test("describeSpawn reports spawn errors and signals, passes status through", () => {
  assert.deepEqual(describeSpawn({ status: 0 }), { code: 0, problem: null });
  assert.deepEqual(describeSpawn({ status: 3 }), { code: 3, problem: null });
  const err = describeSpawn({ error: new Error("boom"), status: null });
  assert.equal(err.code, 1);
  assert.match(err.problem, /boom/);
  const sig = describeSpawn({ signal: "SIGTERM", status: null });
  assert.equal(sig.code, 1);
  assert.match(sig.problem, /SIGTERM/);
});
