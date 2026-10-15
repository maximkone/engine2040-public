import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { KEYS, gateObject, gateReceipt, writeReceipt } from "./loop-receipt.mjs";

function freshDir() {
  return mkdtempSync(join(tmpdir(), "loop-receipt-"));
}

test("writer creates JSON with the 4 keys plus stamp", () => {
  const dir = freshDir();
  const { path, receipt } = writeReceipt({ packet: "demo", dir });
  const onDisk = JSON.parse(readFileSync(path, "utf8"));
  for (const key of [...KEYS, "stamp"]) {
    assert.ok(onDisk[key] !== undefined, `receipt has ${key}`);
  }
  assert.equal(typeof onDisk.stamp, "string");
  assert.deepEqual(Object.keys(receipt).sort(), [...KEYS, "stamp"].sort());
});

test("gate PASSes a complete receipt and FAILs naming each missing key", () => {
  const dir = freshDir();
  writeReceipt({ packet: "full", dir });
  assert.equal(gateReceipt("full", dir).ok, true);
  assert.equal(gateReceipt("ghost", dir).missing, "receipt");
  const base = { spend: 1, policy: 1, verify: 1, rollback: 1, stamp: "s" };
  for (const key of KEYS) {
    const { [key]: _dropped, ...rest } = base;
    const res = gateObject(rest);
    assert.equal(res.ok, false);
    assert.equal(res.missing, key);
  }
});

test("rollback true records the abandon patch path string", () => {
  const dir = freshDir();
  const { receipt } = writeReceipt({ packet: "bad", rollback: true, dir });
  assert.equal(receipt.rollback.needed, true);
  assert.match(receipt.rollback.patch, /target\/abandoned\/bad.*\.patch/);
  assert.equal(gateReceipt("bad", dir).ok, true);
});
