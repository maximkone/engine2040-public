// feature-scan: which crate features does anybody actually use? (PW49)
//   node tools/feature-scan.mjs          print the report, exit 1 while a dead flag is left
//   node tools/feature-scan.mjs --json   print the rows as JSON
// Donor: cargo-unused-features 0.2.0 (MIT), the feature-graph walk. The tool is ours.
//
// The scan reads the manifests and the Rust files only: no cargo, no network.
//
// A feature of a workspace crate is DEAD when no other crate asks for it and no
// `cfg(feature = ...)` in its own crate reads it:
//   - no other crate's manifest turns it on (a `pkg/feature` value or a
//     dependency `features = [...]` list),
//   - no `cfg(feature = "...")` in the crate's own `src/` reads it,
//   - no target of the crate requires it (`required-features`; kam2play asks
//     for kam2-game's `window`, so the crate depends on that flag).
// A sibling feature of the same crate and a `--features` flag in a command in a
// doc are not asks: they name the flag but no other crate turns it on.
// `default` is Cargo's own name: `default = ["x"]` is the crate's default build,
// which every dependent turns on unless it says `default-features = false`, so it
// is alive. `default = []` turns on nothing at all, so it is a no-op line and it
// is dead.
//
// A feature is WEAK when exactly one place in its own `src/` reads it (one
// `cfg(feature = "...")`). One production reader is one edit away from being
// dead, so the crate map records these flags and names the reader, so each one
// gets a second reader before it dies. A test gate or a required-features line
// is support too (the report shows it as `tests:` and `required:`), but the
// production reader is the one that quietly rots.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// TOML tables as { header: [lines] }. Handles [x] and [[x]] headers.
const tables = (text) => {
  const out = {};
  let cur = "";
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const h = /^\[\[?([^\]]+)\]\]?\s*$/.exec(line.trim());
    if (h) { cur = h[1].trim(); out[cur] ??= []; continue; }
    if (cur) out[cur].push(line);
  }
  return out;
};

// The `key = value` pairs of one table. A value may span lines (an array or an
// inline table over several lines), so the reader keeps going until the
// brackets balance.
export const entries = (text) => {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out = [];
  for (let i = 0; i < lines.length; i += 1) {
    const head = /^\s*([A-Za-z0-9_.\-]+)\s*=\s*(.*)$/.exec(lines[i]);
    if (!head) continue;
    let raw = head[2];
    let depth = (raw.match(/[\[\{]/g) ?? []).length - (raw.match(/[\]\}]/g) ?? []).length;
    while (depth > 0 && i + 1 < lines.length) {
      i += 1;
      raw += `\n${lines[i]}`;
      depth += (lines[i].match(/[\[\{]/g) ?? []).length - (lines[i].match(/[\]\}]/g) ?? []).length;
    }
    out.push([head[1], raw]);
  }
  return out;
};

const strings = (raw) => [...raw.matchAll(/"([^"]+)"/g)].map((m) => m[1]);

// The [features] table: feature name -> the strings it holds.
export const featureTable = (toml) => {
  const out = {};
  for (const [key, raw] of entries((tables(toml).features ?? []).join("\n"))) {
    out[key] = strings(raw);
  }
  return out;
};

// Every `required-features` list of the manifest (bins, examples, tests, benches),
// with the target that asks for it.
export const requiredFeatures = (toml) => {
  const out = [];
  for (const [header, lines] of Object.entries(tables(toml))) {
    if (!/^(bin|example|test|bench)$/.test(header)) continue;
    let target = header;
    for (const [key, raw] of entries(lines.join("\n"))) {
      if (key === "name") target = strings(raw)[0] ?? target;
      if (key === "required-features") {
        for (const feature of strings(raw)) out.push({ feature, target });
      }
    }
  }
  return out;
};

