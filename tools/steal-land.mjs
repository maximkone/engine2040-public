// steal-land: make a steal line in docs/plan/steals.md land.
//
// One open line per steal under ## Steals; the builder that lands it marks
// it `landed <sha>`. This tool picks the next line, fetches its pinned donor
// file, and gates the file: at most 3 open lines per part, no open line older
// than 7 days (center size-check.mjs ages the line by name, with no
// file-freshness excuse), no landed mark whose commit misses our/file. A
// landed sha this clone cannot resolve (the public mirror holds none of the
// private shas) is not a verdict: it passes with a named note, never as
// `touches no`. An `expired <date> (<reason>)` or `rejected <date> (<reason>)`
// line is closed.
// Old lines without a size parse as M.
//
// Usage from the repo/lane root:
//   node tools/steal-land.mjs --next <part>   oldest open line of a part
//   node tools/steal-land.mjs --fetch <id>    donor file + LICENSE to target/steals/<id>/
//   node tools/steal-land.mjs --gate           FAIL on the rules above, else PASS
//   node tools/steal-land.mjs --help
//
// Exit codes: 0 ok (gate PASS, next/fetch printed) - 1 gate FAIL or fetch
// failure - 2 usage. Reads the donor repo's own license file at the pinned
// commit, never a README badge. No network writes, no cargo.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { cliArgs } from "./argv.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const STEALS = join(ROOT, "docs", "plan", "steals.md");
const TEAM_DIR = join(ROOT, "team");
const DAY = 86_400_000;

// License allowlist: named as written; anything else lands as PORT (owner 2026-10-10: any donor may be ported).
export const ALLOW = new Set([
  "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause",
  "ISC", "Zlib", "CC0-1.0", "Unlicense",
]);
export const SIZES = new Set(["XS", "S", "M", "L", "XL"]);
const MAX_OPEN_PER_PART = 3;
const STALE_DAYS = 7;

const HELP = `steal-land.mjs ΓÇö land one steal line from docs/plan/steals.md

Usage:
  node tools/steal-land.mjs --next <part>   oldest open line of a part
      (look, bodies, aidoor, net, speed, tools, ship): size, donor file URL
      at the pinned commit, license verdict, the number it promises.
      Skips lines the part's team/<part>.md marks FAILED.
  node tools/steal-land.mjs --fetch <id>    save the donor file to
      target/steals/<id>/donor.<ext> plus a LICENSE copy. Off-allowlist
      licenses are saved with the verdict PORT (porting allowed).
  node tools/steal-land.mjs --gate          FAIL a 4th open line per part,
      an open line older than 7 days by name (center size-check.mjs's rule:
      age alone, so a fresh commit to our/file never saves the line), and a
      landed <sha> whose commit misses our/file. A landed sha this clone
      cannot resolve (the public mirror has no private shas) passes with a
      named note: it is unverifiable here, never a touches-no FAIL.
      expired <date> (<reason>) and rejected <date> (<reason>) read as
      closed, as center reads them.
  -h, --help     print this help and exit 0

Exit codes: 0 ok - 1 gate FAIL or fetch failure - 2 usage.
`;

