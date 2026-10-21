// Run: node --test tools/differential.test.mjs
// The ship differential (HX31): the workflow plays the fast tier on every
// change of the code and the deep tier on a schedule. This file holds the
// shape of .github/workflows/differential.yml: the schedule must play the
// deep tier, the push and pull request legs must keep playing the fast tier,
// and the deep feature string must name every feature the deep binaries
// need.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
// A Windows checkout with core.autocrlf=true lands this file as CRLF; the
// job regexes below match on "\n  name:\n", so normalize before parsing.
const text = readFileSync(join(ROOT, ".github", "workflows", "differential.yml"), "utf8").replace(/\r\n/g, "\n");

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

function onBlock() {
  const start = text.indexOf("\non:");
  assert.notEqual(start, -1, "the workflow has an on: block");
  return text.slice(start, text.indexOf("\npermissions:"));
}

// The feature string the fast leg builds with, and the one the deep leg
// builds with. The fast leg names both: the deep one when the human asks
// for the deep tier, the fast one otherwise.
function fastFeatures() {
  const m = /FEATURES: \$\{\{ github\.event\.inputs\.tier == 'deep' && '([^']+)' \|\| '([^']+)' \}\}/.exec(jobBlock("differential"));
  assert.ok(m, "the fast leg names its feature string");
  return { deep: m[1], fast: m[2] };
}

function deepFeatures() {
  const m = /FEATURES: ([^\s$]+)\n/.exec(jobBlock("differential-deep"));
  assert.ok(m, "the deep leg names its feature string");
  return m[1].split(",");
}

// The bins the proof crate declares, with the features each one needs.
function declaredBins() {
  const toml = readFileSync(join(ROOT, "crates", "proof", "Cargo.toml"), "utf8").replace(/\r\n/g, "\n");
  const bins = [];
  let block = null;
  for (const raw of toml.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("[")) {
      if (block) bins.push(block);
      block = line === "[[bin]]" ? { name: "", features: [] } : null;
      continue;
    }
    if (!block) continue;
    const name = /^name\s*=\s*"([^"]+)"/.exec(line);
    if (name) {
      block.name = name[1];
      continue;
    }
    const required = /^required-features\s*=\s*\[([^\]]*)\]/.exec(line);
    if (required) block.features = [...required[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  }
  if (block) bins.push(block);
  return bins;
}

// The features a scenario binary needs. A bin the manifest declares in a
// [[bin]] block names them; a bin the crate finds by itself (src/bin) needs
// none.
function binFeatures(bin) {
  const declared = declaredBins().find((b) => b.name === bin);
  if (!declared) {
    const source = join(ROOT, "crates", "proof", "src", "bin", `${bin}.rs`);
    assert.ok(existsSync(source), `the crate builds the binary ${bin}`);
    return [];
  }
  return declared.features;
}

// The bins the committed scenario list plays, tier by tier.
function scenarioTiers() {
  const list = readFileSync(join(ROOT, "fixtures", "differential", "scenarios.txt"), "utf8").replace(/\r\n/g, "\n");
  const tiers = new Map();
  let tier = null;
  for (const raw of list.split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line.startsWith("tier ")) {
      tier = line.slice("tier ".length).trim();
      tiers.set(tier, []);
      continue;
    }
    if (line.startsWith("scenario ")) {
      const bin = /\bbin=(\S+)/.exec(line);
      if (tier && bin) tiers.get(tier).push(bin[1]);
    }
  }
  return tiers;
}

test("the workflow plays a tier on a weekly schedule", () => {
  const on = onBlock();
  assert.match(on, /\n  schedule:\n/, "the workflow declares a schedule");
  const cron = /\n  schedule:\n\s+- cron: '([^']+)'\n/.exec(on);
  assert.ok(cron, "the schedule names a cron");
  // Five fields, once a week: minute, hour, day of month, month, day of week.
  assert.match(cron[1], /^\d+ \d+ \* \* [0-6]$/, `the cron is weekly: ${cron[1]}`);
});

test("the scheduled leg plays the deep tier", () => {
  assert.deepEqual(jobNames(), ["differential", "differential-deep"]);
  const deep = jobBlock("differential-deep");
  // The leg runs on the schedule and on nothing else.
  assert.match(deep, /github\.event_name == 'schedule'/);
  assert.match(deep, /TIER: deep/);
  assert.match(deep, /runs-on: windows-latest/);
  // It plays the committed scenario list and diffs the two receipts.
  assert.match(deep, /--scenarios fixtures\/differential\/scenarios\.txt/);
  assert.match(deep, /--tier "\$env:TIER"/);
  assert.match(deep, /--reference \.\.\/reference\.log/);
  assert.match(deep, /--candidate \.\.\/candidate\.log/);
  // The deep feature string is reachable from the scheduled run.
  assert.deepEqual(deepFeatures(), ["arena", "spine", "ragdoll"]);
});

