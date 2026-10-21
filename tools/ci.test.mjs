// Run: node --test tools/ci.test.mjs
// The public mirror is the build farm (Maxim 2026-10-16): heavy checks run in parallel on GitHub's free
// public runners; the private dev repo never runs this workflow (its free minutes are tiny).
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
// A Windows checkout with core.autocrlf=true lands this file as CRLF; the job
// regexes below match on "\n  name:\n", so normalize before parsing.
const text = readFileSync(join(ROOT, ".github", "workflows", "ci.yml"), "utf8").replace(/\r\n/g, "\n");

function jobNames() {
  const body = text.slice(text.indexOf("\njobs:"));
  return [...body.matchAll(/\n  ([a-z][a-z0-9-]*):\n/g)].map((m) => m[1]);
}

function jobBlock(name) {
  const start = text.indexOf(`\n  ${name}:\n`);
  assert.notEqual(start, -1, `job ${name} exists`);
  const rest = text.slice(start + 1);
  const next = rest.slice(name.length + 3).search(/\n  [a-z][a-z0-9-]*:\n/);
  return next === -1 ? rest : rest.slice(0, name.length + 3 + next);
}

test("every job runs only on the public mirror", () => {
  const names = jobNames();
  assert.deepEqual(names, ["quick", "test", "clippy", "release-proof", "window-compile", "arm-golden"]);
  for (const n of names) assert.match(jobBlock(n), /if: github\.repository == 'maximkone\/engine2040-public'/, `${n} is guarded`);
});

test("test groups cover the whole workspace exactly once", () => {
  const pkgs = ["engine2040"];
  for (const d of readdirSync(join(ROOT, "crates"))) {
    const toml = join(ROOT, "crates", d, "Cargo.toml");
    const m = existsSync(toml) ? readFileSync(toml, "utf8").match(/^name\s*=\s*"([^"]+)"/m) : null;
    if (m) pkgs.push(m[1]);
  }
  const args = [...text.matchAll(/args: "([^"]+)"/g)].map((m) => m[1]);
  assert.equal(args.length, 4, "four test groups");
  const named = args.slice(0, 3).flatMap((a) => [...a.matchAll(/-p (\S+)/g)].map((m) => m[1]));
  for (const p of named) assert.ok(pkgs.includes(p), `${p} is a workspace package`);
  assert.equal(new Set(named).size, named.length, "no package in two groups");
  const rest = args[3];
  assert.match(rest, /^--workspace /);
  const excluded = [...rest.matchAll(/--exclude (\S+)/g)].map((m) => m[1]);
  assert.deepEqual([...excluded].sort(), [...named].sort(), "rest excludes exactly the named groups");
});

test("fmt, ratchet, clippy, release proofs and the golden hash stay gated", () => {
  assert.match(jobBlock("quick"), /cargo fmt --all -- --check/);
  assert.match(jobBlock("quick"), /node tools\/rust-quality\.mjs/);
  assert.match(jobBlock("quick"), /cargo deny check/);
  assert.match(jobBlock("clippy"), /cargo clippy --locked --workspace --all-targets -- -D warnings/);
  const rp = jobBlock("release-proof");
  // The shipped shape is the `ship` profile (fat LTO, one codegen unit);
  // `release` is thin LTO for the local loop.
  assert.match(rp, /cargo build --locked --workspace --profile ship/);
  assert.match(rp, /node tools\/proof\.mjs --skip-build/);
  assert.match(rp, /d57b5c8ae3096614/);
  assert.match(jobBlock("arm-golden"), /d57b5c8ae3096614/);
  const rootToml = readFileSync(join(ROOT, "Cargo.toml"), "utf8");
  const ship = rootToml.split(/\r?\n(?=\[)/).find((s) => s.startsWith("[profile.ship]")) ?? "";
  assert.match(ship, /lto = "fat"/, "the ship profile keeps fat LTO");
  assert.match(ship, /codegen-units = 1/, "the ship profile keeps one codegen unit");
});

test("windows smoke (window check) and ARM golden stay", () => {
  const wc = jobBlock("window-compile");
  assert.match(wc, /windows-latest/);
  assert.doesNotMatch(wc, /ubuntu-latest/);
  assert.match(wc, /cargo check --locked --workspace --all-targets --features window/);
  assert.match(jobBlock("arm-golden"), /ubuntu-24\.04-arm/);
});

test("the CI publishes a structured test report (the CTRF shape)", () => {
  const t = jobBlock("test");
  // The receipt: the `ci` nextest profile writes it next to the run.
  assert.match(t, /cargo nextest run --locked --all-targets --profile ci/);
  // The run's own words go to the log. Nextest prints its summary on stderr,
  // so the log must take stderr too, or the log holds nothing to check.
  assert.match(t, /2>&1 \| tee target\/nextest-junit\/ci\.log/);
  // The report: the tool finds the receipt by itself (nextest 0.9 writes it
  // under its store dir, so a hardcoded path breaks) and checks the receipt
  // against the run's own words.
  assert.match(t, /node tools\/ctrf-report\.mjs --log target\/nextest-junit\/ci\.log/);
  assert.doesNotMatch(t, /ctrf-report\.mjs --in /, "the receipt path is not hardcoded");
  // The report is published: as an artifact and in the job page.
  assert.match(t, /actions\/upload-artifact@v5/);
  assert.match(t, /ctrf-io\/github-test-reporter@v1/);
  const cfg = readFileSync(join(ROOT, ".config", "nextest.toml"), "utf8").replace(/\r\n/g, "\n");
  assert.match(cfg, /\[profile\.ci\.junit\]/);
  assert.match(cfg, /path = "target\/nextest-junit\/ci\.xml"/);
  // Skipped tests are reported, so the receipt counts every test the run printed.
  assert.match(cfg, /report-skipped = "all"/);
});