// A steal line: `DATE | bar | donor | license | our/file | [size |] status`
// plus one indented promise line. Old lines without a size parse as M.
export function parseSteals(md) {
  const lines = String(md ?? "").split("\n");
  const start = lines.findIndex((l) => /^## Steals/.test(l.trim()));
  const out = [];
  if (start < 0) return out;
  const head = /^(\d{4}-\d{2}-\d{2}) \| (.*)$/;
  for (let i = start + 1; i < lines.length; i += 1) {
    const m = lines[i].match(head);
    if (!m) continue;
    const cells = m[2].split("|").map((c) => c.trim());
    if (cells.length !== 5 && cells.length !== 6) {
      throw new Error(`steals.md line ${i + 1}: want 5-6 cells (date | bar | donor | license | our/file | [size |] status), got ${cells.length}`);
    }
    const [bar, donor, license, our] = cells;
    const sized = cells.length === 6;
    const size = sized ? cells[4].toUpperCase() : "M";
    const statusRaw = cells[sized ? 5 : 4];
    const landed = statusRaw.match(/^landed\s+([0-9a-fA-F]{7,40})(?:\s+\(.*\))?$/i);
    // Center's size check closes a line with `expired <date> (<reason>)` or
    // `rejected <date> (<reason>)`; read both the same way so one ledger
    // serves both gates (2026-10-17).
    const closed = statusRaw.match(/^(expired|rejected)\b/i);
    const status = /^open$/i.test(statusRaw) ? "open" : landed ? "landed" : closed ? closed[1].toLowerCase() : statusRaw;
    let promise = "";
    if (i + 1 < lines.length && /^[ \t]+\S/.test(lines[i + 1])) {
      promise = lines[i + 1].trim();
    }
    out.push({
      id: out.length + 1, date: m[1], bar, donor, license, our, size,
      status, sha: landed ? landed[1] : null, promise, line: i + 1,
    });
  }
  return out;
}

// Our/file maps to the part whose team/<part>.md can FAILED it.
// Every crates/* dir maps to a seat part so no program crate falls
// through to a shared `crates` part that would bind strangers together.
export function partOf(our) {
  const p = String(our ?? "").toLowerCase();
  if (p.startsWith("tools/")) return "tools";
  if (/^(crates\/(render|anim|audio|ui|viewer|material|world)|content\/look)/.test(p)) return "look";
  if (/^crates\/(assets|physics|destruction|combat|geo|proof|nav|save|observe|playtest|engine-chaos)/.test(p)) return "bodies";
  if (/^crates\/(ai|provider|endpoint|nn|distill|rules|scene|editor|spatial)/.test(p)) return "aidoor";
  if (/^crates\/net/.test(p)) return "net";
  if (/^crates\/(perf|arena|ecs|core|app|game|input)/.test(p)) return "speed";
  return p.split("/")[0] || p;
}

// The license verdict: allowlisted as written, our own code as OURS,
// anything else (including UNVERIFIED) as PORT: owner 2026-10-10 allows porting any donor.
export function verdictOf(license) {
  const raw = String(license ?? "").trim();
  if (/^(ours|our-repo)\b/i.test(raw)) return "OURS";
  const norm = raw.replace(/\s+/g, " ");
  for (const a of ALLOW) {
    if (norm.toLowerCase() === a.toLowerCase()) return a;
  }
  return "PORT";
}

// Donor cell to a pinned file URL. Our-repo tools and arXiv ids have no
// fetchable blob; a bare repo pins to its tree at the commit.
export function donorInfo(donor) {
  const raw = String(donor ?? "").trim();
  if (/our-repo tool|arsenal/i.test(raw) || /^forge\s/i.test(raw)) {
    return { kind: "our-repo", url: null, note: `our-repo tool (no fetch): ${raw}` };
  }
  const arxiv = raw.match(/arxiv:\s*([^\s),;]+)/i);
  if (arxiv) {
    return { kind: "arxiv", url: `https://arxiv.org/abs/${arxiv[1]}`, note: raw };
  }
  const gh = raw.match(/([\w.\-]+)\/([\w.\-]+)@([0-9a-fA-F]{7,40})(?::(\S+?))?(?=[\s(,]|$)/);
  if (gh) {
    const [, owner, repo, commit, path] = gh;
    const clean = path ? path.replace(/[),;:.]+$/, "") : null;
    const url = clean
      ? `https://github.com/${owner}/${repo}/blob/${commit}/${clean}`
      : `https://github.com/${owner}/${repo}/tree/${commit}`;
    return { kind: "github", owner, repo, commit, path: clean, url, note: raw };
  }
  return { kind: "unknown", url: null, note: raw };
}

const dateMs = (d) => Date.parse(`${d}T00:00:00Z`);
const noCargo = (our) => !String(our ?? "").startsWith("crates/");

// A team/<part>.md FAILED mark skips the line when it names the line's
// our/file, donor slug, or bar.
export function failedByTeam(teamMd, line) {
  const md = String(teamMd ?? "");
  if (!/FAILED/i.test(md)) return false;
  const slug = (line.donor.match(/[\w.\-]+\/[\w.\-]+/) ?? [line.donor.split(/\s/)[0]])[0];
  return md.includes(line.our) || md.includes(slug) || md.includes(line.bar);
}

// Oldest open line of a part: FAILED-skipped, no-cargo (tools) first,
// then oldest date, then file order.
export function nextFor(lines, { part, teamMd = "" } = {}) {
  const want = String(part ?? "").toLowerCase();
  const cands = lines.filter((l) =>
    l.status === "open"
    && (partOf(l.our) === want || l.our.toLowerCase().includes(want) || l.bar.toLowerCase().includes(want))
    && !failedByTeam(teamMd, l));
  cands.sort((a, b) =>
    Number(noCargo(b.our)) - Number(noCargo(a.our))
    || dateMs(a.date) - dateMs(b.date)
    || a.id - b.id);
  return cands[0] ?? null;
}

export function formatLine(l) {
  const info = donorInfo(l.donor);
  return [
    `steal #${l.id} [${l.size}] ${l.date} ${l.bar} ${l.our} ${l.status}${l.sha ? ` ${l.sha}` : ""}`,
    `  donor: ${info.url ?? info.note}`,
    `  license: ${verdictOf(l.license)} (${l.license})`,
    `  number: ${l.promise}`,
  ].join("\n");
}

