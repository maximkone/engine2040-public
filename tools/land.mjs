// land: commit a coder packet's work once its review said PASS (owner 2026-10-11: overnight the Muse lead landed none
// of the coder diffs; center's auto-backup swept them onto master unjudged, under "auto: backup" subjects).
//   node tools/land.mjs <packet>        land target/queue/done/<packet>.md: its last RESULT is DONE, PARTIAL or NOOP, its latest
//                                       review (done/<packet>-review[-N].md) ends VERDICT: PASS with DEPTH and RUST at least 3
//                                       (or VERDICT: PASS LIGHT - <why>, a small diff's review, which gives no scores), its
//                                       `paths:` have changes, and node tools/workspace-check.mjs
//                                       has no FAIL on the packet's own files (DP1 landed an undocumented endpoint->net edge
//                                       on RESULT text alone; a FAIL on another writer's file only warns, 2026-10-12: one
//                                       unlanded file refused every landing for 35 min). Commits only those paths
//                                       (git add -- <paths>; git commit -F <msg> -- <paths>) and pushes; a rejected push
//                                       stays a local commit that center's auto-push merges and pushes.
//   node tools/land.mjs <packet> --dry  print the plan and the message; change nothing
// The keeper runs it on every review PASS; the lead may run it by hand. A landed record gets a `Landed:` line, so a
// second run is a no-op. Exit 0 landed or nothing to land, 1 refused, 2 usage.
// What counts as a PASS (2026-10-12 flow audit): the run's own last RESULT line says DONE, PARTIAL or NOOP (the reply
// template and lines the prompt quotes do not count), its last VERDICT line is not FAIL/BLOCKED, its RESULT text does not
// say FAIL, and its proof names a command or file result (argv.mjs proofProblem), not a book cite.
// A PARTIAL lands the paths as they stand (the commit says `Result PARTIAL: <what>`; it ticks no board row). A NOOP is an
// edit that was already in the tree and verified: it lands when its paths have changes, else there is nothing to land
// (exit 0). BLOCKED and no RESULT line still refuse (2026-10-12: NOOP and PARTIAL packets bounced forever).
// After a landing: one line `UTC<TAB>row<TAB>packet<TAB>sha<TAB>proof` goes to target/queue/landed.tsv. A packet with
// `closes: <ROW>` also ticks that board row (docs/SPRINT*.md, ` Landed <sha7>.` at the end of its first line) when the
// landing proof holds the row's Accept command (the backticked commands of its `Accept:` clause); the tick is a second
// commit by path (a commit cannot cite its own sha); it is never made for a ticked row (no unticking), a row with no Accept
// command or a proof that does not hold it.
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { proofProblem } from "./argv.mjs";
import { boardShardFiles } from "./board-slice.mjs";
import { frontMatter, parsePathEntries } from "./queue-cap.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLAUDE_CLAIM_MS = 3 * 3_600_000;

const lastOf = (text, re) => [...String(text ?? "").matchAll(re)].at(-1);
const ROW_ID = /\b[A-Z]{1,4}\d+[a-z]?\b/g;

// The review's verdict line and its scores (the last of each: a quoted earlier verdict does not count).
// light: the verdict line is `VERDICT: PASS LIGHT - <why>`, the keeper's review of a small diff, which gives no scores.
export const verdictOf = (text) => {
  // `(?!\|)`: the template's own "VERDICT: PASS|FAIL|BLOCKED" line is no verdict (a review with no reply read as PASS).
  const line = lastOf(text, /^.*VERDICT:\s*\**\s*(?:PASS|FAIL|BLOCKED)\b(?!\|).*$/gim)?.[0]?.trim() ?? "";
  return {
    verdict: lastOf(text, /VERDICT:\s*\**\s*(PASS|FAIL|BLOCKED)\b(?!\|)/gi)?.[1]?.toUpperCase() ?? null,
    line,
    light: /VERDICT:\s*\**\s*PASS\s+LIGHT\b/i.test(line),
    depth: Number(lastOf(text, /DEPTH:\s*(\d)\s*\/\s*5/gi)?.[1] ?? NaN),
    rust: Number(lastOf(text, /RUST:\s*(\d)\s*\/\s*5/gi)?.[1] ?? NaN),
  };
};

// The run's own words: the text after its last `## Result` heading. The prompt above it quotes earlier RESULT and
// VERDICT lines (a repair packet quotes the FAIL it repairs), and those are history, not this run's.
export const resultSection = (record) => {
  const text = String(record ?? "");
  const at = text.lastIndexOf("\n## Result");
  return at < 0 ? text : text.slice(at);
};

