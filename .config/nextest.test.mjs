import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const load = () => readFileSync(join(dir, "nextest.toml"), "utf8");

test("proof profile exists with fail-fast off and slow-timeout", () => {
  const text = load();
  assert.match(text, /^\s*\[profile\.proof\]/m);
  assert.match(text, /fail-fast\s*=\s*false/);
  assert.match(text, /slow-timeout\s*=\s*\{\s*period\s*=\s*"60s"/);
});

test("retries stay 0: single zero count with zero delay", () => {
  const text = load();
  const counts = [...text.matchAll(/^\s*count\s*=\s*(\d+)/gm)].map((m) => m[1]);
  assert.deepEqual(counts, ["0"], `expected one retries count of 0, found [${counts.join(", ")}]`);
  assert.match(text, /delay\s*=\s*"0s"/);
  assert.ok(!/count\s*=\s*[1-9]/m.test(text), "no non-zero retry count may remain");
  assert.ok(!/delay\s*=\s*"(5s|10s)"/.test(text), "old 5s/10s backoff delays must not remain");
});

test("no per-platform retry override hides flakes on Windows", () => {
  const text = load();
  assert.ok(!/cfg\(windows\)/.test(text), "Windows retry override must be gone");
  assert.ok(!/\[\[profile\.proof\.overrides\]\]/.test(text), "no proof retry overrides may remain");
  assert.ok(!/retries\s*=\s*\{[^}]*count\s*=\s*3[^}]*delay\s*=\s*"10s"[^}]*\}/.test(text), "3 x 10s retry line must be gone");
});

test("quarantine path is documented instead of retries", () => {
  const text = load();
  assert.match(text, /quarantine/i);
  assert.match(text, /flaky-quarantine/);
  assert.match(text, /--perf/);
});

test("junit receipt still wired for the lane log", () => {
  const text = load();
  assert.match(text, /path\s*=\s*"target\/nextest-junit\/proof\.xml"/);
});
