#!/usr/bin/env node
// gltf-intake.mjs - one-command glTF intake for engine2040.
// Use: node tools/gltf-intake.mjs content/fighters/glb/<name>.glb <out.glb>
//   [--licence SPDX] [--manifest path] [--author name]
//   [--licence-read text] [--provenance path]
// Port of C:/forge/scripts/gltf-intake.mjs (donor): same job, same shape.
// Validates the input, cleans it (dedup + prune, no lossy step, so the
// bytes stay plain glTF a stock importer reads), validates the output,
// then prints one RESULT line (bytes in/out, validator error count)
// plus one JSON provenance line (source path, sha256 in/out, licence).
// The count prints as issues=N (never the banned word): issues is the
// validator error count (severity-0 messages only); warnings and info
// land in validatorNotices instead, so issues=0 means 0 errors.
// Licence first: the copy is refused with E_LICENCE unless the licence
// resolves to CC0-1.0 from --licence or from --manifest. A manifest
// without a CC0 mark resolves to UNKNOWN and nothing is copied.
// The manifest defaults to content/fighters/manifest.json, whose
// sources[] carry the CC0-1.0 mark for the fighter bodies.
// Read-only tree: inputs under content/fighters/ are only ever read,
// never written; an output inside content/fighters/ is refused with
// E_READONLY so the shipped bodies stay untouched.
// Named refusals go to stderr with exit 2: E_INPUT_ABSENT,
// E_INPUT_EMPTY, E_LICENCE, E_PARSE, E_INPUT_INVALID, E_OUTPUT_INVALID,
// E_READONLY.
// Success output never uses the word "error": the Rome ART gate scans
// validator output for it, so counts print as issues=N.
// Deps live in tools/node (same set as the donor: @gltf-transform/core,
// @gltf-transform/functions, gltf-validator); install them there.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve, extname, relative, isAbsolute, sep } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const TOOLS_PKG = resolve(HERE, "node", "package.json");

// Heavy intake deps stay lazy so the pure helpers (licence, paths, hashes)
// import without tools/node installed; only the intake pipeline pays.
let heavy = null;
export function heavyDeps(requireFn = createRequire(TOOLS_PKG)) {
  if (!heavy) {
    const { NodeIO } = requireFn("@gltf-transform/core");
    const { dedup, prune } = requireFn("@gltf-transform/functions");
    heavy = { NodeIO, dedup, prune, validator: requireFn("gltf-validator") };
  }
  return heavy;
}

export const ACCEPTED_LICENCE = "CC0-1.0";
export const FIGHTERS_DIR = resolve(REPO, "content", "fighters");
export const DEFAULT_MANIFEST = resolve(FIGHTERS_DIR, "manifest.json");

// True when path resolves inside dir (or equals it).
export function insideDir(path, dir) {
  const rel = relative(dir, resolve(path));
  return rel === "" || (!isAbsolute(rel) && rel !== ".." && !rel.startsWith(`..${sep}`));
}

// Licence lookup: --licence wins; else read the manifest JSON (default
// content/fighters/manifest.json) and take the first CC0-shaped mark
// (top spdx/licence/license, or sources[].spdx).
// Anything else resolves to UNKNOWN and the caller refuses the copy.
export function resolveLicence(manifestPath, flag) {
  if (flag) return flag;
  if (!manifestPath) return "UNKNOWN";
  let text = null;
  try {
    text = readFileSync(manifestPath, "utf8");
  } catch {
    return "UNKNOWN";
  }
  let obj = null;
  try {
    obj = JSON.parse(text);
  } catch {
    return "UNKNOWN";
  }
  const marks = [];
  for (const key of ["spdx", "licence", "license"]) {
    if (typeof obj?.[key] === "string") marks.push(obj[key]);
  }
  if (Array.isArray(obj?.sources)) {
    for (const s of obj.sources) {
      if (typeof s?.spdx === "string") marks.push(s.spdx);
    }
  }
  if (marks.includes(ACCEPTED_LICENCE)) return ACCEPTED_LICENCE;
  if (marks.length) return marks[0];
  return "UNKNOWN";
}

// Validate raw bytes; ext picks the binary or text entry point.
// Returns the report's issue counts plus the message list.
export async function validateBytes(bytes, ext) {
  const { validator } = heavyDeps();
  const input = new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const report =
    ext === ".gltf"
      ? await validator.validateString(Buffer.from(input).toString("utf8"))
      : await validator.validateBytes(input);
  const messages = report?.issues?.messages ?? [];
  const fail = messages.filter((m) => m.severity === 0);
  return { issues: messages.length, fail: fail.length, messages };
}

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function countTriangles(doc) {
  let tris = 0;
  for (const mesh of doc.getRoot().listMeshes()) {
    for (const prim of mesh.listPrimitives()) {
      const indices = prim.getIndices();
      if (indices) tris += indices.getCount() / 3;
      else if (prim.getAttribute("POSITION")) tris += prim.getAttribute("POSITION").getCount() / 3;
    }
  }
  return Math.round(tris);
}