// The run's last real RESULT line. The reply template `RESULT: DONE|PARTIAL|BLOCKED|NOOP - <what> | proof: <where>`
// is no result (a run that ended with no reply read as DONE). `what` is the text before `| proof:`, `proof` after it.
const RESULT_LINE = /^.*\bRESULT:\s*(DONE|PARTIAL|BLOCKED|NOOP)\b(?!\|)(?!\s*-?\s*<(?:what|one line|summary|text)\b)(.*)$/gim;
// The keeper closes a run with `## Keeper result (<time>)` and one line; a builder that skips the template ends it
// `DONE - <what> | proof: <line>` with no `RESULT:` word. 2026-10-15: 3 finished orphan packets parked "land REFUSED"
// and 26 more sat held. That first line is the run's result, same status words, same proof checks.
const KEEPER_LINE = /^[\s>*`-]*(DONE|PARTIAL|BLOCKED|NOOP)\b(?!\|)(?:\s*[-:–—]\s*|\s+)(.*)$/;
const keeperResult = (record) => {
  const text = String(record ?? "");
  const at = text.lastIndexOf("\n## Keeper result");
  if (at < 0) return null;
  const first = text.slice(at + 1).split("\n").slice(1).find((l) => l.trim());
  const k = first?.match(KEEPER_LINE);
  return k ? [`RESULT: ${first.trim()}`, k[1], ` - ${k[2]}`] : null;
};
export const resultOf = (record) => {
  const m = lastOf(resultSection(record), RESULT_LINE) ?? keeperResult(record);
  if (!m) return { status: null, line: "", what: "", proof: "" };
  const cut = [...m[2].matchAll(/\|\s*proof:\s*/gi)].at(-1);
  return {
    status: m[1].toUpperCase(),
    line: m[0].replace(/^[\s>*`-]+/, "").trim(),
    what: (cut ? m[2].slice(0, cut.index) : m[2]).replace(/^[\s:*-]+/, "").trim(),
    // Seats may add sibling cells after the proof (`| paths: target/queue/ready/x.md`): those are not the proof.
    proof: cut ? m[2].slice(cut.index + cut[0].length).replace(/\s*\|\s*(?:paths|owned|row|crate|source)\s*:.*$/i, "").replace(/[\s`*]+$/, "") : "",
  };
};

// The run's own last VERDICT line (a judge-style line in a coder's result): uppercase, at a line start. Prose such as
// "the verdict: fail case is tested" is no verdict.
export const ownVerdict = (record) => {
  const m = lastOf(resultSection(record), /^[\s>*`_-]*VERDICT:\s*\**\s*(PASS|FAIL|BLOCKED)\b(?!\|).*$/gm);
  return m ? { verdict: m[1], line: m[0].replace(/^[\s>*`_-]+/, "").trim() } : { verdict: null, line: "" };
};

// The newest review record of a packet: <packet>-review.md, then -review-2.md, and so on.
export const latestReview = (names, packet) => names
  .map((n) => n.match(new RegExp(`^${packet.replace(/[.+]/g, "\\$&")}-review(?:-(\\d+))?\\.md$`)))
  .filter(Boolean).sort((a, b) => Number(a[1] ?? 1) - Number(b[1] ?? 1)).at(-1)?.[0] ?? null;

// Paths a claude-* claim line written in the last 3 h names (Claude coders work beside the loop).
export const claudeClaimed = (claims, now = Date.now()) => {
  const out = new Set();
  for (const line of String(claims ?? "").split(/\r?\n/)) {
    const cells = line.split("|").map((c) => c.trim());
    if (cells.length < 3 || !/^claude-/.test(cells[0]) || !(now - Date.parse(cells[1]) < CLAUDE_CLAIM_MS)) continue;
    for (const tok of cells.slice(2).join(" ").split(/\s+/)) if (/[\\/]/.test(tok)) out.add(tok.replace(/\\/g, "/"));
  }
  return out;
};

// `git status --porcelain=v1 -z` output to repo paths (a rename gives both).
export const changedPaths = (porcelain) => {
  const tok = String(porcelain ?? "").split("\0").filter(Boolean);
  const out = [];
  for (let i = 0; i < tok.length; i++) {
    out.push(tok[i].slice(3).replace(/\\/g, "/"));
    if (/^[RC]/.test(tok[i])) out.push(tok[++i].replace(/\\/g, "/"));
  }
  return out;
};

// The workspace audit before landing: `node tools/workspace-check.mjs` audits edges, modules,
// determinism, ledger, board and LFS. A landing that trusts the packet's RESULT text alone landed
// DP1's undocumented engine-endpoint -> engine-net edge (3a1cd34c). The audit reads the live shared tree, and on
// 2026-10-12 one writer's unlanded replay.rs refused every landing for 35 min. So a FAIL refuses only when it names
// one of the packet's own paths; a FAIL on other files warns. A FAIL line that names no file (an edge, the board,
// a crashed check) or a check that cannot run still refuses.
// The files one FAIL line names: only the line shapes workspace-check.mjs prints are read (any other names none).
// They come relative to crates/ (genre, float math), to a crate's src/ (undeclared module), by bare name (PF11), or as repo paths.
const FAIL_LINE_FILES = [
  [/^[\w.-]+: (\S+\.rs) is not declared by any module/, (m) => [`src/${m[1]}`]],
  [/^(?:platform float math in simulation code|unlisted genre assumption)\b/, (m, line) => [...line.matchAll(/([^\s,()`]+\.rs) \(/g)].map((x) => x[1])],
  [/^large non-LFS blob: (\S+) is /, (m) => [m[1]]],
  [/^test layout: (\S+\.rs) is a new test executable/, (m) => [m[1]]],
  [/^PF11 append-only gate missing from (\S+\.rs):/, (m) => [m[1]]],
];
export const failFiles = (line) => {
  for (const [re, files] of FAIL_LINE_FILES) {
    const m = re.exec(line);
    if (m) return files(m, line);
  }
  return [];
};
const slashed = (p) => String(p).replace(/\\/g, "/").replace(/^(?:\.{1,2}\/)+/, "");
// A named file is the packet's when it is a path or a path tail of one (crates/ + playtest/src/replay.rs).
const inPacket = (file, own) => own.some((p) => p === slashed(file) || p.endsWith(`/${slashed(file)}`));

