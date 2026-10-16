// Licence gate, leg 2 of row E44 (EB-2026-10-08-S151): the pre-push hook runs `cargo deny check licenses` for the
// dependency graph, then this script for the workspace itself. Two checks: unlicensed packages fail,
// unused-allow entries warn. Prints both counts, exits 1 only when unlicensed is above 0.
// Plus repo licence files plus source header lint. Prints counts, exits 1 on
// unlicensed or missing files; never skips. deny.toml stays the verdict.
// Owner 2026-10-10 ("we can take or steal all"): a copyleft/closed source header is a WARN, never a FAIL.
// PORT(EmbarkStudios/cargo-deny@735416424c5629429d7d59a6fcfc863eebe4e21f:src/licenses.rs) Apache-2.0
// (verified live 2026-10-09: gh api repos/EmbarkStudios/cargo-deny -> Apache-2.0,
// LICENSE-APACHE opens "Apache License Version 2.0").
// Donor shape: Hits { allowed: BitVec, exceptions: BitVec } with BitVec::repeat(false, len),
// as_mut_bitslice().set(i, true) when an allow or exception satisfies a crate license,
// then one warn per !hit via unused_allowed_license and unused_license_exception.
// Fresh JS port with boolean hit arrays, no donor bytes.
// Link: https://github.com/EmbarkStudios/cargo-deny/blob/735416424c5629429d7d59a6fcfc863eebe4e21f/src/licenses.rs
// Header lint (Apache SkyWalking Eyes shape, Apache-2.0): first 20 lines scanned for copyleft or closed
// declarations; a hit prints a WARN so ported files stay visible.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const unlicensed = (metadata) =>
  (metadata?.packages ?? []).filter((p) => !p.license && !p.license_file).map((p) => p.name).sort();

export const CHECKS = 2;

// Read the allow list plus per-crate exceptions from deny.toml text.
// Top-level allow sits at line start; exception allows sit inside { ... }.
export const parseDenyLicenses = (text = "") => {
  const allow = [];
  const exceptions = [];
  const src = String(text);
  const top = /^allow\s*=\s*\[([\s\S]*?)\]/m.exec(src);
  if (top) for (const m of top[1].matchAll(/"([^"]+)"/g)) allow.push(m[1]);
  for (const b of src.matchAll(/\{([^}]*)\}/g)) {
    const body = b[1];
    const crateM = /crate\s*=\s*"([^"]+)"/.exec(body);
    if (!crateM) continue;
    const allowM = /allow\s*=\s*\[([^\]]*)\]/.exec(body);
    const list = allowM ? [...allowM[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]) : [];
    exceptions.push({ crate: crateM[1], allow: list });
  }
  return { allow, exceptions };
};

// Split an SPDX string into license tokens for the satisfies check.
export const licenseTokens = (license = "") =>
  String(license)
    .split(/[\s/()|,;+]+/)
    .filter((t) => t && !["OR", "AND", "WITH"].includes(t.toUpperCase()));

// Unused-allow lint: one boolean hit per allow entry, set when a package
// license satisfies it, warn on every entry never hit.
export const unusedAllow = (allowed = [], packages = []) => {
  const hits = new Array(allowed.length).fill(false);
  for (const p of packages ?? []) {
    if (!p?.license) continue;
    const tokens = new Set(licenseTokens(p.license));
    allowed.forEach((a, i) => {
      if (tokens.has(a)) hits[i] = true;
    });
  }
  return allowed.filter((_, i) => !hits[i]);
};

// Same hit tracking for per-crate license exceptions.
export const unusedExceptions = (exceptions = [], packages = []) => {
  const hits = new Array(exceptions.length).fill(false);
  for (const p of packages ?? []) {
    if (!p?.name || !p?.license) continue;
    const tokens = new Set(licenseTokens(p.license));
    exceptions.forEach((e, i) => {
      if (p.name !== e.crate) return;
      if ((e.allow ?? []).some((a) => tokens.has(a))) hits[i] = true;
    });
  }
  return exceptions.filter((_, i) => !hits[i]);
};

export const verdict = (metadata, deny = { allow: [], exceptions: [] }) => {
  const names = unlicensed(metadata);
  const total = metadata?.packages?.length ?? 0;
  const pkgs = metadata?.packages ?? [];
  const unused = unusedAllow(deny?.allow ?? [], pkgs);
  const unusedExc = unusedExceptions(deny?.exceptions ?? [], pkgs);
  const count = unused.length + unusedExc.length;
  const detail = [...unused, ...unusedExc.map((e) => `${e.crate}:${(e.allow ?? []).join("/")}`)];
  return {
    ok: names.length === 0,
    checks: CHECKS,
    unused,
    unusedExceptions: unusedExc,
    line: `license-gate: ${total} workspace packages, unlicensed ${names.length}${names.length ? `: ${names.join(", ")}` : ""}, unused-allow ${count} (warn)${count ? `: ${detail.join(", ")}` : ""} [${CHECKS} checks]`,
  };
};

export const LICENSE_FILES = ["LICENSE-MIT", "LICENSE-APACHE"];

export const checkLicenseFiles = (repoDir = ROOT) =>
  LICENSE_FILES.filter((name) => !existsSync(join(repoDir, name)));

const HEADER_LINES = 20;

export const headText = (text = "") =>
  String(text).split(/\r?\n/).slice(0, HEADER_LINES).join("\n");

