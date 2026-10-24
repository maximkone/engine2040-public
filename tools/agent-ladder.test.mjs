// Run: node --test tools/agent-ladder.test.mjs
// agent-ladder pin: the tape shape it compares, the drift and verdict reads,
// and the rebuild line it prints. The ladder itself is proven by the tests in
// crates/endpoint/src/e2e_ladder.rs; this file proves the comparison.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BUILD_LINE,
  FIELDS,
  MAGIC,
  compareTapes,
  extractTape,
  parseTape,
  verdictRows,
} from "./agent-ladder.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const LADDER_RS = join(ROOT, "crates", "endpoint", "src", "e2e_ladder.rs");

/// One tape block, in the shape `e2040-ladder` prints: header, then one row
/// per rung over the six rungs of the shipped ladder.
const row = (id, name, gate, calls, tokens, transcript, refused, check) =>
  [id, name, gate, calls, tokens, transcript, refused, check].join("\t");
const TAPE = `${[
  `${MAGIC}\t1`,
  row("E0", "door_alive", "pass", "1", "40", "d754e12fc924374a", "-", "-"),
  row("E1", "player_walks", "pass", "2", "80", "abb4a11c34fa86db", "-", "-"),
  row("E2", "two_fighters_trade", "pass", "1", "40", "54a59f59addebef0", "-", "-"),
  row("E3", "parry_lands", "pass", "1", "40", "1febe92f8d6d6f3a", "-", "-"),
  row("E4", "camp_work_builds", "pass", "1", "40", "8b32a202ec3fbe0b", "-", "-"),
  row("E5", "slice_runs", "pass", "1", "40", "d823548459a4076d", "-", "-"),
].join("\n")}\n`;

const TABLE = [
  "rung name             gate calls tokens transcript       refused verb:code  missed check",
  "E0  door_alive        pass    1     40 d754e12fc924374a",
  "",
  "6 of 6 rungs pass, 0 failed; 7 door calls, 2296 est tokens",
  "wall ms per rung: E0=12 E1=31 E2=18 E3=17 E4=44 E5=90",
  "tape (the compared artifact, no wall clock):",
].join("\n");

test("agent-ladder: the magic, version and columns match the Rust ladder", () => {
  const rust = readFileSync(LADDER_RS, "utf8");
  const magic = /pub const LADDER_MAGIC: &str = "([^"]+)"/.exec(rust);
  assert.ok(magic, "no LADDER_MAGIC in the ladder source");
  assert.equal(MAGIC, magic[1], "the tool compares a different tape than the ladder writes");
  const version = /pub const LADDER_VERSION: u32 = (\d+)/.exec(rust);
  assert.ok(version, "no LADDER_VERSION in the ladder source");
  assert.ok(TAPE.startsWith(`${magic[1]}\t${version[1]}\n`), "the fixture header is not the shipped one");
  // The row format string is the one place the column count lives; a new
  // column there must show up here before the tool reads a shifted tape.
  const rowLine = rust.split("\n").find((line) => line.includes("{refused}") && line.includes("{gate}"));
  assert.ok(rowLine, "the ladder's tape row format changed shape");
  assert.equal((rowLine.match(/\\t/g) ?? []).length + 1, FIELDS.length, "the tool reads a different column count");
});

test("agent-ladder: extractTape cuts the block out of the operator table", () => {
  assert.equal(extractTape(`${TABLE}\n${TAPE}\n# ${MAGIC} v1\n`), TAPE);
  assert.equal(extractTape(TAPE), TAPE, "--tape prints the block alone");
  assert.equal(extractTape("e2040-ladder: unknown flag `--nope`\n"), "");
});

test("agent-ladder: parseTape reads the version and one record per rung", () => {
  const tape = parseTape(TAPE);
  assert.equal(tape.version, "1");
  assert.deepEqual(tape.broken, []);
  assert.equal(tape.rows.length, 6);
  assert.equal(tape.rows[2].name, "two_fighters_trade");
  assert.equal(tape.rows[2].calls, "1");
});

test("agent-ladder: parseTape reports a wrong-shaped row instead of guessing", () => {
  const short = `${MAGIC}\t1\nE0\tdoor_alive\tpass\t1\t40\tdeadbeef\n`;
  const tape = parseTape(short);
  assert.deepEqual(tape.rows, []);
  assert.equal(tape.broken.length, 1);
  assert.match(tape.broken[0], /6 fields, not 8/);
  assert.deepEqual(parseTape(""), { version: null, rows: [], broken: ["empty tape"] });
});

test("agent-ladder: the repeat check agrees on the same tape", () => {
  assert.deepEqual(compareTapes(parseTape(TAPE), parseTape(TAPE)), []);
});

test("agent-ladder: a moved measurement names the rung and its column", () => {
  const moved = TAPE.replace("54a59f59addebef0", "0000000000000000").replace(
    "E3\tparry_lands\tpass",
    "E3\tparry_lands\tfail",
  );
  assert.deepEqual(compareTapes(parseTape(TAPE), parseTape(moved)), [
    "E2: transcript 54a59f59addebef0 -> 0000000000000000",
    "E3: gate pass -> fail",
  ]);
});

test("agent-ladder: a second run that gains or loses a rung is reported", () => {
  const lost = `${TAPE.split("\n").slice(0, 3).join("\n")}\n`;
  const misses = compareTapes(parseTape(TAPE), parseTape(lost));
  assert.deepEqual(misses.map((m) => m.split(": ")[0]).sort(), ["E2", "E3", "E4", "E5"]);
  for (const miss of misses) assert.match(miss, /the second run lost the rung$/);
  const gained = compareTapes(parseTape(lost), parseTape(TAPE));
  for (const miss of gained) assert.match(miss, /the second run added a rung$/);
});

test("agent-ladder: a pass prints its counts, a fail its reason", () => {
  const lines = verdictRows(parseTape(TAPE).rows);
  assert.equal(lines[0], "pass E0 door_alive (1 calls, 40 est tokens)");

  const refused = `${MAGIC}\t1\n${row("E1", "player_walks", "fail", "0", "0", "-", "sim.step:bad_json", "-")}\n`;
  assert.deepEqual(verdictRows(parseTape(refused).rows), [
    "FAIL E1 player_walks: refused sim.step:bad_json",
  ]);

  const missed = `${MAGIC}\t1\n${row("E2", "two_fighters_trade", "fail", "1", "40", "abc", "-", "events.*.kind == hit")}\n`;
  assert.deepEqual(verdictRows(parseTape(missed).rows), [
    "FAIL E2 two_fighters_trade: missed events.*.kind == hit",
  ]);
});

test("agent-ladder: the rebuild line names the endpoint ladder binary", () => {
  assert.match(BUILD_LINE, /^cargo build -p engine-endpoint --features mcp --bin e2040-ladder$/);
  const manifest = readFileSync(join(ROOT, "crates", "endpoint", "Cargo.toml"), "utf8");
  assert.match(manifest, /name = "e2040-ladder"/, "the binary the tool runs is not in the manifest");
});
