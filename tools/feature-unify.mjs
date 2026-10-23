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