// Pure: { refusal, warns }. refusal null = land (the reason after "REFUSED <packet>: " otherwise); warns = the files of
// FAIL lines outside the packet. own = the repo paths the landing commits. Mirrors tools/proof.mjs.
export const workspaceVerdict = (audit, own = []) => {
  const err = audit?.error;
  if (err) return { refusal: `workspace-check could not run (${String(err.message ?? err).slice(0, 120)}; see node tools/workspace-check.mjs)`, warns: [] };
  if (audit?.status === 0) return { refusal: null, warns: [] };
  const tail = String(audit?.stderr ?? audit?.stdout ?? "").trim().split(/\r?\n/).filter(Boolean).slice(-3).join(" | ").slice(0, 300);
  const stderr = String(audit?.stderr ?? "");
  const at = stderr.indexOf("FAIL workspace audit: ");
  const lines = at < 0 ? [] : stderr.slice(at + "FAIL workspace audit: ".length).split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  const named = lines.map(failFiles);
  const files = [...new Set(named.flat())];
  const mine = files.filter((f) => inPacket(f, own));
  if (!lines.length || named.some((f) => !f.length) || mine.length) {
    return { refusal: `workspace-check FAIL${mine.length ? ` on this packet's own ${mine.join(", ")}` : ""} (${tail || "see node tools/workspace-check.mjs"})`, warns: [] };
  }
  return { refusal: null, warns: files };
};

// The rust ratchet scoped to the landing packet: `node tools/rust-quality.mjs --owned <paths>`
// runs the full ratchet, fails only on regressions touching owned paths, warns on foreign debt.
// Pure: { refusal, warns }. refusal null = land; warns = foreign regression lines. Owned never warns.
export const ratchetArgs = (own = []) => own.flatMap((p) => ["--owned", p]);
export const ratchetVerdict = (audit, own = []) => {
  const err = audit?.error;
  if (err) return { refusal: `ratchet could not run (${String(err.message ?? err).slice(0, 120)}; see node tools/rust-quality.mjs --owned ${own.join(", ")})`, warns: [] };
  const text = `${audit?.stdout ?? ""}\n${audit?.stderr ?? ""}`;
  const foreign = text.split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith("[WARN] foreign ratchet")).map((l) => l.slice("[WARN] foreign ratchet".length).trim()).filter(Boolean);
  if (audit?.status === 0) return { refusal: null, warns: foreign };
  const tail = text.trim().split(/\r?\n/).filter(Boolean).slice(-3).join(" | ").slice(0, 300);
  const scope = own.length ? ` on owned ${own.join(", ")}` : "";
  return { refusal: `ratchet FAIL${scope} (${tail || `see node tools/rust-quality.mjs --owned ${own.join(", ")}`})`, warns: foreign };
};