// Pure gate over parsed lines; git access is injected so tests stay offline.
// commitTouches returns true (the commit touches our/file), false (the commit
// is in this clone and misses the file: a FAIL) or null (this clone cannot
// resolve the sha: a note, the only honest reading on the mirror's snapshot
// clone, which holds none of the private shas).
export function gateLines(lines, { nowMs = Date.now(), commitTouches = null } = {}) {
  const openByPart = new Map();
  for (const l of lines) {
    if (l.status !== "open") continue;
    const p = partOf(l.our);
    if (!openByPart.has(p)) openByPart.set(p, []);
    openByPart.get(p).push(l);
  }
  const over = new Map();
  for (const [p, ls] of openByPart) {
    ls.sort((a, b) => dateMs(a.date) - dateMs(b.date) || a.id - b.id);
    ls.forEach((l, i) => { if (i >= MAX_OPEN_PER_PART) over.set(l.id, p); });
  }
  return lines.map((l) => {
    const reasons = [];
    const notes = [];
    if (!SIZES.has(String(l.size).toUpperCase())) reasons.push(`unknown size '${l.size}' (want XS|S|M|L|XL, or no size for M)`);
    if (over.has(l.id)) {
      reasons.push(`4th open line for part ${over.get(l.id)} (${openByPart.get(over.get(l.id)).length} open; max ${MAX_OPEN_PER_PART})`);
    }
    if (l.status === "open") {
      // Center's rule is age alone. The old file-freshness excuse let the four
      // 2026-10-07 lines sit open for 10 days and PASS here while center's
      // size check named them (2026-10-17); with it gone both gates name the
      // same line.
      const ageMs = nowMs - dateMs(l.date);
      const age = Math.floor(ageMs / DAY);
      // Center's comparison exactly (`now - dated > 7 * DAY`), so a line is
      // named in the round it turns stale, not up to 24 h later.
      if (ageMs > STALE_DAYS * DAY) reasons.push(`open ${age}d (since ${l.date}) is older than ${STALE_DAYS} days: mark it landed, expired or rejected`);
    }
    if (l.status === "landed") {
      const touches = commitTouches ? commitTouches(l.sha, l.our) : true;
      // A sha this clone cannot resolve says nothing about our/file: the
      // public mirror's export snapshots hold no private sha (2026-10-17).
      // Reading that as `touches no` was the lie the farm saw. Name it
      // instead, and keep the bite for a sha that is here and misses.
      if (touches === null) notes.push(`landed ${l.sha} not in this clone: cannot verify ${l.our} here (private sha)`);
      else if (!touches) reasons.push(`landed ${l.sha} touches no ${l.our}`);
    }
    if (!["open", "landed", "expired", "rejected"].includes(l.status)) reasons.push(`unknown status '${l.status}' (want open, landed <sha>, expired <date> (<reason>) or rejected <date> (<reason>))`);
    return { id: l.id, ok: reasons.length === 0, reasons, notes };
  });
}

function fail(message, code = 1) {
  process.stderr.write(`steal-land: error: ${message}\n`);
  process.exit(code);
}

function loadSteals() {
  if (!existsSync(STEALS)) fail(`docs/plan/steals.md missing`);
  return parseSteals(readFileSync(STEALS, "utf8").replace(/\r\n/g, "\n"));
}

// One seam for subprocesses: argument arrays, never a shell string, and gh
// only ever reads (gh api GET on contents), never a write verb.
function runBin(bin, args, { cwd = ROOT } = {}) {
  const r = spawnSync(bin, args, { encoding: "utf8", shell: false, cwd, timeout: 120_000 });
  if (r.error) return { ok: false, out: String(r.error) };
  return { ok: r.status === 0, out: `${r.stdout ?? ""}`.trim(), err: `${r.stderr ?? ""}`.trim() };
}

// The sha is resolved first and git is asked about it only then: a sha this
// clone does not hold (the public mirror's shallow export snapshots) is
// "cannot verify here" (null), never "touches no" (false). `^{commit}` also
// takes the 7-40 char prefixes the ledger writes.
const gitCommitTouches = (sha, file) => {
  const have = runBin("git", ["cat-file", "-e", `${sha}^{commit}`]);
  if (!have.ok) return null;
  const r = runBin("git", ["show", "--pretty=", "--name-only", sha, "--"]);
  if (!r.ok) return false;
  return r.out.split("\n").map((s) => s.trim()).includes(file);
};

