#!/usr/bin/env node
// OPT23 API-break gate: the public surface of every engine crate against a
// committed baseline (tools/api-break.baseline.json) or the baseline committed
// at a release tag (`--tag v1.2.3`). A removed item or a changed signature
// fails; docs, spans, attributes and ids never enter a fingerprint, so a
// doc-only change passes.
//
// Engine: rustdoc JSON (format 57) from the stable rustdoc with
// RUSTC_BOOTSTRAP=1; see docs/plan/quality.md FB3 for the policy. Licence read
// first, as the row asks (2026-10-20): cargo-semver-checks 0.51.0 is
// Apache-2.0 OR MIT (crates.io; rustc 1.93 minimum) and is compatible with
// this workspace's Apache-2.0, but it is not installed on the build PC and a
// cargo install is a multi-minute build outside this tree, so this tool keeps
// the same verdict shape from rustdoc JSON. A tagging release also gets
// `--tag` so the comparison is against the last release, not the last commit.
//
//   node tools/api-break.mjs                 check the tree against the baseline
//   node tools/api-break.mjs --write         re-record the baseline (bless a break)
//   node tools/api-break.mjs --crate core    only crates matching core or engine-core
//   node tools/api-break.mjs --tag v0.2.0    baseline as committed at that ref
//   node tools/api-break.mjs --help
//
// Scope: every `engine-*` crate with a lib target (engine-workspace-hack is
// not API), default features only, so an item behind a non-default feature is
// outside this leg. The baseline records the working tree, like the doc ratchet.
//
// Exit: 0 pass - 1 a break (removed or changed public item) - 2 could not run
// (no baseline, a crate that no longer builds, rustdoc format drift).
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { fnv1a64Hex } from "./fnv1a64.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = resolve(HERE, "..");
const BASELINE_NAME = "api-break.baseline.json";
const FORMAT = 57;
const PREFIX = "engine-";
const EXCLUDED = new Set(["engine-workspace-hack"]);
const TARGET_DIR = "target";
const STDOUT_LIMIT = 400;

const help = `api-break.mjs - OPT23 API-break gate (cargo-semver-checks shape, no install)

Usage:
  node tools/api-break.mjs [--write] [--crate <name>]... [--tag <ref>|--baseline <path>] [--root <dir>]

Renders rustdoc JSON per public engine crate (${PREFIX}*, lib target) and compares
its public surface (paths, signatures, fields, variants, trait and impl items)
with a baseline of FNV-1a 64 fingerprints. Removed or changed item = break;
added item = compatible. Docs and spans are not part of a fingerprint.

Options:
  --write          record the current surface as the baseline (after a deliberate break)
  --crate <name>   only this crate (repeatable; accepts core for engine-core)
  --tag <ref>      baseline as committed at <ref> (the last release tag, once tagged)
  --baseline <p>   read the baseline from <p> instead of tools/api-break.baseline.json
  --root <dir>     repo root (default: this checkout)
  --help           print this text and exit 0

Exit codes: 0 pass - 1 break - 2 could not run.`;

function kindOf(item) {
  return Object.keys(item.inner)[0];
}

function labelOf(json, id) {
  const p = json.paths[id];
  if (p) return p.path.slice(1).join("::") || p.kind;
  const it = json.index[id];
  return it ? `${kindOf(it)}:${it.name ?? "?"}` : `id:${id}`;
}

// Normalize a value for a fingerprint: item ids become their public path, so
// the same API in two builds (different numeric ids) hashes the same, and a
// type that moved keeps its name. Object keys are sorted for stability.
export function normalize(json, value) {
  if (typeof value === "number") {
    const item = json.index[String(value)];
    return item ? `@${labelOf(json, String(value))}` : value;
  }
  if (Array.isArray(value)) return value.map((v) => normalize(json, v));
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = normalize(json, value[key]);
    return out;
  }
  return value;
}

function pick(json, source, keys) {
  return JSON.stringify(normalize(json, Object.fromEntries(keys.filter((k) => source[k] !== undefined).map((k) => [k, source[k]]))));
}

function isNonExhaustive(item) {
  return JSON.stringify(item.attrs ?? []).includes("NonExhaustive");
}

