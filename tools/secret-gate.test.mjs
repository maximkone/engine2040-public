import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ENTROPY_THRESHOLD, SECRET_PATTERNS, capEntries, loadAllowList, scanText, shannonEntropy } from "./secret-gate.mjs";

// Fixtures are assembled at run time: this file must stay clean under its own gate.
const fixture = (...parts) => parts.join("");
const FAKE_GH = fixture("ghp", "_", "abcdefghijklmnopqrstuvwxyz0123456789");
const FAKE_GH_LOW = fixture("ghp", "_", "a".repeat(36));
const FAKE_PW_HI = fixture("password", ' = "', "aB3x9KqZ7mQ2vL8pR4sT", '"');
const FAKE_PW_LOW = fixture("password", ' = "', "aaaaaaaa", '"');
const FAKE_AWS_HI = fixture("AKIA", "X7K9P2M4Q8Z1W3E6");
const FAKE_AWS_LOW = fixture("AKIA", "Z".repeat(16));
const FAKE_PEM = fixture("-----BEGIN PRIVATE ", "KEY-----");

test("entropy gate sits at 3.8", () => {
  assert.equal(ENTROPY_THRESHOLD, 3.8);
  const gated = SECRET_PATTERNS.filter((p) => p.entropy === 3.8);
  assert.ok(gated.length >= 1, "at least one pattern carries the 3.8 gate");
});

test("shannon entropy matches the donor shape", () => {
  assert.equal(shannonEntropy(""), 0);
  assert.equal(shannonEntropy("aaaaaaaa"), 0);
  assert.ok(shannonEntropy(FAKE_GH) > 3.8);
  assert.ok(shannonEntropy("aaaaaaaa") <= 3.8);
});

test("low-entropy password placeholder is filtered", () => {
  const found = scanText("f.txt", FAKE_PW_LOW + "\n", SECRET_PATTERNS);
  assert.deepEqual(found, []);
});

test("high-entropy password is kept", () => {
  const found = scanText("f.txt", FAKE_PW_HI + "\n", SECRET_PATTERNS);
  assert.equal(found.length, 1);
  assert.match(found[0], /password-assign/);
});

test("random github token passes, placeholder token is filtered", () => {
  const hi = scanText("f.txt", "token " + FAKE_GH + " here\n", SECRET_PATTERNS);
  assert.equal(hi.length, 1);
  const lo = scanText("f.txt", "token " + FAKE_GH_LOW + " here\n", SECRET_PATTERNS);
  assert.deepEqual(lo, []);
});

test("random AWS key passes, repeated AWS key is filtered", () => {
  const hi = scanText("f.txt", "key " + FAKE_AWS_HI + " here\n", SECRET_PATTERNS);
  assert.equal(hi.length, 1);
  const lo = scanText("f.txt", "key " + FAKE_AWS_LOW + " here\n", SECRET_PATTERNS);
  assert.deepEqual(lo, []);
});

test("private-key header still passes (fixed text, no entropy gate)", () => {
  const found = scanText("f.txt", FAKE_PEM + "\n", SECRET_PATTERNS);
  assert.equal(found.length, 1);
});

test("over-cap staged files fail closed, never a truncated PASS", () => {
  const { entries, error } = capEntries(
    [
      { file: "a.txt", text: "x" },
      { file: "b.txt", text: "y" },
    ],
    1,
  );
  assert.ok(error, "over-cap must return an error");
  assert.match(error, /exceed/);
  assert.deepEqual(entries, []);
});

test("under-cap entries pass through", () => {
  const input = [{ file: "a.txt", text: "x" }];
  const { entries, error } = capEntries(input, 5);
  assert.equal(error, null);
  assert.deepEqual(entries, input);
});

test("unreadable allowlist fails closed, never an empty PASS", () => {
  const dir = mkdtempSync(join(tmpdir(), "secret-gate-allow-"));
  try {
    mkdirSync(join(dir, ".secret-allow"));
    assert.throws(() => loadAllowList(dir), /unreadable/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("missing allowlist stays active (empty set)", () => {
  const dir = mkdtempSync(join(tmpdir(), "secret-gate-noallow-"));
  try {
    assert.deepEqual([...loadAllowList(dir)], []);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("second secret on same line still denies (fail closed)", () => {
  const found = scanText("f.txt", FAKE_PW_LOW + " " + FAKE_PW_HI + "\n", SECRET_PATTERNS);
  assert.equal(found.length, 1);
  assert.match(found[0], /password-assign/);
});