function cmdNext(part) {
  if (!part) {
    process.stderr.write(`steal-land: error: --next needs a part\n\n${HELP}`);
    process.exit(2);
  }
  const lines = loadSteals();
  const teamFile = join(TEAM_DIR, `${part}.md`);
  const teamMd = existsSync(teamFile) ? readFileSync(teamFile, "utf8") : "";
  const open = lines.filter((l) => l.status === "open").length;
  const landed = lines.filter((l) => l.status === "landed").length;
  const found = nextFor(lines, { part, teamMd });
  if (!found) {
    console.log(`steal-land: no open line for part '${part}' (${open} open, ${landed} landed) (RESULT: NOOP)`);
    return;
  }
  const peers = lines.filter((l) => l.status === "open" && partOf(l.our) === partOf(found.our)).length;
  console.log(formatLine(found));
  console.log(`${peers} open line(s) for part ${partOf(found.our)}; ${open} open, ${landed} landed in all (${lines.length} ideas ever filed here)`);
}

function ghFile(owner, repo, path, ref) {
  const r = runBin("gh", ["api", `repos/${owner}/${repo}/contents/${path}?ref=${ref}`, "--jq", ".content"]);
  if (!r.ok) return null;
  const b64 = r.out.replace(/\s+/g, "");
  try {
    return Buffer.from(b64, "base64");
  } catch {
    return null;
  }
}

function cmdFetch(idRaw) {
  const id = Number(idRaw);
  if (!Number.isInteger(id) || id < 1) {
    process.stderr.write(`steal-land: error: --fetch needs a line id (1-based)\n\n${HELP}`);
    process.exit(2);
  }
  const lines = loadSteals();
  const line = lines.find((l) => l.id === id);
  if (!line) fail(`no steal line #${id} (${lines.length} lines)`);
  const info = donorInfo(line.donor);
  if (info.kind !== "github" || !info.path) fail(`line #${id} has no fetchable donor file: ${info.note}`);
  const body = ghFile(info.owner, info.repo, info.path, info.commit);
  if (!body) fail(`gh api could not read ${info.url} (need gh auth + network; first 403/429 ends this host)`);
  const dir = join(ROOT, "target", "steals", String(id));
  mkdirSync(dir, { recursive: true });
  const ext = extname(info.path) || ".txt";
  writeFileSync(join(dir, `donor${ext}`), body);
  const verdict = verdictOf(line.license);
  let lic = null;
  for (const cand of ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING", "COPYING.md"]) {
    lic = ghFile(info.owner, info.repo, cand, info.commit);
    if (lic) break;
  }
  const head = verdict === "PORT"
    ? `PORT: donor license is "${line.license}" (off the allowlist); porting allowed (owner 2026-10-10), donor ${info.owner}/${info.repo}@${info.commit}.\n`
    : `License for steal #${id} (${verdict}), donor ${info.owner}/${info.repo}@${info.commit}.\n`;
  writeFileSync(join(dir, "LICENSE"), Buffer.concat([Buffer.from(head), lic ?? Buffer.from("(license file not found at that commit)\n")]));
  writeFileSync(join(dir, "fetch.json"), `${JSON.stringify({ id, url: info.url, license: line.license, verdict, our: line.our }, null, 2)}\n`);
  console.log(`steal-land: line #${id} -> ${join("target", "steals", String(id), `donor${ext}`)} + LICENSE [${verdict}]`);
}

function cmdGate() {
  const lines = loadSteals();
  const results = gateLines(lines, { commitTouches: gitCommitTouches });
  const byId = new Map(results.map((r) => [r.id, r]));
  let bad = 0;
  for (const l of lines) {
    const r = byId.get(l.id);
    // A note (a landed sha this clone cannot resolve) rides on the line it
    // explains and never changes the PASS/FAIL words.
    const note = r.notes.length ? ` (${r.notes.join("; ")})` : "";
    if (r.ok) console.log(`[PASS] #${l.id} ${l.date} ${l.bar} ${l.our} ${l.status}${note}`);
    else {
      bad += 1;
      for (const reason of r.reasons) console.log(`[FAIL] #${l.id} ${l.date} ${l.bar} ${l.our} ${l.status}: ${reason}`);
      if (note) console.log(`  #${l.id}${note}`);
    }
  }
  const open = lines.filter((l) => l.status === "open").length;
  const landed = lines.filter((l) => l.status === "landed").length;
  console.log(`${open} open, ${landed} landed, ${lines.length} ever filed`);
  console.log(bad ? `RESULT FAIL: ${bad} line(s) would FAIL` : "RESULT PASS: every line lands clean");
  if (bad) process.exitCode = 1;
}

function main(argv) {
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write(HELP);
    return;
  }
  const opt = (k) => {
    const i = argv.indexOf(k);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  if (argv.includes("--gate")) {
    cmdGate();
    return;
  }
  const next = opt("--next");
  if (next !== undefined) {
    cmdNext(next);
    return;
  }
  const fetch = opt("--fetch");
  if (fetch !== undefined) {
    cmdFetch(fetch);
    return;
  }
  process.stderr.write(`steal-land: error: need --next <part>, --fetch <id> or --gate\n\n${HELP}`);
  process.exit(2);
}

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) main(cliArgs());