// The commit subject: the board row it names, else `cut:` for rot cuts, else `loop:`.
export const subjectOf = (fm) => {
  const ids = String(fm.row ?? "").match(ROW_ID) ?? [];
  const title = String(fm.title ?? "").replace(/\s+/g, " ").trim();
  const lead = ids.length ? ids.join(" ") : /^rot:/i.test(String(fm.row ?? "")) ? "cut" : "loop";
  return `${lead}: ${ids.length ? title.replace(new RegExp(`^(?:${ids.join("|")})\\s*:?\\s*`), "") : title}`.slice(0, 100);
};

// A record's facts: a one-off keeps its packet's front matter; a standing seat's run record is
// `# <title> (@<role>, standing)` plus the reply, whose last `OWNED:` and `ROW:` lines give paths and row.
export const recordFacts = (record) => {
  const parsed = frontMatter(record);
  if (parsed) return parsed.fm;
  const head = String(record ?? "").match(/^# (.+?) \(@([\w-]+), standing\)/m);
  if (!head) return null;
  const cell = (key) => lastOf(record, new RegExp(`\\b${key}:\\s*\`?([^|\\n\`]+)`, "g"))?.[1]?.trim() ?? "";
  return { title: head[1], role: head[2], paths: cell("OWNED"), row: cell("ROW") };
};

// Pure: what to land, or why not. record = the coder's done/ record, review = its latest review record.
export const planLanding = ({ packet, record, reviewName, review, changed = [], claims = "", now = Date.now() }) => {
  const fm = recordFacts(record);
  if (!fm) return { ok: false, code: 1, why: `done/${packet}.md has no front matter and no standing-run heading` };
  const landed = lastOf(record, /^Landed:\s*(\S+)/gm);
  if (landed) return { ok: false, code: 0, why: `already landed in ${landed[1]}` };
  if (/^(?:judge|runner|perf|walker|porter|rot-hunter|researcher)$/i.test(String(fm.role ?? ""))) return { ok: false, code: 1, why: `role ${fm.role} lands nothing` };
  const res = resultOf(record);
  // PARTIAL and NOOP land too: the review PASS and the changes under the paths decide (2026-10-12: they bounced forever).
  if (!["DONE", "PARTIAL", "NOOP"].includes(res.status)) return { ok: false, code: 1, why: `the run ended ${res.status ?? "with no RESULT line"}, not DONE, PARTIAL or NOOP` };
  // A DONE that carries a FAIL is no PASS (2026-10-12: 12 judge DONE lines said FAIL and counted as DONE).
  const said = ownVerdict(record);
  if (said.verdict && said.verdict !== "PASS") return { ok: false, code: 1, why: `the run's last VERDICT line says ${said.verdict}, so its ${res.status} is not a PASS (${said.line.slice(0, 120)})` };
  // A FAIL that was fixed ("FAIL -> PASS", "0 FAIL") is no FAIL: BLOCKERS 2026-10-15 pose_sq refused twice on "FAIL arrow PASS" wording.
  const failLeft = res.what.replace(/\bFAIL\s*(?:->|=>|→|to)\s*PASS\b/gi, "").replace(/\b(?:0|no|zero)\s+FAIL\b|\bFAIL[:=]?\s*0\b/gi, "");
  if (/\bFAIL\b/.test(failLeft)) return { ok: false, code: 1, why: `the RESULT text says FAIL after ${res.status}, so it is not a PASS (${res.what.slice(0, 120)})` };
  const badProof = proofProblem(res.proof);
  if (badProof) return { ok: false, code: 1, why: `the RESULT proof ${badProof}` };
  // A filter that matched nothing (engine-game window modules without `--features app`) proves nothing (2026-10-11).
  if (/\brunning 0 tests\b|(?<![\d.])0 passed\b/i.test(res.line)) return { ok: false, code: 1, why: "the proof ran 0 tests: add the feature the module needs (engine-game window modules: --features app) and rerun" };
  // Close-only (2026-10-15): the packet closes a row, nothing changed under its paths because the code is already on master (a backup sweep or an
  // earlier landing took it) and the run says DONE or NOOP with a proof. There is no diff to review and no change to land, but the row still has to
  // tick, or the slice cutter queues the same close again: 36 of 36 close packets sampled ended NOOP with the row open, 35 of 118 landed rows in 4 days
  // were still open. A review that says FAIL or BLOCKED stops it; planClose still ticks only when the proof holds every Accept command of the row.
  if (String(fm.closes ?? "").trim() && res.status !== "PARTIAL") {
    const ownC = parsePathEntries(fm.paths).map((e) => e.path).filter((p) => p && !p.startsWith("target/") && !/^[A-Za-z]:|^\/|\.\./.test(p));
    const reviewed = review ? verdictOf(review).verdict : null;
    if (ownC.length && !changed.some((c) => ownC.some((p) => c === p || c.startsWith(`${p}/`))) && !(reviewed && reviewed !== "PASS")) {
      return { ok: false, code: 0, closeOnly: true, why: `nothing to land: no changes under ${ownC.join(", ")} (landed or swept already)`, fm, proof: res.proof, status: res.status };
    }
  }
  if (!reviewName || !review) return { ok: false, code: 1, why: `no review record for ${packet} in done/` };
  const v = verdictOf(review);
  if (v.verdict !== "PASS") return { ok: false, code: 1, why: `${reviewName} says ${v.verdict ?? "no VERDICT"}, not PASS` };
  if (!v.light && !(v.depth >= 3 && v.rust >= 3)) return { ok: false, code: 1, why: `${reviewName} passes with DEPTH ${v.depth}/5 RUST ${v.rust}/5; both must be given and at least 3 (or the verdict is PASS LIGHT - <why>)` };
  const own = parsePathEntries(fm.paths).map((e) => e.path).filter((p) => p && !p.startsWith("target/") && !/^[A-Za-z]:|^\/|\.\./.test(p));
  if (!own.length) return { ok: false, code: 1, why: `done/${packet}.md has no paths: line` };
  const mine = changed.filter((c) => own.some((p) => c === p || c.startsWith(`${p}/`)));
  if (!mine.length) return { ok: false, code: 0, why: `nothing to land: no changes under ${own.join(", ")} (landed or swept already)` };
  const held = claudeClaimed(claims, now);
  const clash = mine.filter((c) => held.has(c));
  if (clash.length) return { ok: false, code: 1, why: `a claude-* claim holds ${clash.join(", ")}` };
  // DONE keeps its own RESULT line; PARTIAL and NOOP name their kind first (the commit says what it is).
  const resultLine = res.status === "DONE" ? res.line : `Result ${res.status}: ${res.what}${res.proof ? ` | proof: ${res.proof}` : ""}`;
  const scores = [Number.isFinite(v.depth) ? `DEPTH ${v.depth}/5` : "", Number.isFinite(v.rust) ? `RUST ${v.rust}/5` : ""].filter(Boolean).join(", ");
  const message = [
    subjectOf(fm), "",
    resultLine.slice(0, 600),
    `Review ${reviewName.replace(/\.md$/, "")}: ${v.line.replace(/^[\s>*`-]+/, "").slice(0, 300)}${scores ? ` (${scores})` : ""}`,
    `Paths: ${mine.join(", ")}`,
    `Landed by tools/land.mjs after the review PASS (loop packet ${packet}).`, "",
  ].join("\n");
  return { ok: true, code: 0, paths: mine, message, fm, proof: res.proof, status: res.status };
};

// ---- after a landing: the landed.tsv line, and the board row a `closes:` packet finishes ----
export const LANDED_HEADER = "utc\trow\tpacket\tsha\tproof";
const tsvCell = (v, max) => String(v ?? "").replace(/[\t\r\n]+/g, " ").trim().slice(0, max);
export const landedLine = ({ at, row, packet, sha, proof }) => [at, row || "-", packet, sha, proof].map((v, i) => tsvCell(v, i === 4 ? 400 : 120)).join("\t");

// A board row names its acceptance in an `Accept:` clause (it runs to F2P:, P2P:, Diff:, Tier:, Size:, needs:); the
// commands are its backticked spans that start with a command word. Paths and identifiers in backticks are no command.
const COMMAND_SPAN = /^[(!\s]*(?:cargo|node|rg|git|pwsh|python3?|npm|Select-String|Test-Path|Get-[A-Z]\w*)\b/i;
export const acceptCommands = (rowText) => {
  const text = String(rowText ?? "").replace(/\s+/g, " ");
  const at = text.search(/\bAccept:/);
  if (at < 0) return [];
  const clause = text.slice(at + "Accept:".length).split(/\b(?:F2P|P2P|Diff|Tier|Size|needs|Needs|BLOCKED|LATER):/)[0];
  return [...clause.matchAll(/`([^`]+)`/g)].map((m) => m[1].trim()).filter((c) => COMMAND_SPAN.test(c));
};

