// Build farm client (Maxim 2026-10-16, "wire it for 40+ easy fast"): the public mirror's GitHub runners run the heavy
// checks (fmt, ratchet, deny, four test groups, clippy, release proofs, Windows window build, ARM golden) in parallel,
// so a sprint PC compiles only the crate it edits and asks the farm for the rest.
//   node tools/farm.mjs                 export HEAD to the mirror, wait for its CI, print every red job with its error lines
//   node tools/farm.mjs --no-export     only wait for and report the newest run on the mirror's head
//   node tools/farm.mjs --max 40        minutes to wait (default 30)
// Exit 0 = every job green, 1 = a job is red (lines printed and kept in target/farm-last.txt), 2 = no run or timeout.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const REPO = "maximkone/engine2040-public";
const gh = (args) => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 64 * 1024 * 1024 }).trim();

// One failed-log line from `gh run view --log-failed` is `job<TAB>step<TAB>timestamp message`; keep the lines that name a defect.
export const errorLines = (log) => {
  const by = new Map();
  for (const raw of log.split("\n")) {
    const parts = raw.split("\t");
    if (parts.length < 3) continue;
    const msg = parts.slice(2).join("\t").replace(/^﻿/, "").replace(/^\S+Z\s/, "").replace(/\x1b\[[0-9;]*m/g, "");
    if (!/(^|\s)error(\[|:)|panicked at|Diff in |test result: FAILED|^failures:|\bFAILED\b|##\[error\]|assertion/i.test(msg)) continue;
    const list = by.get(parts[0]) ?? []; if (list.length < 15) list.push(msg.trim().slice(0, 240)); by.set(parts[0], list);
  }
  return by;
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const main = async () => {
  const args = process.argv.slice(2); const flag = (n) => args.includes(n);
  const max = Number(args[args.indexOf("--max") + 1]) || 30;
  if (!flag("--no-export")) {
    // The exporter holds a lock; a second caller does nothing and says so. Retry until the mirror's newest snapshot names our HEAD.
    const headNow = () => execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
    let head = headNow(), ok = false;
    for (let i = 0; i < 12 && !ok; i++) {
      head = headNow(); const r = spawnSync("node", [join(ROOT, "tools", "public-export.mjs"), "--commit", "--push"], { cwd: ROOT, encoding: "utf8" });
      process.stdout.write(r.stdout ?? "");
      if (r.status) { console.log("export failed: nothing to wait for"); process.exit(2); }
      const msg = gh(["api", `repos/${REPO}/commits/main`, "--jq", ".commit.message"]);
      ok = msg.includes(`of ${head}`);
      if (!ok) { console.log(`mirror head is not ${head} yet (an export was running); retry in 60 s`); await sleep(60_000); }
    }
    if (!ok) { console.log(`the mirror never reached ${head}`); process.exit(2); }
  }
  const sha = gh(["api", `repos/${REPO}/commits/main`, "--jq", ".sha"]);
  const t0 = Date.now(); let run = null;
  while (Date.now() - t0 < max * 60_000) {
    const list = JSON.parse(gh(["run", "list", "-R", REPO, "--commit", sha, "--json", "databaseId,status,conclusion,url", "-L", "1"]) || "[]");
    run = list[0] ?? null;
    if (run?.status === "completed") break;
    await sleep(30_000);
  }
  if (!run || run.status !== "completed") { console.log(`no finished CI run for ${sha.slice(0, 7)} after ${max} min`); process.exit(2); }
  const jobs = JSON.parse(gh(["run", "view", String(run.databaseId), "-R", REPO, "--json", "jobs"])).jobs ?? [];
  const out = [`farm: ${REPO}@${sha.slice(0, 7)} ${run.url}`];
  for (const j of jobs) out.push(`${j.conclusion === "success" ? "PASS" : String(j.conclusion).toUpperCase()} ${j.name}${j.conclusion === "success" ? "" : ` (step: ${(j.steps ?? []).find((s) => s.conclusion === "failure")?.name ?? "?"})`}`);
  const red = jobs.filter((j) => j.conclusion !== "success" && j.conclusion !== "skipped");
  if (red.length) {
    let log = ""; try { log = gh(["run", "view", String(run.databaseId), "-R", REPO, "--log-failed"]); } catch (e) { log = String(e?.stdout ?? ""); }
    for (const [job, lines] of errorLines(log)) { out.push(`--- ${job}`); out.push(...lines); }
  }
  out.push(red.length ? `RESULT RED: ${red.length} job(s)` : "RESULT GREEN");
  mkdirSync(join(ROOT, "target"), { recursive: true }); writeFileSync(join(ROOT, "target", "farm-last.txt"), `${out.join("\n")}\n`);
  console.log(out.join("\n"));
  process.exit(red.length ? 1 : 0);
};
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main().catch((e) => { console.log("FAILED: " + String(e?.message ?? e).split("\n")[0]); process.exit(2); });
