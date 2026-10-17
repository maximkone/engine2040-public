// Run: node --test tools/book-commands.test.mjs
// DOC2: the four guide pages (web-build, physics-layers, quality-tiers, troubleshooting) name only commands that still exist.
//  - every fenced shell command in the pages is listed in tools/book-commands.json, and every listed command is on its pages;
//  - a `node tools/x.mjs` names a real file, a `cargo test -p pkg filter` names a real package and a real test or module;
//  - the commands marked dry run here and must exit 0, so a renamed tool breaks the build;
//  - the numbers in quality-tiers.md equal the Rust tables they describe (quality.rs and caps.rs).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { chapters } from "./book-links.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const PAGES = ["web-build.md", "physics-layers.md", "quality-tiers.md", "troubleshooting.md"];
const SHELL = /^(?:sh|bash|shell|powershell|pwsh|console|cmd)$/;
const read = (p) => readFileSync(join(ROOT, p), "utf8").replace(/\r\n/g, "\n");
const manifest = JSON.parse(read("tools/book-commands.json"));

// The commands of one page: every non-empty line of every fenced block whose language is a shell.
export function fencedCommands(markdown) {
  const out = [];
  for (const m of markdown.matchAll(/^```(\w*)\n([\s\S]*?)^```/gm)) {
    if (!SHELL.test(m[1])) continue;
    for (const line of m[2].split("\n")) if (line.trim()) out.push(line.trim());
  }
  return out;
}

// A dry command is one of these shapes and nothing else: it cannot write, download or launch anything.
export const DRY_SHAPES = [/^node tools\/[\w.-]+\.mjs --help$/, /^node --test tools\/[\w.-]+\.test\.mjs$/, /^(?:node|cargo|rustc) --version$/];