// The dependency entries of the manifest: { key, package, features }.
// `key.workspace = true` and `key = { package = "other", ... }` both count.
export const dependencyEntries = (toml) => {
  const out = [];
  for (const [header, lines] of Object.entries(tables(toml))) {
    if (!header.endsWith("dependencies")) continue;
    for (const [key, raw] of entries(lines.join("\n"))) {
      const name = key.split(".")[0];
      const pkg = /package\s*=\s*"([^"]+)"/.exec(raw)?.[1] ?? name;
      const feats = /features\s*=\s*\[([^\]]*)\]/.exec(raw)?.[1] ?? "";
      out.push({ key: name, package: pkg, features: strings(feats) });
    }
  }
  return out;
};

// The package name of a manifest.
export const packageName = (toml) => /^name\s*=\s*"([^"]+)"/m.exec((tables(toml).package ?? []).join("\n"))?.[1] ?? "";

// Every .rs file of one crate, with the area it lives in.
const rustFiles = (dir) => {
  const out = [];
  const walk = (path, area) => {
    let list = [];
    try { list = readdirSync(path, { withFileTypes: true }); } catch { return; }
    for (const entry of list) {
      const full = join(path, entry.name);
      if (entry.isDirectory()) walk(full, area);
      else if (entry.isFile() && entry.name.endsWith(".rs")) out.push({ path: full, area });
    }
  };
  const src = join(dir, "src");
  if (existsSync(src)) walk(src, "src");
  for (const area of ["tests", "benches", "examples"]) {
    const path = join(dir, area);
    if (existsSync(path)) walk(path, area);
  }
  const build = join(dir, "build.rs");
  if (existsSync(build)) out.push({ path: build, area: "build" });
  return out;
};

// The `cfg(feature = "x")` reads of one crate: where each one is.
// Line comments go first, so a `feature = "x"` in prose is not a read.
export const gateReads = (dir) => {
  const reads = new Map();
  for (const file of rustFiles(dir)) {
    const lines = readFileSync(file.path, "utf8").replace(/\r\n/g, "\n").split("\n");
    for (const [index, line] of lines.entries()) {
      const code = line.replace(/\/\/.*$/, "");
      for (const m of code.matchAll(/feature\s*=\s*"([^"]+)"/g)) {
        const list = reads.get(m[1]) ?? [];
        list.push({ area: file.area, path: file.path, line: index + 1 });
        reads.set(m[1], list);
      }
    }
  }
  return reads;
};

// The reads of one crate that sit in its own `src/`: the production code.
// The weak rule counts these, so a flag one test file reads is not thin: the
// test is the only reader either way.
export const srcReads = (dir) => {
  const reads = new Map();
  for (const [feature, list] of gateReads(dir)) {
    const inSrc = list.filter((r) => r.area === "src");
    if (inSrc.length) reads.set(feature, inSrc);
  }
  return reads;
};

