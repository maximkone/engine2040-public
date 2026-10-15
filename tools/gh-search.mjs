// gh-search: shell-safe GitHub search wrapper (AW3).
//
// pwsh parses `>=` as a redirect, so a bare `gh search repos ...>=...`
// once sprayed `=` junk files across the repo root. This wrapper calls gh
// with an argument array (spawnSync, shell:false), so `>=2026-06-27` stays
// one argv element and no file is ever created by the query itself.
//
// Usage from the repo/lane root:
//   node tools/gh-search.mjs --query "topic:game created:>=2026-06-27" [--limit 10]
//   node tools/gh-search.mjs [--type repos|code|issues|prs] <words...>
//   node tools/gh-search.mjs --query <q> --dry-run   -> print argv, run nothing
//   node tools/gh-search.mjs --self-check            -> shell-safety self-check
//   node tools/gh-search.mjs --help
//
// Exit codes: 0 ok (self-check passes) - 1 gh failure or self-check FAIL - 2 usage.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fail, junkFiles as argvJunk, runArgv, usageFail } from "./argv.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);

const HELP = `gh-search.mjs — shell-safe GitHub search wrapper (AW3)

Usage:
  node tools/gh-search.mjs --query "<query>" [--type repos|code|issues|prs] [--limit <n>]
  node tools/gh-search.mjs [--type <t>] [--limit <n>] <words...>

Why it exists: pwsh parses >= as a redirect, so hand-typed gh queries
sprayed junk files (=*, chk-*.out) across the repo root. This wrapper
calls gh with an argument array (no shell), so created:>=2026-06-27
stays one argv element and creates no file.

Options:
  --query <q>      full query string (kept as one argv element)
  --type <t>       repos (default), code, issues, prs
  --limit <n>      max results (default 10, 1..100)
  --dry-run        print the gh argv as JSON and run nothing
  --self-check     shell-safety checks: own source uses argv only, a
                   >=2026-06-27 query survives as one element, and a
                   dry run creates no junk file (AW3 acceptance, offline half)
  -h, --help       print this help and exit 0

Exit codes: 0 ok - 1 gh failure or self-check FAIL - 2 usage.
`;

const TYPES = new Set(["repos", "code", "issues", "prs"]);

// One seam for spawning: always an argument array, never a shell string,
// so shell metacharacters (>=, |, &) in queries cannot redirect or split.
// The shared body lives in ./argv.mjs (runArgv); this wrapper pins cmd "gh".
function runGh(args, opts) {
  return runArgv("gh", args, opts);
}

function buildArgs({ type, query, limit }) {
  const fields =
    type === "repos"
      ? "fullName,url,description,createdAt"
      : type === "code"
        ? "path,url,repository"
        : "title,url,state,createdAt";
  return ["search", type, query, "--limit", String(limit), "--json", fields];
}

// Shared helper body lives in ./argv.mjs (both wrappers import it); this
// export stays so existing importers keep working: AW3 junk shapes,
// shell-redirect droppings (=*) plus chk-*.out probes.
export function junkFiles() {
  return argvJunk(ROOT);
}

function selfCheck() {
  let bad = 0;
  const say = (ok, msg) => {
    if (!ok) bad += 1;
    console.log(`[${ok ? "PASS" : "FAIL"}] ${msg}`);
  };

  // 1. gh goes through the shared argv seam (argument array only).
  const src = readFileSync(fileURLToPath(import.meta.url), "utf8");
  say(/from "\.\/argv\.mjs"/.test(src), "gh uses the shared argv seam (./argv.mjs)");
  say(/runArgv\(\s*"gh"\s*,/.test(src), "gh is spawned with an argument array (runArgv(\"gh\", ...))");
  say(!/shell\s*:\s*true/.test(src), "shell stays disabled in every spawn option");
  say(!/\bexecSync\s*\(/.test(src) && !/\bexec\s*\(/.test(src), "no exec/execSync string-command seam in the wrapper");

  // 2. The AW3 probe query survives as one argv element.
  const probe = "topic:game created:>=2026-06-27";
  const args = buildArgs({ type: "repos", query: probe, limit: 10 });
  say(args.includes(probe), `">=2026-06-27" stays one argv element (${JSON.stringify(args)})`);

  // 3. A dry run creates no junk file (acceptance: query creates no file).
  const before = new Set(junkFiles());
  runGh(args, { dryRun: true });
  const created = junkFiles().filter((n) => !before.has(n));
  say(created.length === 0, `dry run creates no file (new junk: ${created.join(", ") || "none"})`);

  // 4. gh presence, via the same argv seam (advisory: offline machines lack gh).
  const probeRun = runGh(["--version"]);
  console.log(`[INFO] gh --version via argv: ${probeRun.ok ? probeRun.out.split("\n")[0] : `unavailable (${probeRun.out.slice(0, 120)})`}`);

  console.log(`RESULT ${bad ? "FAIL" : "PASS"}: shell-safe argv checks ${bad ? "failed" : "passed"}`);
  process.exit(bad ? 1 : 0);
}

function main(argv) {
  const opts = { type: "repos", query: null, limit: 10, dryRun: false, selfCheck: false };
  const positionals = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) usageFail("gh-search", `flag ${a} needs a value`, HELP);
      i += 1;
      return v;
    };
    if (a === "--query") opts.query = next();
    else if (a === "--type") opts.type = next();
    else if (a === "--limit") opts.limit = Number(next());
    else if (a === "--dry-run") opts.dryRun = true;
    else if (a === "--self-check") opts.selfCheck = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else if (a.startsWith("-")) usageFail("gh-search", `unknown flag ${a}`, HELP);
    else positionals.push(a);
  }
  if (opts.selfCheck) selfCheck();
  if (!TYPES.has(opts.type)) usageFail("gh-search", `--type must be one of ${[...TYPES].join("|")}`, HELP);
  if (!Number.isInteger(opts.limit) || opts.limit < 1 || opts.limit > 100) usageFail("gh-search", "--limit must be 1..100", HELP);
  const query = opts.query ?? positionals.join(" ");
  if (!query) usageFail("gh-search", "need --query <q> or positional words", HELP);
  const args = buildArgs({ type: opts.type, query, limit: opts.limit });
  const r = runGh(args, { dryRun: opts.dryRun });
  process.stdout.write(`${r.out}\n`);
  if (!r.ok && !opts.dryRun) fail("gh-search", `gh search failed (exit ${r.status})`);
}

// Importable (junkFiles above) and still a CLI: main runs only when this
// file is the invoked script, so `import { junkFiles } from "./gh-search.mjs"`
// has no side effects.
if (import.meta.main) {
  main(process.argv.slice(2));
}