// Public fields of a struct, union or enum variant: name plus type, or the
// bare type for a tuple field. Ids that do not resolve stay as their number.
function fieldsOf(json, holder) {
  if (!holder) return [];
  if (Array.isArray(holder)) return holder.map((id) => normalize(json, id));
  const ids = holder.fields ?? [];
  return ids.map((id) => {
    const item = json.index[String(id)];
    if (!item) return normalize(json, id);
    return { name: item.name ?? null, sig: normalize(json, item.inner.struct_field ?? item.inner) };
  });
}

function structKindOf(json, struct) {
  const tag = Object.keys(struct.kind)[0];
  return { tag, fields: fieldsOf(json, struct.kind[tag]) };
}

// The fingerprint of one public item: the shape a caller compiles against.
export function fingerprintOf(json, item) {
  const kind = kindOf(item);
  const inner = item.inner[kind];
  switch (kind) {
    case "function":
      return `fn${pick(json, inner, ["sig", "generics", "header"])}`;
    case "struct":
    case "union": {
      const shape = structKindOf(json, inner);
      return `${kind}${JSON.stringify({ generics: normalize(json, inner.generics), shape, ne: isNonExhaustive(item) })}`;
    }
    case "enum": {
      const variants = (inner.variants ?? []).map((id) => {
        const it = json.index[String(id)];
        if (!it) return normalize(json, id);
        const v = it.inner.variant;
        const vtag = Object.keys(v.kind)[0];
        return { name: it.name, tag: vtag, fields: fieldsOf(json, v.kind[vtag]) };
      });
      return `enum${JSON.stringify({ generics: normalize(json, inner.generics), variants, ne: isNonExhaustive(item) })}`;
    }
    case "trait":
      return `trait${JSON.stringify({ generics: normalize(json, inner.generics), is_unsafe: inner.is_unsafe, bounds: normalize(json, inner.bounds), ne: isNonExhaustive(item), items: (inner.items ?? []).map((id) => assocItem(json, id)) })}`;
    case "constant":
      return `const${pick(json, inner, ["type"])}`;
    case "static":
      return `static${pick(json, inner, ["type", "is_mutable", "is_unsafe"])}`;
    case "type_alias":
      return `type${pick(json, inner, ["type", "generics"])}`;
    case "macro":
      return `macro${JSON.stringify(normalize(json, inner))}`;
    case "module":
      return "module";
    case "use":
      return `use${JSON.stringify(normalize(json, inner))}`;
    default:
      return `${kind}${JSON.stringify(normalize(json, inner))}`;
  }
}

function assocItem(json, id) {
  const item = json.index[String(id)];
  if (!item) return normalize(json, id);
  const kind = kindOf(item);
  // `has_body` on a trait method means a default body: losing it makes the
  // method required, which is a break; it is carried only for trait members.
  const hasDefault = kind === "function" ? item.inner.function.has_body : undefined;
  return { name: item.name, kind, sig: fingerprintOf(json, item), has_default: hasDefault };
}

function implsOf(json, item) {
  const inner = item.inner[kindOf(item)];
  return inner?.impls ?? [];
}