// A forbidden header declares the file itself under a copyleft or closed
// licence. Bare mentions (donor notes, idea-only attributions) do not fail:
// the match needs a declaration (Licensed under, SPDX, GNU text, a GPL
// version like GPL-3.0) or a closed marker (Proprietary, All rights reserved).
const FORBIDDEN_HEADER = /(Licensed under|SPDX-License-Identifier)[^\n]*\b(GPL|AGPL|LGPL)\b|\b(GPL|AGPL|LGPL)-\d|GNU (General Public License|Affero General Public License|Lesser General Public License)|Proprietary|All rights reserved/;

export const headerFinding = (path, text) => {
  const head = headText(text);
  if (FORBIDDEN_HEADER.test(head)) {
    const hit = head.match(FORBIDDEN_HEADER)?.[0] ?? "forbidden marker";
    return `${path}: header names ${hit} (ported code: allowed, owner 2026-10-10)`;
  }
  return null;
};

export const collectSources = (repoDir = ROOT) => {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith(".rs")) out.push(full);
    }
  };
  const crates = join(repoDir, "crates");
  if (existsSync(crates)) {
    for (const entry of readdirSync(crates, { withFileTypes: true })) {
      const src = join(crates, entry.name, "src");
      if (entry.isDirectory() && existsSync(src)) walk(src);
    }
  }
  const tools = join(repoDir, "tools");
  if (existsSync(tools)) {
    for (const entry of readdirSync(tools, { withFileTypes: true })) {
      if (!entry.isDirectory() && entry.name.endsWith(".mjs")) out.push(join(tools, entry.name));
    }
  }
  return out;
};

export const hookScript = () =>
  `#!/bin/sh\n# licence gate pre-push: deny.toml is the verdict, this surfaces it earlier.\ncommand -v git-lfs >/dev/null 2>&1 || { printf >&2 "\\n%s\\n\\n" "This repository is configured for Git LFS but 'git-lfs' was not found on your path."; exit 2; }\ngit lfs pre-push "$@"\ncargo deny check licenses || exit 1\nnode tools/license-gate.mjs || exit 1\n`;

export const installHook = (repoDir = ROOT) => {
  let hooksDir = join(repoDir, ".git", "hooks");
  try {
    const p = execFileSync("git", ["-C", repoDir, "rev-parse", "--git-path", "hooks"], { encoding: "utf8" }).trim();
    if (!p.includes(":") && !p.startsWith("/")) hooksDir = join(repoDir, p);
  } catch { /* fall back to .git/hooks */ }
  const hookPath = join(hooksDir, "pre-push");
  try {
    const current = readFileSync(hookPath, "utf8");
    if (/deny check licenses/.test(current) && /license-gate/.test(current)) {
      return { path: hookPath, installed: false, reason: "already wired" };
    }
  } catch { /* missing hook: install fresh below */ }
  writeFileSync(hookPath, hookScript());
  return { path: hookPath, installed: true, reason: "wired deny plus license-gate" };
};

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write("license-gate.mjs — pre-push licence gate (deny verdict, surfaced early)\n\nUsage:\n  node tools/license-gate.mjs [--hook|--install-hook|--help]\n\nChecks: workspace licences (0 unlicensed), repo license files, source headers.\n");
    return;
  }
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  if (argv.includes("--hook")) {
    process.stdout.write(hookScript());
    return;
  }
  if (argv.includes("--install-hook")) {
    const result = installHook(repoDir);
    process.stdout.write(`${result.installed ? "installed" : "kept"} pre-push hook: ${relative(repoDir, result.path)} (${result.reason})\n`);
    return;
  }
  // No skip: every failure below lands in errors and exits 1.
  const errors = [];
  for (const name of checkLicenseFiles(repoDir)) errors.push(`licence: repo license file missing: ${name}`);
  let raw = null;
  try {
    raw = execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", join(repoDir, "Cargo.toml")], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  } catch (error) {
    errors.push(`licence: cargo metadata could not run: ${error.message}`);
  }
  let deny = null;
  try {
    deny = parseDenyLicenses(readFileSync(join(repoDir, "deny.toml"), "utf8"));
  } catch (error) {
    errors.push(`licence: deny.toml could not be read: ${error.message}`);
    deny = { allow: [], exceptions: [] };
  }
  let v = { ok: true, line: "license-gate: 0 workspace packages, unlicensed 0" };
  if (raw !== null) {
    try {
      v = verdict(JSON.parse(raw), deny);
    } catch (error) {
      errors.push(`licence: cargo metadata could not be parsed: ${error.message}`);
    }
    if (!v.ok) errors.push(`licence: ${v.line}`);
  }
  let scanned = 0;
  const warns = [];
  try {
    for (const file of collectSources(repoDir)) {
      scanned += 1;
      try {
        const hit = headerFinding(relative(repoDir, file).split("\\").join("/"), readFileSync(file, "utf8"));
        if (hit) warns.push(`licence: ${hit}`);
      } catch { /* unreadable file: the module scan owns it */ }
    }
  } catch (error) {
    errors.push(`licence gate could not run: ${error.message}`);
  }
  for (const line of warns) console.error(`WARN ${line}`);
  if (errors.length) {
    for (const line of errors) console.error(`FAIL ${line}`);
    process.exitCode = 1;
    return;
  }
  console.log(`${v.line}, headers ${warns.length ? `${warns.length} ported-licence WARN` : "clean"} (${scanned} files)`);
}

if (import.meta.main) {
  main();
}
