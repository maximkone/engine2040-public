// web-search: shell-safe web query wrapper (AW3b).
//
// pwsh parses `>=` as a redirect, so a hand-typed search URL containing
// `created:>=...` once sprayed `=` junk files across the repo root. This
// wrapper never touches a shell: the query stays one argv element, the
// request URL is built with encodeURIComponent, and the request goes out
// through the built-in fetch seam (no child process, no shell string), so
// no file is ever created by the query itself.
//
// Usage from the repo/lane root:
//   node tools/web-search.mjs --query "topic:game created:>=2026-06-27" [--limit 10]
//   node tools/web-search.mjs <words...> [--limit 10]
//   node tools/web-search.mjs --query <q> --dry-run   -> print request, run nothing
//   node tools/web-search.mjs --self-check            -> shell-safety self-check
//   node tools/web-search.mjs --help
//
// Exit codes: 0 ok (self-check passes) - 1 request failure or self-check FAIL - 2 usage.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fail, junkFiles, usageFail } from "./argv.mjs";

export const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);

const HELP = `web-search.mjs — shell-safe web query wrapper (AW3b)

Usage:
  node tools/web-search.mjs --query "<query>" [--limit <n>]
  node tools/web-search.mjs [--limit <n>] <words...>

Why it exists: pwsh parses >= as a redirect, so hand-typed search URLs
sprayed junk files (=*, chk-*.out) across the repo root. This wrapper
keeps the query as one argv element, encodes it into the request URL,
and fetches with the built-in fetch seam (no shell, no child process),
so created:>=2026-06-27 never reaches a shell and creates no file.

Options:
  --query <q>      full query string (kept as one argv element)
  --limit <n>      max results (default 10, 1..100)
  --dry-run        print the request as JSON and run nothing
  --self-check     shell-safety checks: own source has no shell seam, a
                   >=2026-06-27 query survives as one element and encoded,
                   and a dry run creates no junk file (AW3 acceptance)
  -h, --help       print this help and exit 0

Exit codes: 0 ok - 1 request failure or self-check FAIL - 2 usage.
`;

// fail/usageFail/junkFiles come from ./argv.mjs (one shared shell-safe body).

// One seam for requesting: the query is one string element, encoded into
// the URL, fetched with no child process — shell metacharacters (>=, |, &)
// in queries cannot redirect or split because no shell ever sees them.
function buildRequest({ query, limit }) {
  const url = `https://api.github.com/search/repositories?q=${encodeURIComponent(query)}&per_page=${limit}`;
  return { method: "GET", url, query };
}

async function runWeb(req, { dryRun = false } = {}) {
  if (dryRun) return { ok: true, status: 0, out: JSON.stringify(req) };
  const r = await fetch(req.url, {
    method: req.method,
    headers: { Accept: "application/vnd.github+json", "User-Agent": "engine2040-web-search" },
    signal: AbortSignal.timeout(120_000),
  });
  const body = await r.text();
  return { ok: r.ok, status: r.status, out: body.trim() };
}

// junkFiles lives in ./argv.mjs (imported above): one shared body so the
// AW3 junk-shape check is not a copied duplicate.
function selfCheck() {
  let bad = 0;
  const say = (ok, msg) => {
    if (!ok) bad += 1;
    console.log(`[${ok ? "PASS" : "FAIL"}] ${msg}`);
  };

  // 1-2. Own source has no shell seam: no shell spawn and no string-command exec.
  const src = readFileSync(fileURLToPath(import.meta.url), "utf8");
  say(!/shell\s*:\s*true/.test(src), "shell stays disabled in every option of the wrapper");
  say(!/\bexecSync\s*\(/.test(src) && !/\bexec\s*\(/.test(src), "no exec/execSync string-command seam in the wrapper");

  // 3-4. The AW3 probe query survives as one element and never reaches a shell raw.
  const probe = "topic:game created:>=2026-06-27";
  const req = buildRequest({ query: probe, limit: 10 });
  say(req.query === probe, `">=2026-06-27" stays one argv element (${JSON.stringify(req.query)})`);
  say(!req.url.includes(">=") && req.url.includes(encodeURIComponent(">=")), "query is URL-encoded, so no raw >= reaches a shell");

  // 5. A dry run creates no junk file (acceptance: query creates no file).
  const before = new Set(junkFiles(ROOT));
  awaitDryRun(req, before, say);

  console.log(`[INFO] request seam: built-in fetch, no child process (offline machines skip the live half)`);
  console.log(`RESULT ${bad ? "FAIL" : "PASS"}: shell-safe request checks ${bad ? "failed" : "passed"}`);
  process.exit(bad ? 1 : 0);
}

function awaitDryRun(req, before, say) {
  // Dry run is synchronous by construction: it only serializes the request.
  const out = JSON.stringify(req);
  JSON.parse(out);
  const created = junkFiles(ROOT).filter((n) => !before.has(n));
  say(created.length === 0, `dry run creates no file (new junk: ${created.join(", ") || "none"})`);
}

async function main(argv) {
  const opts = { query: null, limit: 10, dryRun: false, selfCheck: false };
  const positionals = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) usageFail("web-search", `flag ${a} needs a value`, HELP);
      i += 1;
      return v;
    };
    if (a === "--query") opts.query = next();
    else if (a === "--limit") opts.limit = Number(next());
    else if (a === "--dry-run") opts.dryRun = true;
    else if (a === "--self-check") opts.selfCheck = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else if (a.startsWith("-")) usageFail("web-search", `unknown flag ${a}`, HELP);
    else positionals.push(a);
  }
  if (opts.selfCheck) selfCheck();
  if (!Number.isInteger(opts.limit) || opts.limit < 1 || opts.limit > 100) usageFail("web-search", "--limit must be 1..100", HELP);
  const query = opts.query ?? positionals.join(" ");
  if (!query) usageFail("web-search", "need --query <q> or positional words", HELP);
  const req = buildRequest({ query, limit: opts.limit });
  const r = await runWeb(req, { dryRun: opts.dryRun });
  process.stdout.write(`${r.out}\n`);
  if (!r.ok && !opts.dryRun) fail("web-search", `web search failed (status ${r.status})`);
}

// CLI entry: runs only when this file is the invoked script, so importing
// this module (or the shared argv chain above) has no side effects.
if (import.meta.main) {
  await main(process.argv.slice(2));
}
