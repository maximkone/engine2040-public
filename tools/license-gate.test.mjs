import assert from "node:assert/strict";
import test from "node:test";
import { CHECKS, licenseTokens, parseDenyLicenses, unlicensed, unusedAllow, unusedExceptions, verdict } from "./license-gate.mjs";
import { checkLicenseFiles, collectSources, headerFinding, headText, hookScript } from "./license-gate.mjs";

test("a package with a licence, an inherited licence or a licence file passes", () => {
  const meta = { packages: [{ name: "a", license: "MIT" }, { name: "b", license: "MIT OR Apache-2.0" }, { name: "c", license: null, license_file: "LICENSE" }] };
  assert.deepEqual(unlicensed(meta), []);
  assert.equal(verdict(meta).ok, true);
  assert.equal(verdict(meta).checks, 2);
  assert.match(verdict(meta).line, /3 workspace packages, unlicensed 0, unused-allow 0 \(warn\) \[2 checks\]$/);
});

test("a package with neither fails and is named", () => {
  const meta = { packages: [{ name: "ok", license: "MIT" }, { name: "zed", license: null, license_file: null }, { name: "alpha" }] };
  assert.deepEqual(unlicensed(meta), ["alpha", "zed"]);
  const v = verdict(meta);
  assert.equal(v.ok, false);
  assert.equal(v.checks, 2);
  assert.match(v.line, /unlicensed 2: alpha, zed/);
  assert.match(v.line, /\[2 checks\]$/);
});

test("empty or missing metadata is not a crash", () => {
  assert.equal(verdict({}).ok, true);
  assert.equal(verdict(undefined).line, "license-gate: 0 workspace packages, unlicensed 0, unused-allow 0 (warn) [2 checks]");
});

test("unused-allow hit tracking marks used entries, warns the rest", () => {
  const allowed = ["MIT", "Apache-2.0", "Zlib"];
  const pkgs = [{ name: "a", license: "MIT" }, { name: "b", license: "MIT OR Apache-2.0" }];
  assert.deepEqual(unusedAllow(allowed, pkgs), ["Zlib"]);
  assert.deepEqual(unusedAllow(allowed, []), allowed);
  assert.deepEqual(licenseTokens("MIT OR Apache-2.0").sort(), ["Apache-2.0", "MIT"]);
});

test("unused exceptions hit only the matching crate", () => {
  const exc = [{ crate: "smartstring", allow: ["MPL-2.0"] }, { crate: "clipper2-rust", allow: ["BSL-1.0"] }];
  const pkgs = [{ name: "smartstring", license: "MPL-2.0" }];
  assert.deepEqual(unusedExceptions(exc, pkgs).map((e) => e.crate), ["clipper2-rust"]);
});

test("deny.toml allow plus exceptions parse", () => {
  const text = '[licenses]\nallow = [\n  "MIT",\n  "Apache-2.0",\n]\nexceptions = [\n  { allow = ["MPL-2.0"], crate = "smartstring" },\n]\n';
  assert.deepEqual(parseDenyLicenses(text), { allow: ["MIT", "Apache-2.0"], exceptions: [{ crate: "smartstring", allow: ["MPL-2.0"] }] });
});

test("verdict runs 2 checks: unused-allow warns without failing", () => {
  const meta = { packages: [{ name: "a", license: "MIT" }] };
  const deny = { allow: ["MIT", "Apache-2.0"], exceptions: [{ crate: "smartstring", allow: ["MPL-2.0"] }] };
  const v = verdict(meta, deny);
  assert.equal(CHECKS, 2);
  assert.equal(v.checks, 2);
  assert.equal(v.ok, true);
  assert.deepEqual(v.unused, ["Apache-2.0"]);
  assert.equal(v.unusedExceptions.length, 1);
  assert.match(v.line, /unused-allow 2 \(warn\): Apache-2\.0, smartstring:MPL-2\.0/);
  assert.match(v.line, /\[2 checks\]$/);
});

test("a GPL header is flagged (a WARN since owner 2026-10-10), a clean header passes", () => {
  const bad = headerFinding("crates/foo/src/lib.rs", "// Licensed under GPL-3.0\nfn main() {}");
  assert.match(bad, /GPL/);
  assert.match(bad, /ported code: allowed/);
  assert.equal(headerFinding("crates/foo/src/lib.rs", "// MIT licence\nfn main() {}"), null);
  assert.equal(headText("a\nb").split("\n").length, 2);
  assert.equal(headerFinding("tools/license-gate.mjs", "// donor note: scanned for GPL/AGPL/LGPL, idea-only, never copied\n// MIT licence"), null);
});

test("a missing repo dir reports both licence files", () => {
  assert.deepEqual(checkLicenseFiles("no-such-dir-licgate"), ["LICENSE-MIT", "LICENSE-APACHE"]);
});

test("the hook surfaces deny early and stays opt-in", () => {
  const script = hookScript();
  assert.match(script, /cargo deny check licenses/);
  assert.match(script, /license-gate/);
  assert.match(script, /git lfs pre-push/);
});

test("source collection finds crates plus tools", () => {
  const files = collectSources();
  assert.ok(files.length > 0);
  for (const f of files) assert.match(f, /\.(rs|mjs)$/);
});