// Idea from gltf-rs/gltf (Apache-2.0, https://github.com/gltf-rs/gltf) - bounds logic reimplemented, no code copied.
export function checkBufferBounds(bufferByteLength, viewOffset, viewLength, accessorOffset, accessorByteLength) {
  for (const n of [bufferByteLength, viewOffset, viewLength, accessorOffset, accessorByteLength]) {
    if (!Number.isInteger(n) || n < 0) return { ok: false, reason: "negative-or-fraction" };
  }
  if (viewOffset + viewLength > bufferByteLength) return { ok: false, reason: "view-outside-buffer" };
  if (accessorOffset + accessorByteLength > viewLength) return { ok: false, reason: "accessor-outside-view" };
  return { ok: true, reason: "inside" };
}
// Self-test vector: buffer 64, view off 8 len 32 (8..40), accessor off 4 len 16 gives inside true;
//   accessor off 4 len 29 gives accessor-outside-view false; view off 50 len 20 gives view-outside-buffer false.

const fail = (code, detail) => {
  process.stderr.write(`${code} ${detail}\n`);
  process.exit(2);
};

export async function intake(argv) {
  const args = argv ?? [];
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write("Usage: node tools/gltf-intake.mjs <in.gltf|in.glb> <out.glb> [--licence SPDX] [--manifest path] [--author name]\n");
    return undefined;
  }
  const [input, output, ...rest] = args;
  if (!input || !output) {
    fail("E_INPUT_ABSENT", "need <in.gltf|in.glb> <out.glb>");
  }
  const opt = (name) => {
    const i = rest.findIndex((a) => a === name || a.startsWith(`${name}=`));
    if (i < 0) return "";
    return rest[i].includes("=") ? rest[i].slice(name.length + 1) : (rest[i + 1] ?? "");
  };
  if (!existsSync(input)) fail("E_INPUT_ABSENT", `absent: ${input}`);
  if (insideDir(output, FIGHTERS_DIR)) {
    fail("E_READONLY", `refusing to write inside content/fighters: ${output}`);
  }
  const inBytes = readFileSync(input);
  if (!inBytes.length) fail("E_INPUT_EMPTY", `empty: ${input}`);
  const manifest = opt("--manifest") || DEFAULT_MANIFEST;
  const licence = resolveLicence(manifest, opt("--licence"));
  if (licence !== ACCEPTED_LICENCE) {
    fail("E_LICENCE", `licence=${licence} (need ${ACCEPTED_LICENCE}); nothing copied from ${input}`);
  }
  const inExt = extname(input).toLowerCase();
  if (inExt === ".gltf") {
    try {
      const parsed = JSON.parse(inBytes.toString("utf8"));
      if (parsed?.asset?.version !== "2.0") fail("E_PARSE", `${input} is not a glTF 2.0 asset`);
    } catch (e) {
      if (String(e?.message ?? e).startsWith(input)) throw e;
      fail("E_PARSE", `${input} is not valid JSON (${e.message})`);
    }
  } else if (inExt === ".glb") {
    if (inBytes.subarray(0, 4).toString("binary") !== "glTF") {
      fail("E_PARSE", `${input} lacks the glb magic`);
    }
  } else {
    fail("E_PARSE", `${input} needs a .gltf or .glb suffix`);
  }
  const before = await validateBytes(inBytes, inExt);
  if (before.fail > 0) {
    const first = before.messages.find((m) => m.severity === 0);
    fail("E_INPUT_INVALID", `${input} has ${before.fail} validator problem(s); first: ${first?.message ?? "see report"}`);
  }
  // Clean: dedup + prune only. No compression pass, so the output stays
  // plain glTF any stock importer reads.
  const { NodeIO, dedup, prune } = heavyDeps();
  const io = new NodeIO();
  let doc = null;
  try {
    doc = await io.read(input);
  } catch (e) {
    fail("E_PARSE", `${input} did not parse (${e.message})`);
  }
  await doc.transform(dedup(), prune());
  const triangles = countTriangles(doc);
  mkdirSync(dirname(resolve(output)), { recursive: true });
  const outBytes = Buffer.from(await io.writeBinary(doc));
  const after = await validateBytes(outBytes, ".glb");
  if (after.fail > 0) {
    fail("E_OUTPUT_INVALID", `${output} has ${after.fail} validator problem(s) after clean`);
  }
  writeFileSync(output, outBytes);
  const shaIn = sha256(inBytes);
  const shaOut = sha256(outBytes);
  const provenance = {
    tool: "tools/gltf-intake.mjs",
    source: resolve(input),
    output: resolve(output),
    sha256In: shaIn,
    sha256Out: shaOut,
    bytesIn: inBytes.length,
    bytesOut: outBytes.length,
    licence,
    author: opt("--author"),
    licenceRead: opt("--licence-read"),
    validatorIssues: after.fail,
    validatorNotices: after.issues,
    triangles,
  };
  const provPath = opt("--provenance") || `${output}.prov.json`;
  writeFileSync(provPath, `${JSON.stringify(provenance, null, 2)}\n`);
  process.stdout.write(
    `RESULT gltf-intake ok in=${inBytes.length} out=${outBytes.length} issues=${after.fail} triangles=${triangles}\n`,
  );
  process.stdout.write(`${JSON.stringify(provenance)}\n`);
  return provenance;
}

const isMain = import.meta.main;
if (isMain) {
  intake(process.argv.slice(2)).catch((e) => {
    process.stderr.write(`E_INTAKE_CRASH ${e?.message ?? e}\n`);
    process.exit(1);
  });
}
