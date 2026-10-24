// OPT36 feature-unify (cargo-hakari pattern, MIT OR Apache-2.0).
// Union lives in crates/workspace-hack/Cargo.toml; fails outside it.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
export function parseDeps(text, section = "dependencies") {
  const marker = `[${section}]`;
  const at = text.indexOf(marker);
  if (at < 0) return new Map();
  const sec = text.slice(at + marker.length).split(/\n(?=\[)/)[0];
  const out = new Map();
  const re = /^([A-Za-z0-9_-]+)\s*=\s*(?:\{([^}]*)\}|"[^"]*")/gm;
  let m;
  while ((m = re.exec(sec)) !== null) {
    if (m[1] === "engine-core") continue;
    const body = m[2] ?? "";
    const farr = body.match(/features\s*=\s*\[([^\]]*)\]/);
    out.set(m[1], {
      features: farr ? [...farr[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]) : [],
      noDefaults: /default-features\s*=\s*false/.test(body),
      workspace: /workspace\s*=\s*true/.test(body),
    });
  }
  return out;
}
// A `workspace = true` dependency takes `default-features` from the root
// `[workspace.dependencies]` table, so the member's own line need not repeat
// it. Without the root map the check reports a false violation.
export function findViolations(crateDir, member, union, root = new Map()) {
  const bad = [];
  for (const [name, dep] of member) {
    const allow = union.get(name);
    if (!allow) continue;
    for (const f of dep.features) if (!allow.features.includes(f)) bad.push(`${crateDir}: ${name} feature "${f}" outside union`);
    const noDefaults = dep.workspace && root.has(name) ? root.get(name).noDefaults : dep.noDefaults;
    if (allow.noDefaults && !noDefaults) bad.push(`${crateDir}: ${name} needs default-features = false`);
  }
  return bad;
}
// The metadata twin of checkUnify: cargo has already resolved `workspace =
// true` inheritance, dependency renames, default-features and target gating, so
// this reads the feature set a member really builds with. Host builds only: a
// target-gated dependency (uuid's `js` on wasm32) is a separate resolve, and the
// union unifies the host graph.
export const HACK_CRATE = "engine-workspace-hack";

export function resolveViolations(metadata) {
  const hack = (metadata?.packages ?? []).find((p) => p.name === HACK_CRATE);
  if (!hack) return [`${HACK_CRATE}: the union crate is missing from the workspace fix: restore crates/workspace-hack with its externals`];
  const union = new Map();
  for (const dep of hack.dependencies ?? []) {
    if (dep.path) continue;
    union.set(dep.name, { features: new Set(dep.features ?? []), noDefaults: dep.uses_default_features === false });
  }
  const members = new Set(metadata.workspace_members ?? []);
  const bad = new Set();
  for (const pkg of metadata.packages ?? []) {
    if (!members.has(pkg.id) || pkg.name === HACK_CRATE) continue;
    for (const dep of pkg.dependencies ?? []) {
      if (dep.path || dep.target) continue;
      const allow = union.get(dep.name);
      if (!allow) continue;
      for (const f of dep.features ?? []) {
        if (!allow.features.has(f)) bad.add(`${pkg.name}: ${dep.name} feature "${f}" is outside the workspace-hack union fix: add "${f}" to ${dep.name} in crates/workspace-hack/Cargo.toml, or drop it from ${pkg.name}`);
      }
      if (allow.noDefaults && dep.uses_default_features !== false) bad.add(`${pkg.name}: ${dep.name} needs default-features = false, the union turns them off fix: set default-features = false on ${dep.name} in the ${pkg.name} manifest`);
    }
  }
  return [...bad];
}

export function checkUnify(repoDir) {
  const union = parseDeps(readFileSync(join(repoDir, "crates", "workspace-hack", "Cargo.toml"), "utf8"));
  let rootText = "";
  try { rootText = readFileSync(join(repoDir, "Cargo.toml"), "utf8"); } catch { rootText = ""; }
  const root = parseDeps(rootText, "workspace.dependencies");
  const bad = [];
  for (const d of readdirSync(join(repoDir, "crates"))) {
    if (d === "workspace-hack") continue;
    let text;
    try { text = readFileSync(join(repoDir, "crates", d, "Cargo.toml"), "utf8"); } catch { continue; }
    bad.push(...findViolations(d, parseDeps(text), union, root));
  }
  return bad;
}
if (import.meta.main) {
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  const bad = checkUnify(repoDir);
  if (bad.length) { console.log(`FAIL feature-unify:\n${bad.join("\n")}`); process.exit(1); }
  console.log("PASS feature-unify: every feature inside the union");
}
