// tools/clean.mjs drills: the loop's cleaner (sprint-loop.mjs --auto) must
// report trash without touching live state. Each drill builds a fixture tree,
// runs tools/clean.mjs with cwd pointed at it, and asserts the fixture is
// byte-unchanged where the law requires it. The mutating drills point the
// quarantine at a temp root (E2040_QUARANTINE_ROOT), never at C:\_quarantine.
//   node --test tools/clean.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const REPO = process.cwd();

const fixture = () => {
  const dir = join(tmpdir(), `clean-drill-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "target", "tmp"), { recursive: true });
  mkdirSync(join(dir, "target", "lanes", "old-build"), { recursive: true });
  writeFileSync(join(dir, "target", "tmp", "scratch.bin"), "scratch\n");
  writeFileSync(join(dir, "target", "canon-fail.log"), "fail\n");
  writeFileSync(join(dir, "target", "keep.txt"), "keep\n");
  return dir;
};

const clean = (dir, args, env = {}) => {
  const r = spawnSync(process.execPath, [join(REPO, "tools", "clean.mjs"), ...args], {
    encoding: "utf8", cwd: dir,
    env: { ...process.env, E2040_CLEAN_TASKLIST: "", ...env },
  });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

test("dry-run reports trash and old lanes but deletes nothing", () => {
  const dir = fixture();
  try {
    const r = clean(dir, ["--dry-run"]);
    assert.equal(r.status, 0, `exit 0 on report-only, got: ${r.out}`);
    assert.match(r.out, /WOULD target[\\/]tmp/, "trash dir listed");
    assert.match(r.out, /WOULD reclaim/, "reclaim summary printed");
    assert.match(r.out, /SKIP pools: lanes-worktrees\.json unreadable/, "unknown pools fail honest");
    assert.ok(existsSync(join(dir, "target", "tmp", "scratch.bin")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "canon-fail.log")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "lanes", "old-build")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "keep.txt")), "non-trash survives");
    assert.ok(!r.out.includes("keep.txt"), "non-trash never listed");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("unknown flag exits 2 with usage and deletes nothing", () => {
  const dir = fixture();
  try {
    const r = clean(dir, ["--bogus"]);
    assert.equal(r.status, 2, `exit 2 on usage, got: ${r.out}`);
    assert.match(r.out, /Usage: node tools\/clean\.mjs/, "usage printed");
    assert.ok(existsSync(join(dir, "target", "tmp", "scratch.bin")), "usage error deletes nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("empty target reports zero paths", () => {
  const dir = join(tmpdir(), `clean-drill-empty-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "target"), { recursive: true });
  try {
    const r = clean(dir, ["--dry-run"]);
    assert.equal(r.status, 0, `exit 0 on empty report, got: ${r.out}`);
    assert.match(r.out, /across 0 path\(s\)/, "zero paths reported");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("wave-1: no silent empty catch in tools/clean.mjs", async () => {
  const { readFileSync } = await import("node:fs");
  const src = readFileSync(join(REPO, "tools", "clean.mjs"), "utf8");
  const parts = src.split("catch").slice(1);
  assert.ok(parts.length > 0, "expected catch blocks");
  for (const p of parts) {
    assert.ok(p.slice(0, 200).includes("console.warn"), "every catch warns");
  }
  const flat = src.replace(/\s+/g, " ");
  assert.ok(!flat.includes("catch (err) { }"), "no empty catch");
  assert.ok(!flat.includes("catch { }"), "no empty bare catch");
});

// The owner rule (AGENTS.md): nothing is hard-deleted. A removed path is moved
// to C:\_quarantine\<date>\ and one MOVES.tsv line records the move. These
// drills point the quarantine at a temp root, so the real one is untouched.

// A fixture with a target/debug cache, plus a temp quarantine root.
const debugFixture = () => {
  const base = join(tmpdir(), `clean-drill-dbg-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  const dir = join(base, "repo");
  const quarantine = join(base, "quarantine");
  mkdirSync(join(dir, "target", "debug", "deps"), { recursive: true });
  mkdirSync(join(dir, "target", "tmp"), { recursive: true });
  writeFileSync(join(dir, "target", "debug", "deps", "libcore.rlib"), "rlib\n");
  writeFileSync(join(dir, "target", "tmp", "scratch.bin"), "scratch\n");
  writeFileSync(join(dir, "target", "keep.txt"), "keep\n");
  return { base, dir, quarantine };
};

// No cargo and no rustc: the canned list is empty.
const NO_BUILD = "";

test("--auto --include-debug moves target/debug to the quarantine and writes a MOVES.tsv line", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const r = clean(dir, ["--auto", "--include-debug"], {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
      E2040_CLEAN_TASKLIST: NO_BUILD,
    });
    assert.equal(r.status, 0, `exit 0 on the move, got: ${r.out}`);
    assert.ok(!existsSync(join(dir, "target", "debug")), "target/debug left the tree");
    const moved = join(quarantine, "2026-10-22", "target-debug");
    assert.ok(existsSync(join(moved, "deps", "libcore.rlib")), "the bytes are in the quarantine, not deleted");
    assert.ok(existsSync(join(dir, "target", "keep.txt")), "non-trash survives");
    const moves = readFileSync(join(quarantine, "2026-10-22", "MOVES.tsv"), "utf8");
    const row = moves.split("\n").find((l) => l.includes("target\\debug"));
    assert.ok(row, `MOVES.tsv names the moved path: ${moves}`);
    const [date, from, to, ...rest] = row.split("\t");
    assert.equal(date, "2026-10-22", "the row carries the date");
    assert.ok(from.endsWith(join("target", "debug")), "the row names the old path");
    assert.ok(to.startsWith(join(quarantine, "2026-10-22")), "the row names the new path");
    assert.ok(rest.join("\t").length > 0, "the row carries a reason");
    assert.match(r.out, /MOVES\.tsv/, "the tool names the MOVES.tsv file");
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("the tool never hard-deletes: no rmSync of a planned path", () => {
  const src = readFileSync(join(REPO, "tools", "clean.mjs"), "utf8");
  assert.ok(!/rmSync/.test(src), "tools/clean.mjs must not delete anything");
});

test("a running cargo build makes the tool refuse and change nothing", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const r = clean(dir, ["--auto", "--include-debug"], {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
      E2040_CLEAN_TASKLIST: '"cargo.exe","1234","Console","1","10,000 K"\n"rustc.exe","5678","Console","1","10,000 K"',
    });
    assert.equal(r.status, 3, `exit 3 while a build runs, got: ${r.out}`);
    assert.match(r.out, /REFUSE/, "the refusal is loud");
    assert.ok(existsSync(join(dir, "target", "debug", "deps", "libcore.rlib")), "the build dir is untouched");
    assert.ok(!existsSync(join(quarantine, "2026-10-22")), "nothing was moved");
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("a dry run moves nothing and lists no target/debug entry", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const r = clean(dir, ["--dry-run"], {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
    });
    assert.equal(r.status, 0, `exit 0 on report-only, got: ${r.out}`);
    // The row's Accept: the plain dry run lists no target/debug entry, because
    // the debug cache only enters the plan with --include-debug.
    assert.ok(!/target[\\/]debug/.test(r.out), `no target/debug entry: ${r.out}`);
    assert.ok(existsSync(join(dir, "target", "debug", "deps", "libcore.rlib")), "a dry run changes nothing");
    assert.ok(!existsSync(quarantine), "a dry run writes no quarantine folder");
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("--include-debug puts the debug cache in the plan, and only then", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const env = {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
    };
    const withFlag = clean(dir, ["--dry-run", "--include-debug"], env);
    assert.equal(withFlag.status, 0, `exit 0 on report-only, got: ${withFlag.out}`);
    assert.match(withFlag.out, /target[\\/]debug/, "the flag names the debug cache");
    assert.match(withFlag.out, /WOULD reclaim/, "the reclaim summary is printed");
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("after the move the dry run lists no target/debug entry", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const env = {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
      E2040_CLEAN_TASKLIST: NO_BUILD,
    };
    const first = clean(dir, ["--auto", "--include-debug"], env);
    assert.equal(first.status, 0, `the move succeeds, got: ${first.out}`);
    const second = clean(dir, ["--dry-run"], env);
    assert.equal(second.status, 0, `exit 0 on the report, got: ${second.out}`);
    assert.ok(!/target[\\/]debug/.test(second.out), `no target/debug entry: ${second.out}`);
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("two moves of the same name land side by side, each with its own MOVES.tsv row", () => {
  const { base, dir, quarantine } = debugFixture();
  try {
    const env = {
      E2040_QUARANTINE_ROOT: quarantine,
      E2040_QUARANTINE_DATE: "2026-10-22",
      E2040_CLEAN_TASKLIST: NO_BUILD,
    };
    const first = clean(dir, ["--auto", "--include-debug"], env);
    assert.equal(first.status, 0, `the first move succeeds, got: ${first.out}`);
    mkdirSync(join(dir, "target", "debug"), { recursive: true });
    writeFileSync(join(dir, "target", "debug", "again.bin"), "again\n");
    const second = clean(dir, ["--auto", "--include-debug"], env);
    assert.equal(second.status, 0, `the second move succeeds, got: ${second.out}`);
    assert.ok(existsSync(join(quarantine, "2026-10-22", "target-debug", "deps", "libcore.rlib")), "the first move stays");
    assert.ok(existsSync(join(quarantine, "2026-10-22", "target-debug-2", "again.bin")), "the second move gets its own name");
    const rows = readFileSync(join(quarantine, "2026-10-22", "MOVES.tsv"), "utf8").split("\n").filter((l) => l.includes("target\\debug"));
    assert.equal(rows.length, 2, `two rows in MOVES.tsv: ${rows.length}`);
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});
