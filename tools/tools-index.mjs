// tools-index: the inventory rule for the tools/ layer.
// A live tool is a top-level `tools/*.mjs` file that is not a `*.test.mjs`
// mate, a dotfile or a file in a subdirectory. This gate enforces three rules:
//   1. every live tool is named in `tools/README.md`;
//   2. every `tools/<name>.mjs` the README names still exists;
//   3. every live tool has a same-name `tools/<name>.test.mjs` mate, unless
//      TEST_MATE_EXCEPTIONS names it with the reason a node:test run cannot
//      cover it (a GPU, a browser, Blender, a tunnel, a manual double run).
// README names for `*.test.mjs` files are listed but need no live script.
// An exception that no longer applies is a gap too: the list may only shrink
// by fixing the tool, never by leaving a stale reason.
// Run: node tools/tools-index.mjs [--json] [--check] [--help]
// Exit: 0 clean or report, 1 a gap under --check, 2 bad arguments.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// A mate is not written for a tool whose proof needs a resource this PC's test
// run does not have. The reason is the debt note; the gate still counts it.
export const TEST_MATE_EXCEPTIONS = Object.freeze({
  "arena-shot.mjs": "daily GPU capture gate; its own --self-check covers the rules, no CI runner",
  "class-validate.mjs": "hand-run content gate over content/classes; the content lane owns the fixture",
  "content-cross.mjs": "hand-run cross-file id check over the faction war content set",
  "content-lib.mjs": "library module with no CLI; exercised through the content tools that import it",
  "faction-war-map.mjs": "hand-run headless loader for content/maps and content/systems/war.json",
  "faction-war-report.mjs": "hand-run campaign report; needs Blender for the archetype blockout",
  "fk1_determinism.mjs": "manual double run of a 6k-tick match; no CI runner",
  "gate-silhouette.mjs": "needs a reference and capture PNG pair nobody publishes",
  "kam2-host.mjs": "manual host proof that needs a cloudflared tunnel",
  "kam2-web.mjs": "manual wasm page proof that needs wasm32 and headless Chrome",
  "reference-captures.mjs": "manual GPU capture verifier for the reference tiers",
  "starter-classes.mjs": "one-shot writer for content/classes; its output is reviewed, never run",
  "unwired-code-gate.mjs": "parked; wiring-scan --diff owns the new-code check",
});

// Only a backticked `tools/<file>.mjs` token names a tool. Prose like
// `*.test.mjs`, placeholders like tools/<name>.mjs and subdir paths stay out.
const README_TOOL_RE = /`tools\/([A-Za-z0-9][A-Za-z0-9._-]*\.mjs)`/g;

export function mateOf(name) {
  return name.replace(/\.mjs$/, ".test.mjs");
}

export function listLiveTools(toolsDir) {
  return readdirSync(toolsDir, { withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith(".mjs") && !e.name.startsWith(".") && !e.name.endsWith(".test.mjs"))
    .map((e) => e.name)
    .sort();
}

export function listTestFiles(toolsDir) {
  return readdirSync(toolsDir, { withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith(".test.mjs") && !e.name.startsWith("."))
    .map((e) => e.name)
    .sort();
}

export function parseReadme(text) {
  const live = new Set();
  const tests = new Set();
  for (const m of String(text).matchAll(README_TOOL_RE)) {
    if (m[1].endsWith(".test.mjs")) tests.add(m[1]);
    else live.add(m[1]);
  }
  return { live: [...live].sort(), tests: [...tests].sort() };
}

export function audit({ live, tests, readme, exceptions = TEST_MATE_EXCEPTIONS }) {
  const listed = parseReadme(readme);
  const liveSet = new Set(live);
  const testSet = new Set(tests);
  const listedLive = new Set(listed.live);
  const missingFromReadme = live.filter((n) => !listedLive.has(n));
  const staleReadme = listed.live.filter((n) => !liveSet.has(n));
  const noTestMate = live.filter((n) => !testSet.has(mateOf(n)));
  const unexplainedNoTestMate = noTestMate.filter((n) => !Object.hasOwn(exceptions, n));
  const staleExceptions = Object.keys(exceptions)
    .filter((n) => !liveSet.has(n) || !noTestMate.includes(n))
    .sort();
  const gaps = missingFromReadme.length + staleReadme.length + unexplainedNoTestMate.length + staleExceptions.length;
  return {
    ok: gaps === 0,
    gaps,
    live,
    tests,
    readmeLive: listed.live,
    readmeTests: listed.tests,
    missingFromReadme,
    staleReadme,
    noTestMate,
    explainedNoTestMate: noTestMate.filter((n) => Object.hasOwn(exceptions, n)),
    unexplainedNoTestMate,
    staleExceptions,
  };
}

const HELP = `usage: node tools/tools-index.mjs [--json] [--check]

  no flag   report the inventory gaps and the counts
  --json    the same as one JSON object
  --check   exit 1 on any gap (the ci quick job runs this on the public mirror)
  --help    print this text, exit 0`;

function printReport(r) {
  console.log(
    `tools-index: ${r.live.length} live tools, ${r.tests.length} test files, ` +
      `${r.readmeLive.length} named live, ${r.noTestMate.length} without a test mate ` +
      `(${r.explainedNoTestMate.length} explained)`,
  );
  const group = (label, items) => {
    if (items.length === 0) return;
    console.log(`${label} (${items.length}):`);
    for (const item of items) console.log(`  ${item}`);
  };
  group("missing from tools/README.md", r.missingFromReadme);
  group("README entries with no file", r.staleReadme);
  group("without a test mate and no exception", r.unexplainedNoTestMate);
  group("exceptions that no longer apply", r.staleExceptions);
  console.log(r.ok ? "tools-index: OK" : `tools-index: ${r.gaps} gap(s)`);
}

export function main(argv = process.argv.slice(2), { root = ROOT } = {}) {
  const flags = argv.filter((a) => a.startsWith("--"));
  const unknown = flags.filter((f) => !["--json", "--check", "--help"].includes(f));
  if (unknown.length > 0 || argv.length !== flags.length) {
    console.error(`unknown argument: ${(unknown.length > 0 ? unknown : argv.filter((a) => !a.startsWith("--"))).join(" ")}`);
    console.error(HELP);
    return 2;
  }
  if (flags.includes("--help")) {
    console.log(HELP);
    return 0;
  }
  const toolsDir = join(root, "tools");
  const result = audit({
    live: listLiveTools(toolsDir),
    tests: listTestFiles(toolsDir),
    readme: readFileSync(join(toolsDir, "README.md"), "utf8"),
  });
  if (flags.includes("--json")) {
    console.log(JSON.stringify({
      ok: result.ok,
      gaps: result.gaps,
      counts: {
        live: result.live.length,
        tests: result.tests.length,
        readmeLive: result.readmeLive.length,
        readmeTests: result.readmeTests.length,
        noTestMate: result.noTestMate.length,
        explainedNoTestMate: result.explainedNoTestMate.length,
        unexplainedNoTestMate: result.unexplainedNoTestMate.length,
      },
      missingFromReadme: result.missingFromReadme,
      staleReadme: result.staleReadme,
      noTestMate: result.noTestMate,
      unexplainedNoTestMate: result.unexplainedNoTestMate,
      staleExceptions: result.staleExceptions,
    }, null, 2));
  } else {
    printReport(result);
  }
  return flags.includes("--check") && !result.ok ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
