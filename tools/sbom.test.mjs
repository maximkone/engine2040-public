// Run: node --test tools/sbom.test.mjs
// TG5: the CycloneDX 1.5 build from one saved `cargo metadata` JSON. No npm
// dependency, no network: the fixture is a small metadata document.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { build, components, licenseEntry, main, serialNumber } from "./sbom.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const metadata = {
  packages: [
    { name: "serde", version: "1.0.210", license: "MIT OR Apache-2.0" },
    { name: "engine-core", version: "0.1.0", license: "Apache-2.0" },
    { name: "vendored", version: "0.3.1", license: null },
  ],
};

function tree() {
  const dir = mkdtempSync(join(tmpdir(), "sbom-"));
  const file = join(dir, "metadata.json");
  writeFileSync(file, JSON.stringify(metadata));
  return dir;
}

test("a single SPDX id is an id, an expression stays an expression", () => {
  assert.deepEqual(licenseEntry("Apache-2.0"), { license: { id: "Apache-2.0" } });
  assert.deepEqual(licenseEntry("MIT OR Apache-2.0"), { expression: "MIT OR Apache-2.0" });
  assert.deepEqual(licenseEntry(null), { license: { name: "unknown" } });
});

test("one component per package with a cargo purl, sorted by purl", () => {
  const rows = components(metadata);
  assert.deepEqual(rows.map((r) => r.purl), [
    "pkg:cargo/engine-core@0.1.0",
    "pkg:cargo/serde@1.0.210",
    "pkg:cargo/vendored@0.3.1",
  ]);
  assert.equal(rows[0].type, "library");
  assert.equal(rows[0].version, "0.1.0");
  assert.deepEqual(rows[1].licenses, [{ expression: "MIT OR Apache-2.0" }]);
});

test("the document is CycloneDX 1.5 and its serial number is content-derived", () => {
  const doc = build(metadata);
  assert.equal(doc.bomFormat, "CycloneDX");
  assert.equal(doc.specVersion, "1.5");
  assert.equal(doc.components.length, 3);
  assert.match(doc.serialNumber, /^urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-8[0-9a-f]{3}-[0-9a-f]{12}$/);
  assert.equal(serialNumber(components(metadata)), doc.serialNumber);
  const reordered = build({ packages: [...metadata.packages].reverse() });
  assert.equal(reordered.serialNumber, doc.serialNumber, "package order does not move the serial");
});

test("main writes the file twice byte-equal (no timestamp in the output)", () => {
  const dir = tree();
  try {
    const out = join(dir, "sbom.cdx.json");
    const lines = [];
    assert.equal(main(["--metadata", join(dir, "metadata.json"), "--out", out], (l) => lines.push(l)), 0);
    const first = readFileSync(out, "utf8");
    assert.equal(main(["--metadata", join(dir, "metadata.json"), "--out", out], () => {}), 0);
    assert.equal(readFileSync(out, "utf8"), first);
    const doc = JSON.parse(first);
    assert.equal(doc.components.length, 3);
    assert.match(lines[0], /sbom: 3 components -> /);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing metadata file is exit 2 and a bad flag too", () => {
  assert.equal(main(["--metadata", join(tmpdir(), "no-such-metadata.json")], () => {}), 2);
  assert.equal(main(["--bogus"], () => {}), 2);
  assert.equal(main(["--help"], () => {}), 0);
});

test("the real lock gives a CycloneDX 1.5 document with the workspace crates", () => {
  // The test reads the repo's own metadata only when it is cheap: `cargo
  // metadata --no-deps` is a lock read, so this never builds anything.
  const raw = execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  const doc = build(JSON.parse(raw));
  assert.equal(doc.specVersion, "1.5");
  const names = doc.components.map((c) => c.name);
  for (const name of ["engine2040", "engine-core", "engine-endpoint", "engine-proof"]) {
    assert.ok(names.includes(name), `${name} is in the SBOM`);
  }
  for (const c of doc.components) assert.equal(c.purl, `pkg:cargo/${c.name}@${c.version}`);
});