// Public surface of one rustdoc JSON: a Map from item path to fingerprint.
export function surfaceOf(json, crate) {
  if (json.format_version !== FORMAT) {
    throw new Error(`rustdoc JSON format ${json.format_version}, this tool reads ${FORMAT}: refresh the baseline after a toolchain move`);
  }
  const entries = new Map();
  const seenModules = new Set();
  const add = (key, text) => {
    entries.set(key, fnv1a64Hex(Buffer.from(String(text), "utf8")));
  };
  const canonical = (id) => {
    const p = json.paths[String(id)];
    return p ? p.path.slice(1).join("::") : null;
  };
  const walkImpl = (implId, typeKey) => {
    const im = json.index[String(implId)]?.inner.impl;
    if (!im || im.is_synthetic || im.blanket_impl) return;
    if (im.trait === null) {
      for (const itemId of im.items ?? []) {
        const item = json.index[String(itemId)];
        if (!item || item.visibility !== "public") continue;
        add(`${typeKey}::${item.name ?? "?"}`, fingerprintOf(json, item));
      }
      return;
    }
    const traitKey = im.trait.path ?? labelOf(json, String(im.trait.id));
    const items = (im.items ?? []).map((itemId) => {
      const item = json.index[String(itemId)];
      return item ? { name: item.name, kind: kindOf(item), sig: fingerprintOf(json, item) } : normalize(json, itemId);
    });
    add(`${typeKey} as ${traitKey}`, JSON.stringify(items));
  };
  const walkItems = (ids, base, depth) => {
    if (depth > 40) return;
    for (const rawId of ids ?? []) {
      const id = String(rawId);
      const item = json.index[id];
      if (!item || item.visibility !== "public") continue;
      const kind = kindOf(item);
      const key = canonical(id) ?? [...base, item.name ?? "?"].join("::");
      if (kind === "module") {
        add(key, "module");
        if (seenModules.has(id)) continue;
        seenModules.add(id);
        walkItems(item.inner.module.items, [...base, item.name ?? "?"], depth + 1);
      } else if (kind === "use") {
        add(`${key} use`, fingerprintOf(json, item));
        const target = item.inner.use.id === null ? null : json.index[String(item.inner.use.id)];
        if (target && kindOf(target) === "module" && !seenModules.has(String(item.inner.use.id))) {
          seenModules.add(String(item.inner.use.id));
          const prefix = item.inner.use.is_glob ? [...base, item.inner.use.name ?? ""] : [...base, item.name ?? ""].filter(Boolean);
          walkItems(target.inner.module.items, prefix, depth + 1);
        }
      } else {
        add(key, fingerprintOf(json, item));
        for (const implId of implsOf(json, item)) walkImpl(implId, key);
      }
    }
  };
  const root = json.index[String(json.root)];
  // Keys are crate-relative, like `paths[id].path.slice(1)`: the baseline is
  // stored per crate, so the crate name is not part of a key.
  walkItems(root?.inner.module?.items ?? [], [], 0);
  const sorted = {};
  for (const key of [...entries.keys()].sort()) sorted[key] = entries.get(key);
  return { crate, count: entries.size, entries: sorted };
}

export function diffSurface(before, after) {
  const breaks = [];
  for (const [key, hash] of Object.entries(before ?? {})) {
    if (!(key in (after ?? {}))) breaks.push({ key, kind: "removed" });
    else if (after[key] !== hash) breaks.push({ key, kind: "changed" });
  }
  const added = Object.keys(after ?? {}).filter((key) => !(key in (before ?? {})));
  return { breaks, added };
}

// One plain cargo line, as AGENTS.md proof rule: rustdoc JSON for one crate.
export function rustdocSurface({ crate, lib, cwd, manifest = null, targetDir = null }) {
  const args = ["rustdoc"];
  if (manifest) args.push("--manifest-path", manifest);
  else args.push("-p", crate);
  if (targetDir) args.push("--target-dir", targetDir);
  args.push("--lib", "--", "-Z", "unstable-options", "--output-format", "json");
  const r = spawnSync("cargo", args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: 900_000,
    shell: false,
    env: { ...process.env, RUSTC_BOOTSTRAP: "1" },
  });
  if (r.error) return { ok: false, reason: `cargo rustdoc did not start: ${String(r.error.message).slice(0, STDOUT_LIMIT)}` };
  if (r.status !== 0) {
    const tail = String(r.stderr ?? "").trim().split("\n").filter(Boolean).slice(-1)[0] ?? "";
    return { ok: false, reason: `cargo rustdoc exit ${r.status}: ${tail}`.slice(0, STDOUT_LIMIT) };
  }
  const jsonPath = join(targetDir ?? join(cwd, TARGET_DIR), "doc", `${lib}.json`);
  if (!existsSync(jsonPath)) return { ok: false, reason: `no rustdoc JSON at ${jsonPath}` };
  let json;
  try {
    json = JSON.parse(readFileSync(jsonPath, "utf8"));
  } catch (e) {
    return { ok: false, reason: `rustdoc JSON at ${jsonPath} is not JSON: ${String(e.message).slice(0, 120)}` };
  }
  try {
    return { ok: true, surface: surfaceOf(json, crate) };
  } catch (e) {
    return { ok: false, reason: String(e.message).slice(0, STDOUT_LIMIT) };
  }
}