test("the push and pull request legs still play the fast tier", () => {
  const on = onBlock();
  assert.match(on, /\n  push:\n/, "the workflow still runs on a push");
  assert.match(on, /\n  pull_request:\n/, "the workflow still runs on a pull request");
  const fast = jobBlock("differential");
  // The fast tier is the default and the deep tier is the named one.
  assert.match(fast, /TIER: \$\{\{ github\.event\.inputs\.tier \|\| 'fast' \}\}/);
  assert.match(fast, /FEATURES: \$\{\{ github\.event\.inputs\.tier == 'deep' && 'arena,spine,ragdoll' \|\| 'arena' \}\}/);
  // The schedule plays the deep tier in the other job, so the fast leg
  // stands down for it.
  assert.match(fast, /github\.event_name != 'schedule'/);
  assert.doesNotMatch(fast, /schedule:/);
});

test("the manual and pull request legs are not skipped by the push guard", () => {
  const guard = /if: >-\n([\s\S]*?)\n    runs-on/.exec(jobBlock("differential"))[1];
  // The push guard (a push with no previous commit) must only read
  // `github.event.before` on a push. On the other events `before` is null,
  // and GitHub compares loosely: null and a string are both cast to a number,
  // so null and the zero string are the same number and a guard that reads
  // `before` everywhere skips the manual and the pull request leg as well.
  assert.match(guard, /github\.event_name != 'push'/, "the guard names the push event");
  const scopeAt = guard.indexOf("github.event_name != 'push'");
  const beforeAt = guard.indexOf("github.event.before");
  assert.ok(scopeAt >= 0 && beforeAt > scopeAt, "the before guard sits inside the push clause");
  // The rule that makes it matter.
  const asNumber = (value) => (value === null ? 0 : Number(value));
  assert.equal(asNumber(null), asNumber("0".repeat(40)), "null and the zero string are the same number");
  assert.notEqual(asNumber(null), asNumber("a".repeat(40)), "a real commit is a different number");
});

test("the deep feature string names every feature the deep binaries need", () => {
  const tiers = scenarioTiers();
  const features = fastFeatures();
  const deep = deepFeatures();
  // The manual deep tier and the scheduled leg build the same features.
  assert.deepEqual(features.deep.split(","), deep, "both deep legs build the same features");
  for (const bin of tiers.get("deep") ?? []) {
    for (const feature of binFeatures(bin)) {
      assert.ok(deep.includes(feature), `the deep leg needs the feature ${feature} for ${bin}`);
    }
  }
  // The deep leg builds every bin, so it carries the fast tier's features
  // too: the arena row needs the arena feature.
  for (const bin of tiers.get("fast") ?? []) {
    for (const feature of binFeatures(bin)) {
      assert.ok(features.fast.split(",").includes(feature), `the fast leg needs the feature ${feature} for ${bin}`);
      assert.ok(deep.includes(feature), `the deep leg needs the feature ${feature} for ${bin}`);
    }
  }
});

test("the scheduled leg names the ref it compares against", () => {
  const deep = jobBlock("differential-deep");
  const refs = [...deep.matchAll(/ref: \$\{\{ ([^}]+) \}\}/g)].map((m) => m[1]);
  assert.ok(refs.length >= 2, "both checkouts name a ref");
  // A scheduled run has no "before" commit and no input, so both sides
  // build the scheduled commit: the leg proves the receipts are stable.
  for (const ref of refs) assert.equal(ref, "github.sha", "the scheduled leg compares the scheduled commit with itself");
});

test("the scheduled leg fails on a scenario that did not exit zero", () => {
  const deep = jobBlock("differential-deep");
  // Two builds of one commit that both crash look alike, so the comparison
  // alone would call a broken deep tier clean.
  assert.match(deep, /- name: every deep scenario exited zero/);
  assert.match(deep, /'exit=0'/);
});

test("the weekly run never cancels a push run on the same ref", () => {
  // The concurrency group carries the event name, so the schedule run and
  // the push run that plays the fast tier live in different groups.
  assert.match(text, /group: differential-\$\{\{ github\.event_name \}\}-\$\{\{ github\.ref \}\}/);
  assert.match(text, /cancel-in-progress: true/);
});
