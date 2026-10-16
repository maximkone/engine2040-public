// OPT36 feature-unify (cargo-hakari pattern, MIT OR Apache-2.0).
// Union lives in crates/workspace-hack/Cargo.toml; fails outside it.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
export function parseDeps(text) {
  const sec = text.split("[dependencies]")[1]?.split("\n[")[0] ?? "";
  const out = new Map();
  const re = /([A-Za-z0-9_-]+)\s*=\s*\{[^}]*?features\s*=\s*\[([^\]]*)\]/gs;
  let m;
  while ((m = re.exec(sec)) !== null) {
    if (m[1] === "engine-core") continue;
    const feats = [...m[2].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
    out.set(m[1], { features: feats, noDefaults: /default-features\s*=\s*false/.test(m[0]) });
  }
  return out;
}
export function findViolations(crateDir, member, union) {
  const bad = [];
  for (const [name, dep] of member) {
    const allow = union.get(name);
    if (!allow) continue;
    for (const f of dep.features) if (!allow.features.includes(f)) bad.push(`${crateDir}: ${name} feature "${f}" outside union`);
    if (allow.noDefaults && !dep.noDefaults) bad.push(`${crateDir}: ${name} needs default-features = false`);
  }
  return bad;
}
export function checkUnify(repoDir) {
  const union = parseDeps(readFileSync(join(repoDir, "crates", "workspace-hack", "Cargo.toml"), "utf8"));
  const bad = [];
  for (const d of readdirSync(join(repoDir, "crates"))) {
    if (d === "workspace-hack") continue;
    let text;
    try { text = readFileSync(join(repoDir, "crates", d, "Cargo.toml"), "utf8"); } catch { continue; }
    bad.push(...findViolations(d, parseDeps(text), union));
  }
  return bad;
}
if (import.meta.main) {
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  const bad = checkUnify(repoDir);
  if (bad.length) { console.log(`FAIL feature-unify:\n${bad.join("\n")}`); process.exit(1); }
  console.log("PASS feature-unify: every feature inside the union");
}