// The cargo commands of the repo that ask for a feature of a package.
export const commandRequests = (root, packages) => {
  const out = new Map();
  const note = (pkg, feat, where) => {
    const key = `${pkg}/${feat}`;
    const list = out.get(key) ?? [];
    list.push(where);
    out.set(key, list);
  };
  const files = [];
  const walk = (path) => {
    let list = [];
    try { list = readdirSync(path, { withFileTypes: true }); } catch { return; }
    for (const entry of list) {
      if (entry.name === ".git" || entry.name === "target" || entry.name === "node_modules") continue;
      const full = join(path, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile() && /\.(md|mjs|js|yml|yaml|toml|txt)$/.test(entry.name)) files.push(full);
    }
  };
  walk(root);
  for (const file of files.sort()) {
    const text = readFileSync(file, "utf8");
    for (const [index, line] of text.split("\n").entries()) {
      if (!/cargo\s/.test(line)) continue;
      const wanted = /--features[= ]([^\s"']+)/.exec(line)?.[1];
      if (!wanted) continue;
      const where = `${relative(root, file).split("\\").join("/")}:${index + 1}`;
      const picks = [...line.matchAll(/(?:-p|--package)\s+([A-Za-z0-9_\-]+)/g)].map((m) => m[1]);
      // A markdown code span glues a trailing backtick to the last name, and
      // prose glues a comma, a period or a bracket. Strip anything that is not
      // a feature-name character from both ends of each name.
      const clean = (name) => name.replace(/^[^A-Za-z0-9_/-]+|[^A-Za-z0-9_/-]+$/g, "");
      for (const feature of wanted.split(",").map((f) => clean(f.trim())).filter(Boolean)) {
        if (feature.includes("/")) {
          const [pkg, feat] = feature.split("/");
          if (packages.has(pkg)) note(pkg, feat, where);
          continue;
        }
        const targets = picks.length ? picks : [...packages];
        for (const pkg of targets) if (packages.has(pkg)) note(pkg, feature, where);
      }
    }
  }
  return out;
};

// One row per crate feature. The command walk is skipped when `commands` is off,
// so the crate map can read the weak flags without touching the repo's docs.
export const featureRows = (root = ROOT, { commands = true } = {}) => {
  const cratesDir = join(root, "crates");
  const dirs = existsSync(cratesDir) ? readdirSync(cratesDir).filter((d) => existsSync(join(cratesDir, d, "Cargo.toml"))).sort() : [];
  const manifests = dirs.map((dir) => {
    const path = join(cratesDir, dir, "Cargo.toml");
    const toml = readFileSync(path, "utf8");
    return { dir, path, toml, name: packageName(toml), features: featureTable(toml) };
  });
  const packages = new Set(manifests.map((m) => m.name).filter(Boolean));
  const rootToml = existsSync(join(root, "Cargo.toml")) ? readFileSync(join(root, "Cargo.toml"), "utf8") : "";

  // Who asks for what: another crate's manifest, or the workspace root.
  const requests = new Map();
  const ask = (pkg, feat, from) => {
    if (!packages.has(pkg)) return;
    const key = `${pkg}/${feat}`;
    const list = requests.get(key) ?? [];
    list.push(from);
    requests.set(key, list);
  };
  // A feature of the same crate that another feature turns on.
  const siblings = new Map();
  const sibling = (pkg, feat, from) => {
    const key = `${pkg}/${feat}`;
    const list = siblings.get(key) ?? [];
    list.push(from);
    siblings.set(key, list);
  };
  for (const crate of manifests) {
    // A dependency entry with a features list turns those features on.
    for (const dep of dependencyEntries(crate.toml)) {
      for (const feature of dep.features) ask(dep.package, feature, crate.name);
    }
    // A feature of this crate may name another crate's feature (`pkg/feature`).
    for (const [name, values] of Object.entries(crate.features)) {
      for (const value of values) {
        if (value.includes("/")) {
          const [pkg, feature] = value.split("/");
          ask(pkg, feature, crate.name);
        } else if (value.startsWith("dep:")) {
          continue;
        } else if (crate.features[value]) {
          sibling(crate.name, value, name);
        }
      }
    }
  }
  // The workspace root asks with its own feature table and its dependencies.
  for (const [name, values] of Object.entries(featureTable(rootToml))) {
    for (const value of values) {
      if (!value.includes("/")) continue;
      const [pkg, feature] = value.split("/");
      ask(pkg, feature, "workspace");
    }
  }
  for (const dep of dependencyEntries(rootToml)) {
    for (const feature of dep.features) ask(dep.package, feature, "workspace");
  }

  const commandMap = commands ? commandRequests(root, packages) : new Map();
  const rows = [];
  for (const crate of manifests) {
    if (!Object.keys(crate.features).length) continue;
    const reads = gateReads(join(cratesDir, crate.dir));
    const srcOnly = srcReads(join(cratesDir, crate.dir));
    const required = requiredFeatures(crate.toml).filter((r) => r.feature);
    for (const feature of Object.keys(crate.features).sort()) {
      const key = `${crate.name}/${feature}`;
      const gates = reads.get(feature) ?? [];
      const inSrc = srcOnly.get(feature) ?? [];
      const wanted = required.filter((r) => r.feature === feature);
      rows.push({
        crate: crate.name,
        dir: crate.dir,
        feature,
        key,
        body: crate.features[feature],
        requests: requests.get(key) ?? [],
        siblings: siblings.get(key) ?? [],
        commands: commandMap.get(key) ?? [],
        gates: gates.length,
        gateFiles: [...new Set(gates.map((g) => `crates/${crate.dir}/${relative(join(cratesDir, crate.dir), g.path).split("\\").join("/")}`))],
        srcGates: inSrc.length,
        srcGateFiles: [...new Set(inSrc.map((g) => `crates/${crate.dir}/${relative(join(cratesDir, crate.dir), g.path).split("\\").join("/")}`))],
        required: wanted.length,
        requiredTargets: [...new Set(wanted.map((r) => r.target))],
      });
    }
  }
  return rows;
};

// Weak = alive but thin: exactly one place in its own `src/` reads it. The
// manifests and the Rust files decide this, so the list does not move when a
// doc changes.
export const isWeak = (row) => row.feature !== "default" && row.srcGates === 1;

// The weak flags with the place that reads them.
export const weakFeatures = (root = ROOT) => featureRows(root, { commands: false })
  .filter((row) => isWeak(row))
  .map((row) => ({
    key: row.key,
    where: row.srcGateFiles[0] ?? `crates/${row.dir}/Cargo.toml`,
    target: row.requiredTargets[0] ?? "",
  }));

// The two buckets, the row's rules:
//   DEAD = asked for by no other crate AND read by no `cfg(feature = ...)` in
//          its own crate. A sibling feature of the same crate does not count as
//          an ask, and a cargo command in a doc does not count as an ask: it
//          asks, but no code reads the flag.
//          A `required-features` line IS a read in its own crate: the crate's
//          own target list asks for the flag, so the crate depends on it.
//          `kam2play` requires kam2-game's `window`, so that flag is alive.
//   WEAK = one `cfg` use in the crate's own `src/` and no external request.
// The report prints the sibling, command and required-features counts as
// `sibling:`, `cmds:` and `required:`, so a reader can see what else names
// the flag before deleting it.
export const classify = (row) => {
  // `default = ["x"]` is the crate's default build: every dependent turns it on.
  if (row.feature === "default") return row.body.length ? "live" : "dead";
  const asked = row.requests.length;
  // A read is a `cfg(feature = ...)` in the crate's own src/ or a
  // required-features line in the crate's own manifest.
  const used = row.srcGates + row.required;
  if (!asked && !used) return "dead";
  if (isWeak(row)) return "weak";
  return "live";
};

export const scanFeatures = (root = ROOT) => {
  const rows = featureRows(root);
  const dead = rows.filter((r) => classify(r) === "dead");
  const weak = rows.filter((r) => classify(r) === "weak");
  return { rows, dead, weak };
};

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.length > 1 || (args[0] && args[0] !== "--json")) {
    console.log("usage: node tools/feature-scan.mjs [--json]");
    process.exit(2);
  }
  const scan = scanFeatures();
  if (args[0] === "--json") {
    console.log(JSON.stringify(scan, null, 1));
    process.exit(scan.dead.length ? 1 : 0);
  }
  for (const row of scan.rows) {
    const marks = [
      row.requests.length ? `crates:${row.requests.join(",")}` : "",
      row.siblings.length ? `sibling:${row.siblings.join(",")}` : "",
      row.commands.length ? `cmds:${row.commands.length}` : "",
      row.srcGates ? `src:${row.srcGates}` : "",
      row.gates > row.srcGates ? `tests:${row.gates - row.srcGates}` : "",
      row.required ? `required:${row.required}` : "",
    ].filter(Boolean).join(" ");
    console.log(`${classify(row).padEnd(5)} ${row.key.padEnd(28)} ${marks}`);
  }
  console.log(`dead features: ${scan.dead.length}`);
  console.log(`weak features: ${scan.weak.length}`);
  for (const row of scan.dead) console.log(`  dead  ${row.key}`);
  for (const row of scan.weak) {
    const where = row.srcGateFiles[0] ?? `crates/${row.dir}/Cargo.toml`;
    console.log(`  weak  ${row.key}  ${where}`);
  }
  process.exit(scan.dead.length ? 1 : 0);
}
