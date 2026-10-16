// Run: node --test tools/steal-next.test.mjs
// steal-next coldest-first picker: futures table scores, stalest Parts
// row, and the CLI on the real repo (help, text, json, usage errors).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { coldestTopics, parseFuturesTopics, parseVisionParts, stalestPart } from "./steal-next.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const TOOL = join(HERE, "steal-next.mjs");

const FUTURES_SNIP = [
  "| Capability | Status | Evidence |",
  "|---|---|---|",
  "| Audio middleware class | GAP: 0 plan files, 0 research | `crates/audio/src/dsp.rs:1` |",
  "| Facial animation | COLD: 2 plan files, 0 research | `crates/anim/src/lipsync.rs:1` |",
  "| Texture transcoding | OK code, no FU row: 4 research, 4 plan, 3 code files | `crates/assets/src/bc.rs:1` |",
  "",
].join("\n");

const PARTS_SNIP = [
  "| Part | Consumer | Ours | Best | They beat | Steal next | Proof | Swept |",
  "|---|---|---|---|---|---|---|---|",
  "| Look (render) | window | est. 5% | Unreal | GPU cost | wgpu timestamps | Q1 | 2026-10-01 |",
  "| Online (net) | LAN | est. 10% | ggrs | replication | confirmed-frame discard | ON4 | 2026-09-29 |",
  "",
].join("\n");

function run(args, cwd = ROOT) {
  const r = spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", cwd });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

test("futures GAP 0 scores coldest, then COLD, then OK", () => {
  const topics = parseFuturesTopics(FUTURES_SNIP);
  assert.equal(topics.length, 3);
  assert.equal(topics[0].score, 0);
  assert.equal(topics[1].score, 2);
  assert.equal(topics[2].score, 8);
  const cold = coldestTopics(FUTURES_SNIP, 1);
  assert.equal(cold.length, 1);
  assert.match(cold[0].capability, /Audio middleware/);
});

test("futures --top 2 keeps coldest order", () => {
  const cold = coldestTopics(FUTURES_SNIP, 2);
  assert.equal(cold.length, 2);
  assert.match(cold[0].capability, /Audio middleware/);
  assert.match(cold[1].capability, /Facial animation/);
});

test("parts stalest row is the oldest Swept date", () => {
  const rows = parseVisionParts(PARTS_SNIP);
  assert.equal(rows.length, 2);
  const old = stalestPart(PARTS_SNIP);
  assert.match(old.part, /Online/);
  assert.equal(old.swept, "2026-09-29");
  assert.match(old.stealNext, /confirmed-frame discard/);
});

test("empty tables give no topics and no stalest part", () => {
  assert.deepEqual(parseFuturesTopics("no table here\n"), []);
  assert.deepEqual(coldestTopics("no table here\n"), []);
  assert.equal(stalestPart("no table here\n"), null);
});

test("CLI on the real repo names a coldest topic and a stalest part", () => {
  const r = run([]);
  assert.equal(r.code, 0);
  assert.match(r.out, /coldest-topic: /);
  assert.match(r.out, /stalest-part: /);
  assert.match(r.out, /steal-next: /);
});

test("CLI --json parses and --top 2 prints two coldest lines", () => {
  const j = run(["--json"]);
  assert.equal(j.code, 0);
  const body = JSON.parse(j.out);
  assert.ok(Array.isArray(body.coldest) && body.coldest.length >= 1);
  assert.ok(body.stalest && body.stalest.swept);
  const t = run(["--top", "2"]);
  assert.equal(t.code, 0);
  assert.equal((t.out.match(/coldest-topic: /g) ?? []).length, 2);
});

test("CLI --help exits 0; unknown flag and bad --top exit 2", () => {
  assert.equal(run(["--help"]).code, 0);
  assert.match(run(["--help"]).out, /--top/);
  assert.equal(run(["--bogus"]).code, 2);
  assert.equal(run(["--top", "0"]).code, 2);
});