const squash = (s) => String(s ?? "").replace(/\\/g, "/").replace(/\s+/g, " ").trim().toLowerCase();

// Pure: tick the one row a packet closes. boards = [{ file, text }] in board order (index, then shards). null = the
// packet closes nothing. { ok: true, file, text } = the board file with `- [ ] ID` ticked and ` Landed <sha7>.` at the end
// of the row's first line; { ok: false, why } = left alone. A ticked row is never touched; the first file that holds the
// id decides; every Accept command of the row must stand in the landing proof (equal to it or contained in it).
// A row with no Accept command in backticks (14 of 25 close-only packets sampled 2026-10-15) could never tick: its close packet ended NOOP and the
// cutter queued it again. Then the packet's own `test:` command stands in for it: the proof must hold that command.
export const planClose = ({ closes, proof, sha, boards, test = "" }) => {
  const id = String(closes ?? "").trim();
  if (!id) return null;
  if (!/^[A-Z][A-Z0-9]*\d[a-z]?$/.test(id)) return { ok: false, why: `closes: "${id}" is not one row id` };
  const head = new RegExp(`^- \\[( |x)\\] ${id}\\b`);
  for (const board of boards) {
    const lines = String(board.text).split("\n");
    const at = lines.findIndex((l) => head.test(l));
    if (at < 0) continue;
    if (head.exec(lines[at])[1] === "x") return { ok: false, why: `${id} is already ticked` };
    let end = at + 1;
    while (end < lines.length && /^\s+\S/.test(lines[end])) end++;
    let commands = acceptCommands(lines.slice(at, end).join(" "));
    if (!commands.length && String(test ?? "").trim()) commands = [String(test).trim()];
    if (!commands.length) return { ok: false, why: `${id} has no Accept command in backticks` };
    const have = squash(proof);
    const miss = commands.find((c) => !have.includes(squash(c)));
    if (miss) return { ok: false, why: `the proof does not hold ${id}'s Accept command: ${miss}` };
    const crlf = lines[at].endsWith("\r");
    lines[at] = `${lines[at].replace(/\r$/, "").replace("- [ ]", "- [x]")} Landed ${sha}.${crlf ? "\r" : ""}`;
    return { ok: true, file: board.file, text: lines.join("\n"), id, accept: commands };
  }
  return { ok: false, why: `${id} is not a row on the board` };
};

