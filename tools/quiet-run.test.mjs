// quiet-run: no timing command runs on a loaded box, STOP is held only for
// the window, and a hand-made STOP is never deleted. Run:
//   node --test tools/quiet-run.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  parseArgs,
  parseProcs,
  parseStop,
  quietVerdict,
  runQuiet,
} from "./quiet-run.mjs";

const tempRoot = () => {
  const dir = join(tmpdir(), `quiet-run-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "target", "nonstop"), { recursive: true });
  return dir;
};

const quietScan = () => ({ nonstop: [], cargo: 0, rustc: 0, arena: 0 });
let lowTick = 0;
const lowCpu = () => {
  lowTick += 1;
  return { total: 1000 + lowTick * 100, idle: 950 + lowTick * 95 };
};
let tick = 0;
const risingCpu = () => {
  tick += 1;
  return { total: 1000 + tick * 100, idle: 950 + tick * 95 };
};

test("quietVerdict refuses any load source and any CPU at or above the bar", () => {
  assert.match(quietVerdict({ cpu: [3, 4, 2] }), /^QUIET /);
  assert.match(quietVerdict({ cpu: [10] }), /^LOADED .*cpu max 10\.0% >= 10%/);
  assert.match(quietVerdict({ cpu: [1], nonstop: [123] }), /^LOADED nonstop pid 123/);
  assert.match(quietVerdict({ cpu: [1], cargo: 2 }), /^LOADED 2 cargo/);
  assert.match(quietVerdict({ cpu: [1], rustc: 1 }), /^LOADED 1 rustc/);
  assert.match(quietVerdict({ cpu: [1], arena: 1 }), /^LOADED 1 arena/);
  assert.match(quietVerdict({ cpu: [] }), /^LOADED /, "an empty sample fails closed");
});

test("parseStop tells live, stale and hand-made STOP apart", () => {
  assert.deepEqual(parseStop("not json"), { state: "foreign" });
  assert.deepEqual(parseStop("{}"), { state: "foreign" });
  assert.deepEqual(parseStop(JSON.stringify({ pid: 7, at: 1 }), () => true, 2), {
    state: "live",
    pid: 7,
  });
  assert.deepEqual(parseStop(JSON.stringify({ pid: 7, at: 1 }), () => false, 2), {
    state: "stale",
    pid: 7,
    why: "pid gone",
  });
  assert.equal(
    parseStop(JSON.stringify({ pid: 7, at: 1 }), () => true, 4 * 60 * 60_000).state,
    "stale",
    "a live pid older than the TTL is a crash, not a held window",
  );
});

test("parseProcs counts nonstop, cargo, rustc and arena", () => {
  assert.deepEqual(
    parseProcs([
      { ProcessId: 10, Name: "node.exe", CommandLine: "node tools/nonstop.mjs --match x" },
      { ProcessId: 11, Name: "node.exe", CommandLine: "node tools/other.mjs" },
      { ProcessId: 12, Name: "cargo.exe", CommandLine: "cargo test" },
      { ProcessId: 13, Name: "rustc.exe", CommandLine: "rustc x" },
      { ProcessId: 14, Name: "arena.exe", CommandLine: "arena.exe --frames 300" },
    ]),
    { nonstop: [10], cargo: 1, rustc: 1, arena: 1 },
  );
  assert.deepEqual(parseProcs({ ProcessId: 1, Name: "node.exe", CommandLine: "x" }), {
    nonstop: [],
    cargo: 0,
    rustc: 0,
    arena: 0,
  });
});

test("parseArgs requires a command after -- and validates numbers", () => {
  assert.deepEqual(parseArgs(["--dry", "--times", "2", "--", "node", "-e", "x"]), {
    dry: true,
    times: 2,
    maxWaitMin: 45,
    cpuMax: 10,
    samples: 3,
    gap: 10,
    cmd: "node",
    args: ["-e", "x"],
  });
  assert.equal(parseArgs(["--dry"]), null);
  assert.equal(parseArgs(["--times", "0", "--", "x"]), null);
  assert.equal(parseArgs(["--cpu", "101", "--", "x"]), null);
  assert.equal(parseArgs(["--bogus", "--", "x"]), null);
});

test("a quiet dry run writes and removes STOP without running the command", async () => {
  const root = tempRoot();
  try {
    let spawned = 0;
    const code = await runQuiet(["--dry", "--", "node", "-e", "process.exit(0)"], {
      root,
      scan: quietScan,
      cpuTimes: lowCpu,
      wait: async () => {},
      spawn: () => {
        spawned += 1;
        return { status: 0 };
      },
    });
    assert.equal(code, 0);
    assert.equal(spawned, 0);
    assert.equal(existsSync(join(root, "target", "nonstop", "STOP")), false);
    assert.equal(existsSync(join(root, "target", "nonstop", "quiet.lock")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a busy box refuses to run the command and leaves no STOP behind", async () => {
  const root = tempRoot();
  try {
    let spawned = 0;
    const code = await runQuiet(["--max-wait", "0", "--", "node", "-e", "x"], {
      root,
      scan: () => ({ nonstop: [], cargo: 2, rustc: 1, arena: 1 }),
      cpuTimes: () => ({ total: 100, idle: 5 }),
      wait: async () => {},
      spawn: () => {
        spawned += 1;
        return { status: 0 };
      },
    });
    assert.equal(code, 3);
    assert.equal(spawned, 0);
    assert.equal(existsSync(join(root, "target", "nonstop", "STOP")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a failed process probe is a refusal, never a quiet window", async () => {
  const root = tempRoot();
  try {
    let spawned = 0;
    const code = await runQuiet(["--", "node", "-e", "x"], {
      root,
      scan: () => null,
      cpuTimes: lowCpu,
      wait: async () => {},
      spawn: () => {
        spawned += 1;
        return { status: 0 };
      },
    });
    assert.equal(code, 2);
    assert.equal(spawned, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a stale STOP is removed; a hand-made STOP is respected", async () => {
  const root = tempRoot();
  const stop = join(root, "target", "nonstop", "STOP");
  try {
    writeFileSync(stop, JSON.stringify({ pid: 999999, at: Date.now() }));
    const stale = await runQuiet(["--dry", "--", "node", "-e", "x"], {
      root,
      scan: quietScan,
      cpuTimes: lowCpu,
      wait: async () => {},
      alive: (pid) => pid !== 999999,
    });
    assert.equal(stale, 0);
    assert.equal(existsSync(stop), false);

    writeFileSync(stop, "STOP\n");
    const foreign = await runQuiet(["--dry", "--", "node", "-e", "x"], {
      root,
      scan: quietScan,
      cpuTimes: lowCpu,
      wait: async () => {},
    });
    assert.equal(foreign, 4);
    assert.equal(existsSync(stop), true, "a human STOP is never deleted");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a live quiet.lock makes a second keeper exit 4 without writing STOP", async () => {
  const root = tempRoot();
  const lock = join(root, "target", "nonstop", "quiet.lock");
  try {
    writeFileSync(lock, JSON.stringify({ pid: 1, at: Date.now() }));
    let spawned = 0;
    const code = await runQuiet(["--dry", "--", "node", "-e", "x"], {
      root,
      scan: quietScan,
      cpuTimes: lowCpu,
      wait: async () => {},
      alive: (pid) => pid === 1,
      spawn: () => {
        spawned += 1;
        return { status: 0 };
      },
    });
    assert.equal(code, 4);
    assert.equal(spawned, 0);
    assert.equal(existsSync(join(root, "target", "nonstop", "STOP")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the command runs with STOP held, twice when asked, and the window closes after", async () => {
  const root = tempRoot();
  const stop = join(root, "target", "nonstop", "STOP");
  try {
    let calls = 0;
    let stopSeen = 0;
    const code = await runQuiet(["--times", "2", "--", "node", "-e", "x"], {
      root,
      scan: quietScan,
      cpuTimes: risingCpu,
      wait: async () => {},
      spawn: () => {
        calls += 1;
        if (existsSync(stop)) stopSeen += 1;
        return { status: 0 };
      },
    });
    assert.equal(code, 0);
    assert.equal(calls, 2);
    assert.equal(stopSeen, 2, "STOP is held for every pass");
    assert.equal(existsSync(stop), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