export function publicCrates(root) {
  const r = spawnSync("cargo", ["metadata", "--no-deps", "--locked", "--format-version", "1"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    shell: false,
  });
  if (r.status !== 0) throw new Error(`cargo metadata exit ${r.status}: ${String(r.stderr ?? "").trim().split("\n").slice(-1)[0] ?? ""}`);
  return JSON.parse(r.stdout).packages
    .filter((p) => p.name.startsWith(PREFIX) && !EXCLUDED.has(p.name))
    .map((p) => ({ name: p.name, lib: (p.targets.find((t) => t.kind.includes("lib")) || {}).name ?? null }))
    .filter((p) => p.lib !== null)
    .sort((a, b) => a.name.localeCompare(b.name));
}

function gitShow(root, ref, path) {
  const r = spawnSync("git", ["show", `${ref}:${path}`], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: false });
  return r.status === 0 ? r.stdout : null;
}

function lastReleaseTag(root) {
  const r = spawnSync("git", ["tag", "--list", "v[0-9]*", "--sort=-v:refname"], { cwd: root, encoding: "utf8", shell: false });
  return r.status === 0 ? (r.stdout.trim().split("\n")[0] ?? "") : "";
}

function baselineFrom(root, tag, explicit) {
  const rel = `tools/${BASELINE_NAME}`;
  if (explicit) {
    const path = resolve(root, explicit);
    if (!existsSync(path)) return { error: `no baseline at ${path}` };
    return { baseline: JSON.parse(readFileSync(path, "utf8")), source: explicit };
  }
  if (!tag) {
    const path = join(root, rel);
    if (!existsSync(path)) return { error: `no baseline at ${rel} (run node tools/api-break.mjs --write)` };
    return { baseline: JSON.parse(readFileSync(path, "utf8")), source: rel };
  }
  const text = gitShow(root, tag, rel);
  if (text === null) return { error: `${rel} does not exist at ${tag}: the API contract started after that ref` };
  return { baseline: JSON.parse(text), source: `${tag}:${rel}` };
}

function printUsage(out = console.log) {
  for (const line of help.split("\n")) out(line);
}

function wrt(stream, line) {
  stream(line);
}

