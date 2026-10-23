#!/usr/bin/env node
// sbom: write a CycloneDX 1.5 software bill of materials from cargo metadata (TG5).
// One component per package in the build graph, sorted by purl, each with its
// license and a `pkg:cargo/<name>@<version>` purl. No npm dependency: node's
// own `cargo metadata` read plus node:crypto for the serial number.
// The output holds no timestamp, so two runs over one lock are byte-equal.
//   node tools/sbom.mjs [--metadata <cargo-metadata.json>] [--out <file.json>]
//   node tools/sbom.mjs --help
// Exit: 0 the SBOM is written, 1 cargo metadata failed, 2 bad arguments.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const OUT_REL = "target/reports/sbom.cdx.json";
export const SPEC_VERSION = "1.5";

const HELP = `usage: node tools/sbom.mjs [--metadata <cargo-metadata.json>] [--out <file.json>]

  no flag    read \`cargo metadata --format-version 1\` and write ${OUT_REL}
  --metadata read a saved metadata JSON instead of running cargo
  --out      write here instead of ${OUT_REL}
  --help     print this text, exit 0`;

/** A single SPDX id is an id; an `A OR B`/`A AND B`/`A WITH B` string is an expression. */
export function licenseEntry(license) {
  const text = String(license ?? "").trim();
  if (!text) return { license: { name: "unknown" } };
  if (/\s(?:OR|AND|WITH)\s/.test(text)) return { expression: text };
  return { license: { id: text } };
}

/** The component list: one row per package, sorted by purl for a stable file. */
export function components(metadata) {
  const rows = [];
  for (const pkg of metadata?.packages ?? []) {
    const name = String(pkg?.name ?? "");
    const version = String(pkg?.version ?? "");
    if (!name || !version) continue;
    rows.push({
      type: "library",
      name,
      version,
      purl: `pkg:cargo/${name}@${version}`,
      licenses: [licenseEntry(pkg.license)],
    });
  }
  return rows.sort((a, b) => (a.purl < b.purl ? -1 : a.purl > b.purl ? 1 : 0));
}

/** A content-derived urn:uuid, so the same lock gives the same serial number. */
export function serialNumber(rows) {
  const hex = createHash("sha256").update(JSON.stringify(rows)).digest("hex");
  const uuid = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-8${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
  return `urn:uuid:${uuid}`;
}

export function build(metadata) {
  const rows = components(metadata);
  return {
    bomFormat: "CycloneDX",
    specVersion: SPEC_VERSION,
    serialNumber: serialNumber(rows),
    version: 1,
    metadata: {
      tools: [{ vendor: "engine2040", name: "tools/sbom.mjs", version: "1" }],
    },
    components: rows,
  };
}

const atRoot = (p) => (isAbsolute(p) ? p : join(ROOT, p));

export function main(argv = process.argv.slice(2), say = console.log) {
  if (argv.includes("--help") || argv.includes("-h")) {
    say(HELP);
    return 0;
  }
  let metadataPath = null;
  let out = OUT_REL;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if ((a === "--metadata" || a === "--out") && i + 1 < argv.length) {
      if (a === "--metadata") metadataPath = argv[++i];
      else out = argv[++i];
      continue;
    }
    say(`unknown argument: ${a}`);
    say(HELP);
    return 2;
  }
  const outPath = atRoot(out);
  let metadata;
  if (metadataPath) {
    try {
      metadata = JSON.parse(readFileSync(atRoot(metadataPath), "utf8"));
    } catch (error) {
      say(`sbom: cannot read ${metadataPath}: ${error.message}`);
      return 2;
    }
  } else {
    const r = spawnSync("cargo", ["metadata", "--format-version", "1"], { cwd: ROOT, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
    if (r.status !== 0) {
      say(`sbom: cargo metadata failed: ${String(r.stderr ?? "").trim().split("\n")[0]}`);
      return 1;
    }
    metadata = JSON.parse(r.stdout);
  }
  const doc = build(metadata);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, `${JSON.stringify(doc, null, 2)}\n`);
  say(`sbom: ${doc.components.length} components -> ${outPath}`);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