// Another writer's commit holds .git/index.lock while its hooks run (30 s and more): wait it out for about 90 s
// (30 tries, 3 s apart) instead of 10 s, so a landing is not given up for a lock that frees itself (center 2026-10-12).
const git = (args, opt = {}) => {
  for (let i = 0; ; i++) {
    try { return execFileSync("git", args, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 120_000, ...opt }); } catch (e) {
      const err = String(e.stderr ?? e.message ?? "");
      if (i < 30 && /index\.lock|unable to lock/i.test(err)) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 3000); continue; }
      throw Object.assign(new Error(err.trim().split(/\r?\n/).slice(-2).join(" ") || String(e.message)), { stderr: err });
    }
  }
};

// The ledger of landings: a header, then one `landedLine` per landing. `wx` makes the header race-safe.
const recordLanded = (file, line) => {
  mkdirSync(dirname(file), { recursive: true });
  try { writeFileSync(file, `${LANDED_HEADER}\n`, { flag: "wx" }); } catch { /* it exists */ }
  appendFileSync(file, `${line}\n`);
};

// Tick the row a landed packet closes. Returns the words for the LANDED line, "" when the packet closes nothing.
// The board file is read fresh and only its one row line changes. A file that carried other uncommitted edits is
// ticked in the tree but not committed: a by-path commit would sweep those edits in.
const closeRow = ({ fm, proof, sha, packet, closeOnly = false }) => {
  const files = [join(ROOT, "docs", "SPRINT.md"), ...boardShardFiles(ROOT)].filter((f) => existsSync(f));
  const boards = files.map((f) => ({ file: relative(ROOT, f).replace(/\\/g, "/"), text: readFileSync(f, "utf8") }));
  const plan = planClose({ closes: fm.closes, proof, sha, boards, test: fm.test });
  if (!plan) return "";
  if (!plan.ok) return `not ticked: ${plan.why}`;
  const clean = git(["status", "--porcelain=v1", "--", plan.file]).trim() === "";
  writeFileSync(join(ROOT, plan.file), plan.text);
  if (!clean) return `ticked ${plan.id} in ${plan.file} (not committed: the file has other edits)`;
  const tmp = mkdtempSync(join(tmpdir(), "land-"));
  const msgFile = join(tmp, "msg.txt");
  writeFileSync(msgFile, [
    closeOnly ? `${plan.id}: board tick, code already on master at ${sha}` : `${plan.id}: board tick, landed ${sha}`, "",
    `Row ${plan.id} ticked on ${plan.file}: the landing proof holds its Accept command (${plan.accept.join("; ").slice(0, 300)}).`,
    `Proof: ${proof.slice(0, 300)}`,
    closeOnly ? `Ticked by tools/land.mjs close-only: loop packet ${packet} changed nothing (the code was already in the tree) and its proof holds the Accept command.` : `Ticked by tools/land.mjs after the landing of loop packet ${packet}.`, "",
  ].join("\n"));
  try {
    git(["add", "--", plan.file]);
    git(["commit", "-q", "-F", msgFile, "--", plan.file]);
    return `ticked ${plan.id} in ${plan.file}`;
  } catch (e) { return `ticked ${plan.id} in ${plan.file} (commit failed: ${String(e.message).slice(0, 120)}; the edit stays in the tree)`; } finally { rmSync(tmp, { recursive: true, force: true }); }
};

