// docs-links: every relative link in the live docs resolves.
//   node tools/docs-links.mjs            check tracked markdown outside docs/archive
//   node tools/docs-links.mjs --json     the same, as JSON
// History under docs/archive/ keeps museum links; a live page must resolve.
// External links, anchors and build output (docs/api, rustdoc) are not checked.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

/** Targets that are not files in this tree: build output and placeholders. */
const SKIP_PREFIXES = ["api/", "<"];
const LINK = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;

/** Relative link targets of one markdown body, with 1-based line numbers. */
export function extractLinks(text) {
  const out = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((line, i) => {
    LINK.lastIndex = 0;
    let m;
    while ((m = LINK.exec(line)) !== null) {
      const target = m[1];
      if (/^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith("#")) continue;
      if (/^['"]/.test(target)) continue; // JS call in prose, not a markdown link
      if (SKIP_PREFIXES.some((p) => target.startsWith(p))) continue;
      if (target.includes("::") || target.endsWith("...")) continue;
      out.push({ target, line: i + 1 });
    }
  });
  return out;
}

/** Every broken relative link in a list of {file, text} docs, resolved from root. */
export function findBroken(root, docs) {
  const broken = [];
  for (const { file, text } of docs) {
    for (const { target, line } of extractLinks(text)) {
      const bare = decodeURIComponent(target.split("#")[0].split("?")[0]);
      if (!bare) continue;
      const rel = target.startsWith("/")
        ? posix.normalize(bare.slice(1))
        : posix.normalize(posix.join(posix.dirname(file.split(sep).join("/")), bare));
      const abs = join(root, rel.split("/").join(sep));
      if (!existsSync(abs)) broken.push({ file, line, target, resolved: rel });
    }
  }
  return broken;
}

/** Tracked live markdown, docs/archive excluded. */
export function liveDocs(root) {
  const files = execFileSync("git", ["ls-files", "*.md"], { cwd: root, encoding: "utf8" })
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((f) => f && !f.startsWith("docs/archive/"));
  return files.map((file) => {
    const abs = join(root, file.split("/").join(sep));
    if (!existsSync(abs) || statSync(abs).isDirectory()) return { file, text: "" };
    return { file, text: readFileSync(abs, "utf8") };
  });
}

export function checkDocs(root) {
  const docs = liveDocs(root);
  const broken = findBroken(root, docs);
  return { docs: docs.length, broken };
}

if (import.meta.main) {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const json = process.argv.includes("--json");
  const { docs, broken } = checkDocs(root);
  if (json) {
    console.log(JSON.stringify({ docs, broken }, null, 2));
  } else if (broken.length) {
    console.log(`FAIL docs-links: ${broken.length} broken of ${docs} live docs`);
    for (const b of broken) console.log(`  ${b.file}:${b.line} -> ${b.target} (${b.resolved})`);
  } else {
    console.log(`PASS docs-links: ${docs} live docs, every relative link resolves`);
  }
  process.exit(broken.length ? 1 : 0);
}