export function main(argv, io = { out: console.log, err: console.error, root: DEFAULT_ROOT }) {
  const out = io.out;
  const flags = { write: false, crates: [], tag: null, baseline: null, root: io.root };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") { printUsage(out); return 0; }
    else if (a === "--write") flags.write = true;
    else if (a === "--crate") flags.crates.push(argv[++i] ?? "");
    else if (a === "--tag") flags.tag = argv[++i] ?? "";
    else if (a === "--baseline") flags.baseline = argv[++i] ?? "";
    else if (a === "--root") flags.root = resolve(argv[++i] ?? ".");
    else { wrt(io.err, `api-break: unknown argument ${a} (node tools/api-break.mjs --help)`); return 2; }
  }
  if (flags.crates.some((c) => !c)) { wrt(io.err, "api-break: --crate needs a name"); return 2; }
  if (flags.tag === "") { wrt(io.err, "api-break: --tag needs a ref"); return 2; }
  if (flags.baseline === "") { wrt(io.err, "api-break: --baseline needs a path"); return 2; }

  let crates;
  try {
    crates = publicCrates(flags.root);
  } catch (e) {
    wrt(io.err, `api-break: ${e.message}`);
    return 2;
  }
  const wanted = (name) => !flags.crates.length || flags.crates.some((c) => c === name || `${PREFIX}${c}` === name || c.replace(/^engine-/, "") === name.replace(/^engine-/, ""));
  const chosen = crates.filter((c) => wanted(c.name));
  if (!chosen.length) { wrt(io.err, `api-break: no crate matches ${flags.crates.join(", ")}`); return 2; }

  if (flags.write && flags.tag) { wrt(io.err, "api-break: --write records the working tree; it does not combine with --tag"); return 2; }
  if (flags.write && flags.baseline) { wrt(io.err, "api-break: --write always records tools/api-break.baseline.json; it does not combine with --baseline"); return 2; }
  if (flags.write && flags.crates.length) { wrt(io.err, "api-break: --write records the whole tree; --crate narrows a check only"); return 2; }

  const loaded = flags.write ? null : baselineFrom(flags.root, flags.tag, flags.baseline);
  if (loaded?.error) { wrt(io.err, `api-break: ${loaded.error}`); return 2; }
  if (loaded && loaded.baseline.rustdoc_format !== FORMAT) {
    wrt(io.err, `api-break: ${loaded.source} records rustdoc format ${loaded.baseline.rustdoc_format}, this tool reads ${FORMAT}; refresh the baseline`);
    return 2;
  }
  if (!flags.write && !flags.tag) {
    const tag = lastReleaseTag(flags.root);
    out(tag ? `baseline: ${loaded.source}; last release tag ${tag} (--tag ${tag} compares against it)` : `baseline: ${loaded.source}; no v* release tag exists yet, so there is no release to compare with`);
  }

  const recorded = {};
  const unbuilt = {};
  const breaks = [];
  const added = [];
  let failedRun = false;
  for (const crate of chosen) {
    const got = rustdocSurface({ crate: crate.name, lib: crate.lib, cwd: flags.root });
    if (!got.ok) {
      failedRun = true;
      unbuilt[crate.name] = got.reason;
      wrt(io.err, `[FAIL] ${crate.name}: ${got.reason}`);
      continue;
    }
    const { surface } = got;
    recorded[crate.name] = surface.entries;
    if (flags.write) {
      out(`${crate.name} ${surface.count} items`);
      continue;
    }
    const base = loaded.baseline.crates?.[crate.name];
    if (!base) {
      failedRun = true;
      wrt(io.err, `[FAIL] ${crate.name}: not in ${loaded.source} (new crate: run --write)`);
      continue;
    }
    const diff = diffSurface(base, surface.entries);
    for (const b of diff.breaks) breaks.push(`${crate.name} ${b.key} ${b.kind}`);
    for (const key of diff.added) added.push(`${crate.name} ${key}`);
    out(`${crate.name.padEnd(22)} ${String(surface.count).padStart(5)} items  ${diff.breaks.length ? `FAIL ${diff.breaks.length} break(s)` : "PASS"}`);
  }

  if (flags.write) {
    const doc = {
      schema: 1,
      rustdoc_format: FORMAT,
      note: "OPT23 API-break baseline: FNV-1a 64 of each public item's shape, recorded by node tools/api-break.mjs --write. A break here is a removed or changed item; bless it only after the version bump or the deliberate fix.",
      crates: recorded,
      unbuilt,
    };
    writeFileSync(join(flags.root, "tools", BASELINE_NAME), `${JSON.stringify(doc, null, 1)}\n`);
    const skipped = Object.keys(unbuilt);
    out(`WROTE tools/${BASELINE_NAME} (${Object.keys(recorded).length} crates${skipped.length ? `, ${skipped.length} unbuilt: ${skipped.join(", ")}` : ""})`);
    return failedRun && !Object.keys(recorded).length ? 2 : 0;
  }

  if (loaded.baseline.crates) {
    for (const name of Object.keys(loaded.baseline.crates)) {
      if (chosen.some((c) => c.name === name)) continue;
      if (!wanted(name)) continue;
      breaks.push(`${name} crate removed`);
    }
  }
  for (const line of added) out(`[INFO] added (compatible): ${line}`);
  for (const line of breaks) out(`BREAK ${line}`);
  if (failedRun) {
    wrt(io.err, `RESULT COULD NOT RUN: ${Object.keys(unbuilt).length || "one or more"} crate(s) did not render`);
    return 2;
  }
  if (breaks.length) {
    out(`RESULT FAIL: ${breaks.length} break(s) over ${chosen.length} crate(s); bless with --write only after the deliberate change`);
    return 1;
  }
  out(`RESULT PASS: 0 breaks over ${chosen.length} crate(s), ${added.length} added item(s)`);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (e) {
    console.error(`RESULT FAIL: ${e.message}`);
    process.exitCode = 2;
  }
}