const main = (argv) => {
  const packet = argv.find((a) => !a.startsWith("--"))?.replace(/\.md$/, "");
  const dry = argv.includes("--dry");
  if (!packet || !/^[\w.+-]+$/.test(packet)) { console.log("usage: node tools/land.mjs <packet> [--dry]"); return 2; }
  const done = join(ROOT, "target", "queue", "done");
  const recordFile = join(done, `${packet}.md`);
  if (!existsSync(recordFile)) { console.log(`REFUSED ${packet}: no target/queue/done/${packet}.md`); return 1; }
  const record = readFileSync(recordFile, "utf8");
  const reviewName = latestReview(readdirSync(done), packet);
  const review = reviewName ? readFileSync(join(done, reviewName), "utf8") : null;
  const branch = git(["rev-parse", "--abbrev-ref", "HEAD"]).trim();
  if (branch !== "master") { console.log(`REFUSED ${packet}: the tree is on ${branch}, not master`); return 1; }
  const own = parsePathEntries(recordFacts(record)?.paths).map((e) => e.path).filter(Boolean);
  const changed = own.length ? changedPaths(git(["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", ...own])) : [];
  const claimsFile = join(ROOT, "target", "queue", "claims.txt");
  const plan = planLanding({ packet, record, reviewName, review, changed, claims: existsSync(claimsFile) ? readFileSync(claimsFile, "utf8") : "" });
  if (!plan.ok && plan.closeOnly) {
    if (dry) { console.log(`DRY ${packet}: close-only, would tick ${plan.fm.closes} if the proof holds its Accept command`); return 0; }
    let said = "";
    try { said = closeRow({ fm: plan.fm, proof: plan.proof, sha: git(["rev-parse", "--short", "HEAD"]).trim(), packet, closeOnly: true }); } catch (e) { said = `not ticked: ${String(e.message).slice(0, 120)}`; }
    console.log(`NOOP ${packet}: ${plan.why}${said ? ` | board: ${said}` : ""}`);
    return 0;
  }
  if (!plan.ok) { console.log(`${plan.code ? "REFUSED" : "NOOP"} ${packet}: ${plan.why}`); return plan.code; }
  if (dry) { console.log(`DRY ${packet}: would commit ${plan.paths.join(", ")}\n\n${plan.message}`); return 0; }
  // The workspace audit runs on the live tree; a FAIL on this packet's paths refuses the landing, a FAIL on
  // other files warns (2026-10-12). The isolated land.test.mjs harness has no workspace-check.mjs unless a
  // test writes one, so a missing script skips there; the real tree always has it.
  const auditPath = join(ROOT, "tools", "workspace-check.mjs");
  let outside = [];
  if (existsSync(auditPath)) {
    let verdict;
    try {
      verdict = workspaceVerdict(spawnSync(process.execPath, [auditPath], { encoding: "utf8", timeout: 180_000 }), plan.paths);
    } catch (e) {
      verdict = { refusal: `workspace-check could not run (${String(e?.message ?? e).slice(0, 120)})`, warns: [] };
    }
    if (verdict.refusal) { console.log(`REFUSED ${packet}: ${verdict.refusal}`); return 1; }
    outside = verdict.warns;
    if (outside.length) console.log(`WARN ${packet}: workspace-check FAIL outside this packet: ${outside.join(", ").slice(0, 400)}; landing anyway`);
  }
  // The rust ratchet runs scoped to the landing packet's owned paths: foreign staged debt warns,
  // an owned regression refuses. A missing script skips (the e2e harness has none unless a test writes one).
  const ratchetPath = join(ROOT, "tools", "rust-quality.mjs");
  let ratchetForeign = [];
  if (existsSync(ratchetPath) && plan.paths.length) {
    let verdict;
    try {
      verdict = ratchetVerdict(spawnSync(process.execPath, [ratchetPath, ...ratchetArgs(plan.paths)], { encoding: "utf8", timeout: 180_000 }), plan.paths);
    } catch (e) {
      verdict = { refusal: `ratchet could not run (${String(e?.message ?? e).slice(0, 120)})`, warns: [] };
    }
    if (verdict.refusal) { console.log(`REFUSED ${packet}: ${verdict.refusal}`); return 1; }
    ratchetForeign = verdict.warns;
    if (ratchetForeign.length) console.log(`WARN ${packet}: ratchet foreign debt outside this packet: ${ratchetForeign.join(" | ").slice(0, 400)}; landing anyway`);
  }
  const tmp = mkdtempSync(join(tmpdir(), "land-"));
  const msgFile = join(tmp, "msg.txt");
  writeFileSync(msgFile, plan.message);
  try {
    const existing = plan.paths.filter((p) => existsSync(join(ROOT, p)));
    const missing = plan.paths.filter((p) => !existsSync(join(ROOT, p)));
    if (existing.length) git(["add", "--", ...existing]);
    for (const p of missing) {
      try { git(["rm", "-q", "--", p]); } catch (e) { if (!/did not match|pathspec/i.test(String(e.message ?? ""))) throw e; }
    }
    git(["commit", "-q", "-F", msgFile, "--", ...plan.paths]);
  } catch (e) { console.log(`REFUSED ${packet}: git commit failed: ${e.message}`); return 1; } finally { rmSync(tmp, { recursive: true, force: true }); }
  const sha = git(["rev-parse", "--short", "HEAD"]).trim();
  appendFileSync(recordFile, `\nLanded: ${sha} ${new Date().toISOString()} (tools/land.mjs, ${plan.paths.length} path${plan.paths.length === 1 ? "" : "s"})\n`);
  try { recordLanded(join(ROOT, "target", "queue", "landed.tsv"), landedLine({ at: new Date().toISOString().replace(/\.\d+Z$/, "Z"), row: plan.fm.row, packet, sha, proof: plan.proof })); } catch { /* the landing stands without its ledger line */ }
  let board = "";
  // A PARTIAL leaves its row open.
  try { board = plan.status === "PARTIAL" ? "" : closeRow({ fm: plan.fm, proof: plan.proof, sha: sha.slice(0, 7), packet }); } catch (e) { board = `not ticked (${String(e.message).slice(0, 120)})`; }
  let pushed = "no";
  for (let i = 0; i < 2 && pushed === "no"; i++) {
    try { git(["push", "-q", "origin", "master"], { timeout: 180_000 }); pushed = "yes"; } catch (e) { pushed = i ? `no (${e.message.slice(0, 160)}); center's auto-push merges and pushes it` : "no"; }
  }
  // The keeper logs only the last line, so the warning rides on it too.
  const warnNote = [...(outside.length ? ["workspace-check FAIL outside this packet"] : []), ...(ratchetForeign.length ? ["ratchet foreign debt outside this packet"] : [])].join(" + ");
  console.log(`LANDED ${packet}: ${sha} ${plan.paths.join(", ")}${board ? ` | board: ${board}` : ""} | pushed: ${pushed}${warnNote ? ` | warn: ${warnNote}` : ""}`);
  return 0;
};

if (import.meta.main) process.exitCode = main(process.argv.slice(2));
