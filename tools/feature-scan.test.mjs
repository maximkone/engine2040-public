// feature-scan: a feature is dead when no other crate asks for it and no cfg
// read in its own crate reaches it, and weak when one production place reads
// it. The scan reads manifests and Rust files only.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { classify, featureRows, isWeak, scanFeatures, weakFeatures } from "./feature-scan.mjs";

// alpha asks beta for `wanted` and reads its own `lonely`; beta declares `wanted`
// (asked for), `lonely` (nobody asks, one src read), `quiet` (nothing at all),
// `gated` (only a bin needs it), `umbrella` (turns on `lonely2`), `lonely2`
// (only `umbrella` turns it on), `tested` (only a test file reads it) and a
// `default` that turns on nothing. A doc command asks for `asked`.
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "feature-scan-"));
  const files = {
    "crates/alpha/Cargo.toml": '[package]\nname = "engine-alpha"\n\n[dependencies]\nengine-beta = { path = "../beta", features = ["wanted"] }\n\n[features]\nlonely = []\n',
    "crates/alpha/src/lib.rs": '#[cfg(feature = "lonely")]\npub mod lonely;\n',
    "crates/beta/Cargo.toml": [
      '[package]',
      'name = "engine-beta"',
      '',
      '[features]',
      'default = []',
      'wanted = []',
      'asked = []',
      'lonely = []',
      'quiet = []',
      'gated = []',
      'umbrella = ["lonely2"]',
      'lonely2 = []',
      'tested = []',
      '',
      '[[bin]]',
      'name = "gate"',
      'required-features = ["gated"]',
      '',
    ].join("\n"),
    "crates/beta/src/lib.rs": '#[cfg(feature = "lonely")]\npub mod lonely;\n',
    "crates/beta/tests/it.rs": '#![cfg(feature = "tested")]\n',
    "crates/beta/src/bin/gate.rs": "fn main() {}\n",
    "docs/note.md": "`cargo test -p engine-beta --features asked`\n`cargo test -p engine-beta --features umbrella`\n",
  };
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), body);
  }
  return root;
}

test("dead, weak and live come from the manifests and the Rust files", () => {
  const root = fixture();
  try {
    const { rows, dead, weak } = scanFeatures(root);
    const at = (key) => rows.find((r) => r.key === key);
    assert.equal(classify(at("engine-beta/quiet")), "dead", "nobody asks, nobody reads");
    assert.equal(classify(at("engine-beta/default")), "dead", "an empty default turns on nothing");
    assert.equal(classify(at("engine-beta/wanted")), "live", "alpha asks for it");
    assert.equal(classify(at("engine-beta/gated")), "live", "the bin's required-features reads it");
    assert.equal(classify(at("engine-beta/lonely")), "weak", "one src read, nobody asks");
    assert.equal(classify(at("engine-alpha/lonely")), "weak", "one src read, nobody asks");
    // A doc command asks but no code reads the flag, and a sibling feature of
    // the same crate is not another crate: neither keeps a flag alive.
    assert.equal(classify(at("engine-beta/asked")), "dead", "a doc command is not a reader");
    assert.equal(classify(at("engine-beta/umbrella")), "dead", "a sibling is not another crate");
    assert.equal(classify(at("engine-beta/lonely2")), "dead", "the umbrella is not another crate");
    // A test file gates on the flag, but the read that keeps a flag alive is
    // one in the crate's own src/, so a test-only flag is still dead.
    assert.equal(classify(at("engine-beta/tested")), "dead", "a tests/ read is not a src read");
    assert.deepEqual(dead.map((r) => r.key).sort(), [
      "engine-beta/asked",
      "engine-beta/default",
      "engine-beta/lonely2",
      "engine-beta/quiet",
      "engine-beta/tested",
      "engine-beta/umbrella",
    ]);
    assert.deepEqual(weak.map((r) => r.key), ["engine-alpha/lonely", "engine-beta/lonely"]);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("a default that names a feature is the crate's default build, so it lives", () => {
  const root = fixture();
  try {
    const beta = join(root, "crates", "beta", "Cargo.toml");
    writeFileSync(beta, readFileSync(beta, "utf8").replace('default = []', 'default = ["wanted"]'));
    const rows = featureRows(root, { commands: false });
    const at = (key) => rows.find((r) => r.key === key);
    assert.equal(classify(at("engine-beta/default")), "live");
    assert.equal(classify(at("engine-beta/wanted")), "live", "the default turns it on");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("the weak list names the one production place that reads the flag", () => {
  const root = fixture();
  try {
    const weak = weakFeatures(root);
    const at = (key) => weak.find((r) => r.key === key);
    assert.equal(weak.length, 2, "gated, tested and lonely2 have no src read");
    assert.equal(at("engine-alpha/lonely").where, "crates/alpha/src/lib.rs");
    assert.equal(at("engine-beta/lonely").where, "crates/beta/src/lib.rs");
    assert.equal(isWeak({ feature: "x", srcGates: 2 }), false, "two readers are not thin");
    assert.equal(isWeak({ feature: "x", srcGates: 1 }), true, "one reader is thin");
    assert.equal(isWeak({ feature: "default", srcGates: 1 }), false, "default is Cargo's own name");
  } finally { rmSync(root, { recursive: true, force: true }); }
});