const packages = () => {
  const names = new Map([["engine2040", "."]]);
  for (const d of readdirSync(join(ROOT, "crates"))) {
    const toml = join(ROOT, "crates", d, "Cargo.toml");
    const m = existsSync(toml) ? /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(toml, "utf8")) : null;
    if (m) names.set(m[1], join("crates", d));
  }
  return names;
};
const rsFiles = (dir, out = []) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) rsFiles(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

test("the manifest lists exactly the fenced shell commands of the four pages", () => {
  const byPage = new Map(PAGES.map((p) => [p, fencedCommands(read(`docs/book/${p}`))]));
  for (const [page, cmds] of byPage) {
    assert.ok(cmds.length >= 3, `${page} has runnable snippets`);
    for (const c of cmds) {
      const entry = manifest.commands.find((e) => e.cmd === c);
      assert.ok(entry, `${page}: \`${c}\` is not in tools/book-commands.json`);
      assert.ok(entry.pages.includes(page), `${page}: \`${c}\` lists the wrong pages in the manifest`);
    }
  }
  for (const e of manifest.commands) {
    assert.equal(typeof e.dry, "boolean", `${e.cmd}: dry is true or false`);
    if (!e.dry) assert.ok(e.why, `${e.cmd}: a command that is not dry says why`);
    for (const page of e.pages) assert.ok(byPage.get(page)?.includes(e.cmd), `${e.cmd} is listed for ${page} but is not a fenced command there`);
  }
  assert.equal(new Set(manifest.commands.map((e) => e.cmd)).size, manifest.commands.length, "no command is listed twice");
});

test("every node tool and cargo package a command names exists", () => {
  const pkgs = packages();
  for (const { cmd } of manifest.commands) {
    for (const m of cmd.matchAll(/\btools\/([\w.-]+\.mjs)\b/g)) assert.ok(existsSync(join(ROOT, "tools", m[1])), `${cmd}: tools/${m[1]} does not exist`);
    const cargo = /\bcargo (?:test|check|clippy)\b(.*)$/.exec(cmd);
    if (!cargo) continue;
    const words = cargo[1].trim().split(/\s+/);
    const pkg = words[words.indexOf("-p") + 1];
    assert.ok(pkgs.has(pkg), `${cmd}: ${pkg} is not a workspace package`);
    // The test filter: the word after --lib, or the last bare word. It must be a test fn, a module or a file in that crate.
    const rest = words.slice(words.indexOf("-p") + 2);
    const skip = new Set(["--lib", "--features"]);
    const filter = rest.filter((w, i) => !w.startsWith("-") && !skip.has(rest[i - 1]) && rest[i - 1] !== "--features").pop();
    if (!filter) continue;
    const src = rsFiles(join(ROOT, pkgs.get(pkg))).map((f) => [f, readFileSync(f, "utf8")]);
    const found = src.some(([f, t]) => new RegExp(String.raw`\bfn ${filter}\b|\bmod ${filter}\b`).test(t) || f.replace(/\\/g, "/").endsWith(`/${filter}.rs`));
    assert.ok(found, `${cmd}: no test fn, module or file named ${filter} in ${pkg}`);
  }
});

test("the dry commands have a safe shape and exit 0", () => {
  const dry = manifest.commands.filter((e) => e.dry);
  assert.ok(dry.length >= 3, "the pages carry dry commands");
  const env = { ...process.env };
  delete env.NODE_TEST_CONTEXT; // a child `node --test` must run as its own test run
  for (const { cmd } of dry) {
    assert.ok(DRY_SHAPES.some((re) => re.test(cmd)), `${cmd}: a dry command is --help, --version or node --test of a named test file`);
    const [bin, ...args] = cmd.split(/\s+/);
    const r = spawnSync(bin === "node" ? process.execPath : bin, args, { cwd: ROOT, env, encoding: "utf8", timeout: 120_000 });
    assert.equal(r.status, 0, `${cmd} exited ${r.status}: ${String(r.stderr || r.stdout).slice(-400)}`);
  }
});

test("the dry-shape guard refuses a command that writes or downloads", () => {
  for (const bad of ["node tools/web-build.mjs", "curl -sI https://example.com", "node tools/web-doctor.mjs", "sudo apt-get install -y x"]) assert.ok(!DRY_SHAPES.some((re) => re.test(bad)), bad);
});

test("the four pages are linked from SUMMARY.md", () => {
  const listed = chapters(read("docs/book/SUMMARY.md")).map((c) => c.path);
  for (const p of PAGES) assert.ok(listed.includes(p), `${p} is not in SUMMARY.md`);
});

// ---- quality-tiers.md equals the Rust tables ----
const rustTable = (file, head) => {
  // { field: { Tier: value } } from `Tier::X => Head { a: 1, b: Gtao::Off, .. }` arms
  const text = read(file);
  const out = {};
  for (const arm of text.matchAll(new RegExp(String.raw`(?:Quality|Tier)::(\w+)\s*=>\s*${head}\s*\{([^}]*)\}`, "g"))) {
    for (const f of arm[2].matchAll(/(\w+):\s*([^,\n]+)/g)) (out[f[1]] ??= {})[arm[1]] = f[2].trim().replace(/^Gtao::/, "").replace(/_/g, "");
  }
  return out;
};
const pageRows = (md) => {
  const rows = new Map();
  for (const line of md.split("\n")) {
    const cells = line.split("|").map((c) => c.trim());
    if (cells.length < 5 || !cells[1]) continue;
    rows.set(cells[1].replace(/`/g, "").toLowerCase().replace(/\s*\(.*\)/, "").replace(/ /g, "_"), cells.slice(2, -1));
  }
  return rows;
};

test("quality-tiers.md carries the render knobs and the physfx caps of the Rust tables", () => {
  const md = read("docs/book/quality-tiers.md");
  const rows = pageRows(md);
  const knobs = rustTable("crates/render/src/quality.rs", "Knobs");
  assert.ok(Object.keys(knobs).length >= 12, "parsed the knob table");
  for (const [field, byTier] of Object.entries(knobs)) {
    if (field === "mip_bias_x100") continue; // flat 100 on every tier; the page leaves it out
    const row = rows.get(field);
    assert.ok(row, `the page has a row for ${field}`);
    const cells = row.slice(1); // skip the unit column
    assert.deepEqual(cells.map((c) => c.toLowerCase()), ["Potato", "Comfort", "High"].map((t) => String(byTier[t]).toLowerCase()), `${field} on the page equals quality.rs`);
  }
  const caps = rustTable("crates/physfx/src/caps.rs", "Caps");
  assert.ok(Object.keys(caps).length >= 8, "parsed the caps table");
  for (const [field, byTier] of Object.entries(caps)) {
    const row = rows.get(field);
    assert.ok(row, `the page has a row for ${field}`);
    assert.deepEqual(row.map((c) => c.replace(/,/g, "")), ["Potato", "Web", "Comfort", "High"].map((t) => String(byTier[t])), `${field} on the page equals caps.rs`);
  }
  const bars = [...read("crates/physfx/src/caps.rs").matchAll(/Tier::(\w+)\s*=>\s*([\d_]+),/g)].map((m) => [m[1], m[2].replace(/_/g, "")]);
  assert.deepEqual(bars.map((b) => b[0]), ["Potato", "Web", "Comfort", "High"]);
  assert.deepEqual(rows.get("step_bar").map((c) => c.replace(/,/g, "")), bars.map((b) => b[1]), "the step bar row equals tick_bar_us");
  const potato = JSON.parse(read("tools/potato-bar.json"));
  assert.ok(md.includes(`p50 ${potato.p50_ms} ms`) && md.includes(`p99 ${potato.p99_ms} ms`) && md.includes(`${potato.working_set_mb} MB`), "the potato bar numbers equal tools/potato-bar.json");
});

test("the numbers the guides quote from the probe appear in the probe report", () => {
  const probe = read("docs/research/web-2026-10-17/07-wasm-probe.md");
  const pages = PAGES.map((p) => read(`docs/book/${p}`)).join("\n");
  for (const n of ["d57b5c8ae3096614", "2aca9338670aa0cb", "b37fc6d66df2d417", "5d7aedd4c6095b7c", "2,552,864", "638,281", "433,950"]) {
    if (pages.includes(n)) assert.ok(probe.includes(n), `${n} is quoted in the guides but not in the probe report`);
  }
  assert.ok(pages.includes("d57b5c8ae3096614") && pages.includes("5d7aedd4c6095b7c"), "the guides quote the golden and the ragdoll digest");
});
