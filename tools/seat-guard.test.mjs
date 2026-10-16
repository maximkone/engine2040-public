// Seat guard rules (.opencode/seat-guard.mjs): each seat class writes only its lane, the bash
// deny list, coder dispatch caps and the cargo slot files.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  acquireSlot, allowedPaths, checkBash, checkCall, checkDispatch, checkEditPath, classOf, claimLines,
  editTargets, isCargo, isTestOf, listSlots, pathMatch, queueCap, relPath, releaseSlot, scanShell, seatMix, shellWrites, withCargoJobs,
} from "../.opencode/seat-guard.mjs";

const root = "C:/engine2040";
const knobs = {
  coders: { value: 2, seat: "builder", class: "code" },
  smiths: { value: 6, seat: "smith", class: "write" },
  porters: { value: 6, seat: "porter", class: "research" },
  runners: { value: 3, seat: "runner", class: "run" },
  queue_per_writer: { value: 3 },
};
const now = Date.parse("2026-10-10T12:00:00Z");

test("classes come from defaults and seat knobs", () => {
  assert.equal(classOf("builder", knobs), "code");
  assert.equal(classOf("judge", knobs), "read");
  assert.equal(classOf("porter", knobs), "research");
  assert.equal(classOf("smith-paid", knobs), "write");
  assert.equal(classOf("build", knobs), null);
  assert.deepEqual(seatMix(knobs).total, 17);
  assert.equal(queueCap(knobs), 24);
});

test("relPath keeps repo paths and rejects outside ones", () => {
  assert.equal(relPath("C:\\engine2040\\crates\\net\\src\\lib.rs", root), "crates/net/src/lib.rs");
  assert.equal(relPath("crates/net/src/lib.rs", root), "crates/net/src/lib.rs");
  assert.equal(relPath("C:/empire/center/x.md", root), null);
});

test("path globs, folders and test siblings", () => {
  assert.ok(pathMatch("crates/net/src/a.rs", "crates/net/"));
  assert.ok(pathMatch("crates/net/src/a.rs", "crates/net/**"));
  assert.ok(pathMatch("crates/net/src/a.rs", "crates/net/src/*.rs"));
  assert.ok(!pathMatch("crates/net/src/x/a.rs", "crates/net/src/*.rs"));
  assert.ok(isTestOf("crates/net/tests/snap.rs", "crates/net/src/snap.rs"));
  assert.ok(isTestOf("crates/net/src/snap_test.rs", "crates/net/src/snap.rs"));
  assert.ok(isTestOf("crates/net/src/snap/tests.rs", "crates/net/src/snap.rs"));
  assert.ok(isTestOf("tools/board-slice.test.mjs", "tools/board-slice.mjs"));
  assert.ok(!isTestOf("crates/ai/tests/x.rs", "crates/net/src/snap.rs"));
});

test("claims: fresh lines named for the packet add paths", () => {
  const claims = [
    "builder-rome-20261010-ui3 | 2026-10-10T11:50Z | UI3 crates/ui/src/lib.rs",
    "builder-rome-old | 2026-10-10T01:00Z | UI1 crates/ui/src/old.rs",
    "mp9-repair | 2026-10-10T11:00Z | MP9 crates/net/src/session.rs (two expects, lane mp9)",
  ].join("\n");
  assert.equal(claimLines(claims, now).length, 2);
  assert.deepEqual(allowedPaths({ packet: { id: "builder-rome-r4", standing: "builder-rome.md", fm: {} }, claims, now }), ["crates/ui/src/lib.rs"]);
  assert.deepEqual(allowedPaths({ packet: { id: "x", fm: { paths: "crates/a/src/b.rs, tools/c.mjs" } }, claims, now }), ["crates/a/src/b.rs", "tools/c.mjs"]);
});

test("claims: a standing run never inherits a sibling run's claim (seat-guard-scope x3)", () => {
  const claims = [
    "skill-hunter-loop-r1 | 2026-10-10T11:50Z | skills/gaffer-netcode/SKILL.md",
    "skill-hunter-loop-r2 | 2026-10-10T11:51Z | skills/grep-overflow-guard/SKILL.md",
    "skill-hunter-loop-20261010 | 2026-10-10T11:52Z | skills/shared-index.md",
  ].join("\n");
  const run = (id) => allowedPaths({ packet: { id, standing: "skill-hunter-loop.md", fm: {} }, claims, now });
  assert.deepEqual(run("skill-hunter-loop-r1"), ["skills/gaffer-netcode/skill.md", "skills/shared-index.md"]);
  assert.deepEqual(run("skill-hunter-loop-r2"), ["skills/grep-overflow-guard/skill.md", "skills/shared-index.md"]);
  assert.deepEqual(run("skill-hunter-loop-r3"), ["skills/shared-index.md"]);
  const seat = { cls: "write", packet: { id: "skill-hunter-loop-r3", standing: "skill-hunter-loop.md", fm: {} } };
  const why = checkCall({ tool: "write", args: { filePath: "skills/task-abort-guard/SKILL.md" }, seat, root, claims, now });
  assert.match(why, /^out of packet paths: skills\/task-abort-guard\/SKILL\.md; allowed: skills\/shared-index\.md; if your item owns it, first append `skill-hunter-loop-r3 \| /i);
  assert.equal(checkCall({ tool: "write", args: { filePath: "skills/task-abort-guard/SKILL.md" }, seat, root, claims: `${claims}\nskill-hunter-loop-r3 | 2026-10-10T11:55Z | skills/task-abort-guard/SKILL.md`, now }), null);
});

test("edit targets from edit, write and apply_patch", () => {
  assert.deepEqual(editTargets("edit", { filePath: "a.rs" }), ["a.rs"]);
  assert.deepEqual(editTargets("read", { filePath: "a.rs" }), []);
  assert.deepEqual(editTargets("apply_patch", { patchText: "*** Begin Patch\n*** Update File: crates/a/src/b.rs\n*** Add File: docs/x.md\n*** End Patch" }), ["crates/a/src/b.rs", "docs/x.md"]);
});

test("coder edits stay inside packet paths plus tests", () => {
  const seat = { cls: "code", packet: { id: "gp18", fm: { paths: "crates/combat/src/feel.rs" } } };
  const call = (filePath) => checkCall({ tool: "edit", args: { filePath }, seat, root, claims: "", now });
  assert.equal(call("C:/engine2040/crates/combat/src/feel.rs"), null);
  assert.equal(call("crates/combat/tests/feel.rs"), null);
  assert.equal(call("target/reports/gp18.md"), null);
  assert.match(call("crates/net/src/lib.rs"), /^out of packet paths: crates\/net\/src\/lib\.rs; allowed: crates\/combat\/src\/feel\.rs \[rule: packet-paths; no claim line names it\]$/);
  assert.match(call("target/sprint-handoff.md"), /belong to the lead/);
  const none = checkCall({ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat: { cls: "code", packet: null }, root, claims: "", now });
  assert.match(none, /^no packet paths/);
});

test("research seats write only queue packets and reports", () => {
  const seat = { cls: "research", packet: { id: "porter-1", fm: {} } };
  assert.equal(checkCall({ tool: "write", args: { filePath: "C:/engine2040/target/queue/ready/p1.md" }, seat, root, claims: "", now }), null);
  assert.equal(checkCall({ tool: "write", args: { filePath: "target/reports/porter.md" }, seat, root, claims: "", now }), null);
  assert.match(checkCall({ tool: "edit", args: { filePath: "docs/plan/steals.md" }, seat, root, claims: "", now }), /^research seat: write only a packet/);
  assert.match(checkCall({ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat, root, claims: "", now }), /^research seat/);
  assert.equal(checkCall({ tool: "write", args: { filePath: join(tmpdir(), "opencode", "x.mjs") }, seat, root, claims: "", now }), null);
  assert.match(checkCall({ tool: "write", args: { filePath: "C:/empire/center/x.md" }, seat, root, claims: "", now }), /^out of repo/);
});

test("write seats: claimed paths, never Rust or the board", () => {
  const seat = { cls: "write", packet: { id: "smith-2", fm: { paths: "tools/board-slice.mjs, skills/engine-look/" } } };
  const call = (filePath) => checkCall({ tool: "edit", args: { filePath }, seat, root, claims: "", now });
  assert.equal(call("tools/board-slice.mjs"), null);
  assert.equal(call("skills/engine-look/SKILL.md"), null);
  assert.match(call("crates/a/src/b.rs"), /^no Rust for this seat/);
  assert.match(call("docs/FOCUS.md"), /belong to the lead/);
  assert.match(call("docs/INDEX.md"), /^out of packet paths/);
});

test("read and run seats edit only target/; the lead never edits Rust", () => {
  assert.match(checkCall({ tool: "edit", args: { filePath: "docs/x.md" }, seat: { cls: "read" }, root, claims: "", now }), /^judge seat/);
  assert.equal(checkCall({ tool: "write", args: { filePath: "target/captures/a.png" }, seat: { cls: "run" }, root, claims: "", now }), null);
  assert.equal(checkCall({ tool: "edit", args: { filePath: "docs/SPRINT.md" }, seat: { cls: "lead" }, root, claims: "", now }), null);
  assert.match(checkCall({ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat: { cls: "lead" }, root, claims: "", now }), /lead never edits Rust/);
  assert.equal(checkCall({ tool: "edit", args: { filePath: "x" }, seat: { cls: null }, root, claims: "", now }), null);
});

test("bash deny list for every seat, the lead included", () => {
  const bad = [
    "git add -A", "git add .", "git add --all docs", "git -C C:\\engine2040 add -A", "git reset --hard HEAD", "git checkout -- crates/a.rs",
    "git clean -fd", "git rebase origin/master", "git push --force origin master", "git push -f", "git worktree add ../x",
    "node tools/lanes.mjs create x", "git commit -am wip",
    "git stash", "git stash push -m x", "git -C C:\\engine2040 stash pop", "git pull --autostash origin master",
    "git -c rebase.autoStash=true pull origin master", "git restore --source=HEAD -- .", "git restore --staged --worktree -- .",
  ];
  for (const command of bad) assert.match(checkBash({ command, cls: "lead", root, allowed: [] }) ?? "", /^blocked: /, command);
  for (const command of ["git add -- docs/SPRINT.md", "git status --short", "git restore --source=HEAD -- crates/a/src/b.rs", "git pull --no-rebase --no-edit origin master"]) {
    assert.equal(checkBash({ command, cls: "lead", root, allowed: [] }), null, command);
  }
});

test("bash: crate-wide cargo fmt and fix are blocked for seats; --check, rustfmt on own files and the lead pass", () => {
  for (const command of ["cargo fmt -p engine-endpoint", "cargo fmt", "$env:CARGO_BUILD_JOBS='2'; cargo fmt --all", "cargo +nightly fmt -p engine-ai",
    "cargo clippy -p engine-ai --fix --allow-dirty", "cargo fix -p engine-ai --allow-dirty"]) {
    for (const cls of ["code", "read"]) assert.match(checkBash({ command, cls, root, allowed: ["crates/ai/src/a.rs"] }) ?? "", /^blocked: cargo (?:fmt|clippy --fix)/, `${cls}: ${command}`);
  }
  for (const command of ["cargo fmt -p engine-ai -- --check", "cargo clippy -p engine-ai --all-targets -- -D warnings", "cargo test -p engine-ai --lib fixture"]) {
    assert.equal(checkBash({ command, cls: "code", root, allowed: ["crates/ai/src/a.rs"] }), null, command);
  }
  assert.equal(checkBash({ command: "cargo fmt -p engine-ai", cls: "lead", root, allowed: [] }), null, "the lead may format");
});

test("target/ and the workspace are safe: cargo clean, deleting target, workspace cargo, proof.mjs and mutants.mjs", () => {
  for (const command of ["cargo clean", "cargo clean -p engine-ai", "rm -rf target", "Remove-Item -Recurse -Force target", "Remove-Item -Recurse target/queue", "rm -rf ./target/", "rd /s /q target"]) {
    assert.match(checkBash({ command, cls: "lead", root, allowed: [] }) ?? "", /^blocked: /, `lead too: ${command}`);
  }
  for (const command of ["Remove-Item target/queue/ready/x.md", "rm target/reports/old.md", "Remove-Item -Recurse target/harvest2"]) {
    assert.equal(checkBash({ command, cls: "lead", root, allowed: [] }), null, command);
  }
  for (const command of ["cargo test --workspace", "cargo check --all", "node tools/proof.mjs", "node tools/mutants.mjs --patch - -p engine-ai"]) {
    assert.match(checkBash({ command, cls: "code", root, allowed: ["crates/ai/src/a.rs"] }) ?? "", /^blocked: /, command);
  }
  for (const command of ["cargo clippy -p engine-ai --all-targets --all-features", "node tools/proof.mjs --skip-build"]) {
    assert.equal(checkBash({ command, cls: "read", root, allowed: [] }), null, command);
  }
});

test("the loop's machinery and every board shard are off limits unless the packet's own paths name them", () => {
  const seat = (cls, paths) => ({ cls, packet: { id: "smith-tools-r3", standing: "smith-tools.md", fm: paths ? { paths } : {} } });
  const claims = "smith-tools-r3 | 2026-10-11T07:00Z | X1 .opencode/seat-guard.mjs docs/SPRINT-18-gaps.md tools/a.mjs\n";
  const now = Date.parse("2026-10-11T07:30Z");
  const edit = (s, file) => checkCall({ tool: "edit", args: { filePath: `${root}/${file}` }, seat: s, root, claims, now });
  assert.match(edit(seat("write"), ".opencode/seat-guard.mjs") ?? "", /^loop machinery: /, "a claim is not enough");
  assert.match(edit(seat("write"), ".opencode/plugin/loop-keeper.js") ?? "", /^loop machinery: /);
  assert.match(edit(seat("write"), "tools/rust-quality.baseline.json") ?? "", /^loop machinery: /);
  assert.equal(edit(seat("write", ".opencode/seat-guard.mjs"), ".opencode/seat-guard.mjs"), null, "its own paths: line is");
  assert.equal(edit(seat("write"), "tools/a.mjs"), null, "ordinary claimed tools stay open");
  assert.match(edit(seat("code"), "docs/FOCUS.md") ?? "", /board and FOCUS belong to the lead/);
  assert.match(edit({ cls: "write", packet: { id: "smith-x-r1", fm: {} } }, "docs/SPRINT-19-receipts.md") ?? "", /board and FOCUS|no packet paths/);
  assert.equal(edit({ cls: "lead" }, "docs/SPRINT-18-gaps.md"), null);
});

test("claims: the older keeper note's `<item> | <run> | <UTC> | <paths>` reads as the run's claim", () => {
  const now = Date.parse("2026-10-11T07:30Z");
  const lines = claimLines("EG13 | builder-focus-r4 | 2026-10-11T07:00Z | crates/render/src/msaa.rs\nbuilder-focus-r5 | 2026-10-11T07:10Z | EG14 crates/game/src/hud.rs\n", now);
  assert.deepEqual(lines.map((c) => [c.seat, c.row, c.paths]), [["builder-focus-r4", "EG13", ["crates/render/src/msaa.rs"]], ["builder-focus-r5", "EG14", ["crates/game/src/hud.rs"]]]);
});

test("bash: cargo only for code, read and lead; only the lead commits", () => {
  assert.ok(isCargo("cargo test -p net --lib"));
  assert.ok(isCargo("$env:X='1'; cargo check -p ai"));
  assert.ok(!isCargo("node tools/cargo-report.mjs"));
  assert.equal(checkBash({ command: "cargo test -p net", cls: "code", root, allowed: [] }), null);
  assert.equal(checkBash({ command: "cargo test -p net", cls: "read", root, allowed: [] }), null);
  assert.match(checkBash({ command: "cargo run -p game", cls: "run", root, allowed: [] }), /^runner seats call cargo only to rebuild a binary/);
  // tools/bins.mjs rebuild lines pass for runner and perf seats; nothing else does.
  const rebuild = "$env:CARGO_BUILD_JOBS='2'; $env:RUSTC_WRAPPER=''; cargo build --release -p engine-game --features app --bin staged_fight";
  assert.equal(checkBash({ command: rebuild, cls: "run", root, allowed: [] }), null);
  assert.equal(checkBash({ command: "cargo build --release -p engine-perf --bin perf", cls: "run", root, allowed: [] }), null);
  for (const bad of ["cargo build -p engine-perf --bin perf", "cargo build --release -p engine-perf --bin perf; cargo test -p net", "cargo build --release -p engine-perf --bin perf --target-dir C:/x", "cargo test --release -p engine-perf"]) {
    assert.match(checkBash({ command: bad, cls: "run", root, allowed: [] }), /^runner seats call cargo only/, bad);
  }
  assert.match(checkBash({ command: rebuild, cls: "research", root, allowed: [] }), /never calls cargo/);
  assert.match(checkBash({ command: "cargo check", cls: "research", root, allowed: [] }), /never calls cargo/);
  const allowed = ["tools/x.mjs"];
  assert.match(checkBash({ command: "git add -- tools/x.mjs; git commit -m \"tools: x\" -- tools/x.mjs", cls: "write", root, allowed }), /never stages or commits/);
  assert.match(checkBash({ command: "git add -- crates/a/src/b.rs", cls: "code", root, allowed: ["crates/a/src/b.rs"] }), /never stages or commits/);
  assert.match(checkBash({ command: "git stash", cls: "write", root, allowed }), /git stash takes every session/);
  assert.equal(withCargoJobs("cargo test -p a"), "$env:CARGO_BUILD_JOBS='10'; cargo test -p a");
  assert.equal(withCargoJobs("node x.mjs"), "node x.mjs");
});

test("dispatch: coder cap and disjoint crates", () => {
  const p = (id, paths) => ({ id, role: "builder", fm: { paths } });
  assert.equal(checkDispatch({ packet: p("a", "crates/net/src/a.rs"), running: [p("b", "crates/ai/src/b.rs")], knobs, claims: "", now }), null);
  assert.match(checkDispatch({ packet: p("a", "crates/net/src/a.rs"), running: [p("b", "crates/net/src/c.rs")], knobs, claims: "", now }), /^paths overlap with running coder b/);
  assert.match(checkDispatch({ packet: p("a", "crates/x/src/a.rs"), running: [p("b", "crates/y/src/b.rs"), p("c", "crates/z/src/c.rs")], knobs, claims: "", now }), /^coder seats full: 2 of 2/);
  assert.equal(checkDispatch({ packet: { id: "s", role: "smith", fm: {} }, running: [p("b", "x"), p("c", "y")], knobs, claims: "", now }), null);
});

test("dispatch: a packet naming a Claude-held file waits in ready/ (2026-10-11)", () => {
  const claims = "claude-eg33-melee-brains | 2026-10-10T11:50Z | EG33 crates/ai/src/depth.rs crates/ai/src/gladiator.rs";
  const cut = { id: "cut-ai-depth-winloss", role: "builder", fm: { paths: "crates/ai/src/depth.rs" } };
  assert.match(checkDispatch({ packet: cut, running: [], knobs, claims, now }), /^crates\/ai\/src\/depth\.rs is held by Claude coder claude-eg33-melee-brains \(row EG33\); the packet waits/);
  const smith = { id: "skill-x", role: "smith", fm: { paths: "crates/ai/src/gladiator.rs, skills/x/SKILL.md" } };
  assert.match(checkDispatch({ packet: smith, running: [], knobs, claims, now }), /held by Claude coder/);
  assert.equal(checkDispatch({ packet: { ...cut, fm: { paths: "crates/ai/src/guard.rs" } }, running: [], knobs, claims, now }), null);
  const stale = "claude-eg33-melee-brains | 2026-10-10T07:00Z | EG33 crates/ai/src/depth.rs";
  assert.equal(checkDispatch({ packet: cut, running: [], knobs, claims: stale, now }), null);
});

test("dispatch: a smith packet on FOCUS or a SPRINT shard is held for a planner (smith-FOCUS-deny x2)", () => {
  const focus = { id: "cut-focus-hs15", role: "smith", fm: { paths: "docs/FOCUS.md" } };
  assert.match(checkDispatch({ packet: focus, running: [], knobs, claims: "", now }), /^a smith seat cannot edit docs\/focus\.md .*role to planner$/);
  assert.match(checkDispatch({ packet: { ...focus, fm: { paths: "docs/SPRINT-18-gaps.md" } }, running: [], knobs, claims: "", now }), /cannot edit docs\/sprint-18-gaps\.md/);
  assert.equal(checkDispatch({ packet: { ...focus, role: "planner" }, running: [], knobs, claims: "", now }), null);
  assert.equal(checkDispatch({ packet: { ...focus, fm: { paths: "docs/INDEX.md" } }, running: [], knobs, claims: "", now }), null);
});

test("cargo slots: at most max, freed by release, age or a dead owner", () => {
  const dir = mkdtempSync(join(tmpdir(), "cargo-slots-"));
  try {
    assert.ok(acquireSlot({ dir, max: 2, id: "c1", info: { seat: "builder", cmd: "cargo test" }, now }).ok);
    assert.ok(acquireSlot({ dir, max: 2, id: "c2", now }).ok);
    const full = acquireSlot({ dir, max: 2, id: "c3", now });
    assert.equal(full.ok, false);
    assert.equal(full.busy.length, 2);
    releaseSlot({ dir, id: "c1" });
    assert.ok(acquireSlot({ dir, max: 2, id: "c3", now }).ok);
    assert.ok(acquireSlot({ dir, max: 2, id: "c4", now: now + 76 * 60_000 }).ok, "old slots expire");
    assert.ok(acquireSlot({ dir, max: 2, id: "c5", now: now + 76 * 60_000, alive: (s) => s.id !== "c4" }).ok, "a dead owner's slot frees");
    assert.deepEqual(listSlots(dir).map((s) => s.id), ["c5"]);
    // A tool's slot (no session, a pid) frees once that process is gone, long before 75 min.
    assert.ok(acquireSlot({ dir, max: 2, id: "q1", info: { seat: "perf-quiet", pid: 4242 }, now: now + 76 * 60_000 }).ok);
    assert.equal(acquireSlot({ dir, max: 2, id: "c6", now: now + 77 * 60_000, pidLive: () => true }).ok, false, "a live tool keeps its slot");
    assert.ok(acquireSlot({ dir, max: 2, id: "c6", now: now + 77 * 60_000, pidLive: (pid) => pid !== 4242 }).ok, "a killed tool's slot frees");
    assert.deepEqual(listSlots(dir).map((s) => s.id).sort(), ["c5", "c6"]);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("cargo slots: a keeper slot past 3 min frees when no cargo or rustc runs; a tool's slot and a young one stay", () => {
  const dir = mkdtempSync(join(tmpdir(), "cargo-slots-"));
  try {
    assert.ok(acquireSlot({ dir, max: 3, id: "k1", info: { sid: "s1", seat: "judge" }, now, procs: () => 0 }).ok);
    assert.ok(acquireSlot({ dir, max: 3, id: "q1", info: { seat: "product-check", pid: 4242 }, now, procs: () => 0 }).ok);
    assert.ok(acquireSlot({ dir, max: 3, id: "k2", info: { sid: "s2" }, now: now + 4 * 60_000, pidLive: () => true, procs: () => 2 }).ok);
    // 5 min on: k1 is 5 min old, k2 1 min, q1 is a live tool. A cargo still running keeps them all.
    assert.equal(acquireSlot({ dir, max: 3, id: "k3", now: now + 5 * 60_000, pidLive: () => true, procs: () => 2 }).ok, false);
    assert.equal(acquireSlot({ dir, max: 3, id: "k3", now: now + 5 * 60_000, pidLive: () => true, procs: () => -1 }).ok, false, "unknown frees nothing");
    assert.ok(acquireSlot({ dir, max: 3, id: "k3", now: now + 5 * 60_000, pidLive: () => true, procs: () => 0 }).ok, "the orphan frees");
    assert.deepEqual(listSlots(dir).map((s) => s.id).sort(), ["k2", "k3", "q1"]);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("an unknown edit path with no paths names the fix", () => {
  assert.match(checkEditPath({ file: "tools/x.mjs", cls: "write", root, allowed: [] }), /claims\.txt/);
});

// ---- shell writes: a command that writes a file counts like an edit ----
const sh = (command, cls, allowed = []) => checkBash({ command, cls, root, allowed });
const oneLine = (why) => { assert.ok(typeof why === "string" && !/\n/.test(why), `one line: ${why}`); return why; };

test("relPath resolves dot segments, so target/.. cannot hide a write", () => {
  assert.equal(relPath("target/../docs/a.md", root), "docs/a.md");
  assert.equal(relPath("C:/engine2040/target/../crates/a.rs", root), "crates/a.rs");
  assert.equal(relPath("../outside.md", root), null);
  assert.match(checkCall({ tool: "edit", args: { filePath: "target/../docs/x.md" }, seat: { cls: "read" }, root, claims: "", now }), /^judge seat/);
});

test("shell scan: quoted operators are text, redirect targets and sinks are read", () => {
  assert.deepEqual(shellWrites('echo "a > b"; rg ">" docs'), { targets: [], blind: false });
  assert.deepEqual(shellWrites("node x.mjs 2>&1 | Select-Object -Last 5 > $null"), { targets: [], blind: false });
  assert.deepEqual(shellWrites("cargo test *> $null; x 2>nul"), { targets: [], blind: false });
  assert.deepEqual(shellWrites("echo a >> docs/x.md; echo b 2> docs/y.md").targets, ["docs/x.md", "docs/y.md"]);
  assert.deepEqual(shellWrites("@'\nnot > a/file\n'@ | Set-Content target/x.md").targets, ["target/x.md"]);
  assert.deepEqual(shellWrites("Set-Content `\n docs/y.md hi").targets, ["docs/y.md"]);
  assert.deepEqual(scanShell("git commit -m 'x; y > z' -- a.md")[0].words, ["git", "commit", "-m", "x; y > z", "--", "a.md"]);
});

test("research and run seats: every shell write outside target/ is blocked", () => {
  const writes = [
    'Set-Content docs/x.md "hi"', "Set-Content -Path docs/x.md -Value hi", "Set-Content -LiteralPath docs/x.md hi", '"hi" | Add-Content docs/x.md',
    "Get-Date | Out-File -FilePath docs/x.md -Append", "New-Item -ItemType File docs/x.md", "New-Item -Path docs -Name x.md -ItemType File",
    "Copy-Item target/a.txt docs/x.md", "Move-Item target/a.txt docs/x.md", "Remove-Item -Recurse -Force docs/x.md", "rm -rf docs/x.md", "del docs/x.md",
    "echo hi > docs/x.md", "echo hi >> docs/x.md", "node tools/x.mjs 2> docs/x.md", "node tools/x.mjs | Tee-Object -FilePath docs/x.md",
    "node -e \"require('fs').writeFileSync('docs/x.md','hi')\"", "node --eval \"fs.appendFileSync('docs/x.md', 'hi')\"",
    "node -e \"fs.renameSync('target/a.txt', 'docs/x.md')\"", "pwsh -Command \"Set-Content docs/x.md hi\"", "cmd /c \"echo hi > docs/x.md\"",
    "[System.IO.File]::WriteAllText('docs/x.md', 'hi')", "curl -o docs/x.md https://example.com", "Invoke-WebRequest -Uri https://example.com -OutFile docs/x.md",
    "git rm docs/x.md", "git mv target/a.md docs/x.md", "python -c \"open('docs/x.md', 'w').write('hi')\"", "Set-Content target/../docs/x.md hi",
  ];
  for (const [cls, name] of [["research", "research"], ["run", "runner"], ["read", "judge"]]) {
    for (const command of writes) {
      assert.equal(oneLine(sh(command, cls)), `${name} seat: no shell writes outside target/ (docs/x.md); report the fix as a packet`, `${cls}: ${command}`);
    }
  }
  assert.match(sh("Set-Content C:/empire/center/x.md hi", "research"), /^out of repo: C:\/empire\/center\/x\.md/);
  assert.match(sh("echo hi > target/sprint-handoff.md", "run"), /^the handoff, lock and halt belong to the lead/);
});

test("research, run and read seats may write under target/ and the temp folder", () => {
  const fine = [
    "Set-Content target/reports/x.md hi", "echo hi > target/queue/ready/p.md", "node tools/x.mjs 2>&1 | Select-Object -Last 5", "node tools/x.mjs > $null",
    "node tools/x.mjs 2>&1 | Tee-Object -FilePath target/r.txt", "Remove-Item -Recurse -Force target/tmp", "New-Item -ItemType Directory target/reports/port-1",
    "Copy-Item donor.rs target/reports/port-1/", "node -e \"fs.writeFileSync('target/a.txt','x')\"", 'echo "a > b"', "rg -n foo docs", "git status --short",
    "Invoke-WebRequest -Uri https://example.com -OutFile target/page.html", "Set-Content $env:TEMP/opencode/x.mjs hi", "Get-Content docs/x.md | Select-Object -First 3",
  ];
  for (const cls of ["research", "run", "read"]) for (const command of fine) assert.equal(sh(command, cls), null, `${cls}: ${command}`);
});

test("write and code seats: a parsed write must be inside the packet paths", () => {
  const allowed = ["tools/x.mjs", "docs/plan/seats.md"];
  assert.equal(sh("Set-Content tools/x.mjs hi", "write", allowed), null);
  assert.equal(sh("echo hi >> docs/plan/seats.md", "write", allowed), null);
  assert.equal(sh("echo hi > target/reports/x.md", "write", allowed), null);
  assert.equal(sh("Copy-Item target/a.mjs tools/x.mjs", "write", allowed), null);
  assert.equal(sh("node -e \"fs.writeFileSync('tools/x.mjs','x')\"", "write", allowed), null);
  assert.equal(oneLine(sh("Set-Content tools/y.mjs hi", "write", allowed)), "out of packet paths: tools/y.mjs; allowed: tools/x.mjs, docs/plan/seats.md");
  assert.match(sh("echo hi > docs/INDEX.md", "write", allowed), /^out of packet paths: docs\/index\.md/);
  assert.match(sh("Remove-Item tools/y.mjs", "write", allowed), /^out of packet paths/);
  assert.match(sh("Move-Item tools/x.mjs tools/z.mjs", "write", allowed), /^out of packet paths: tools\/z\.mjs/);
  assert.match(sh("git rm tools/y.mjs", "write", allowed), /^out of packet paths/);
  assert.match(sh("node -e \"fs.writeFileSync('tools/y.mjs','x')\"", "write", allowed), /^out of packet paths/);
  assert.match(sh("echo hi > crates/a/src/b.rs", "write", ["crates/a/src/b.rs"]), /^no Rust for this seat/);
  assert.match(sh("echo hi > docs/FOCUS.md", "write", []), /belong to the lead/);
  assert.match(sh("Set-Content ../x.md hi", "write", allowed), /^out of repo/);
  const coder = ["crates/combat/src/feel.rs"];
  assert.equal(sh("echo hi > crates/combat/src/feel.rs", "code", coder), null);
  assert.equal(sh("Set-Content crates/combat/tests/feel.rs hi", "code", coder), null);
  assert.equal(sh("cargo test -p combat 2>&1 | Tee-Object -FilePath target/t.txt", "code", coder), null);
  assert.match(sh("Set-Content crates/net/src/lib.rs hi", "code", coder), /^out of packet paths: crates\/net\/src\/lib\.rs; allowed: crates\/combat\/src\/feel\.rs$/);
  assert.match(sh("Remove-Item crates/net/src/lib.rs", "code", coder), /^out of packet paths/);
  assert.match(sh("echo hi > crates/net/src/lib.rs", "code", []), /^no packet paths/);
});

test("a write whose path cannot be read: blocked for research and run seats, allowed for the rest", () => {
  const blind = [
    "Set-Content $path hi", "Get-ChildItem docs | Remove-Item", 'echo hi > "$dir/x.md"', "node -e \"fs.writeFileSync(process.argv[1], 'x')\"",
    "node -e \"fs.writeFileSync(path.join('docs', 'x.md'), 'x')\"", "iex $cmd", "sed -i s/a/b/ docs/x.md", "Get-ChildItem | ForEach-Object { Remove-Item $_.FullName }",
    "[System.IO.File]::WriteAllText($p, 'hi')", "Copy-Item $src",
  ];
  for (const command of blind) {
    assert.match(oneLine(sh(command, "research")), /^research seat: this command writes to a path I cannot read; name a literal path under target\//, command);
    assert.match(oneLine(sh(command, "run")), /^runner seat: this command writes to a path I cannot read/, command);
    for (const cls of ["write", "code", "read"]) assert.equal(sh(command, cls, ["tools/x.mjs"]), null, `${cls}: ${command}`);
  }
  assert.equal(sh("Set-Content $path hi", "lead"), null);
});

test("git forms that write: apply, checkout <ref> -- <path>, restore", () => {
  const own = ["crates/a/src/b.rs"];
  for (const cls of ["code", "write", "read", "run", "research"]) {
    assert.match(sh("git apply fix.patch", cls, own), /^this seat never applies, stashes, restores or reverts/, cls);
    assert.match(sh("git -C C:/engine2040 apply --index fix.patch", cls, own), /^this seat never applies/, cls);
    assert.match(sh("git checkout master", cls, own), /^this seat never applies/, cls);
    assert.match(sh("git switch -c x", cls, own), /^this seat never applies/, cls);
  }
  for (const cls of ["lead", "code", "write", "read", "run", "research"]) {
    for (const command of ["git checkout HEAD~1 -- crates/a/src/b.rs", "git checkout abc1234 -- docs/x.md", "git checkout origin/master -- tools/x.mjs", "git checkout -- ."]) {
      assert.match(sh(command, cls, own), /^blocked: git checkout -- discards edits/, `${cls}: ${command}`);
    }
  }
  assert.equal(sh("git apply fix.patch", "lead"), null);
  // restore: the lead as before; a coder only from HEAD and only its packet paths
  assert.equal(sh("git restore --source=HEAD -- crates/a/src/b.rs", "lead"), null);
  for (const command of ["git restore --source=HEAD -- crates/a/src/b.rs", "git restore --source HEAD -- crates/a/src/b.rs", "git restore -s HEAD -- crates/a/src/b.rs", "git restore --source=HEAD -W -- crates/a/src/b.rs crates/a/tests/t.rs"]) {
    assert.equal(sh(command, "code", own), null, command);
  }
  assert.match(oneLine(sh("git restore --source=HEAD -- crates/net/src/lib.rs", "code", own)), /^out of packet paths: crates\/net\/src\/lib\.rs/);
  assert.match(sh("git restore --source=HEAD -- .", "code", own), /^blocked: git restore -- . discards/);
  for (const command of ["git restore crates/a/src/b.rs", "git restore --staged -- crates/a/src/b.rs", "git restore --source=HEAD~1 -- crates/a/src/b.rs", "git restore --source=HEAD", "git restore -p -- crates/a/src/b.rs"]) {
    assert.match(oneLine(sh(command, "code", own)), /^coder seats may only run git restore --source=HEAD -- <your packet paths>/, command);
  }
  for (const cls of ["write", "read", "run", "research"]) assert.match(sh("git restore --source=HEAD -- crates/a/src/b.rs", cls, own), /^this seat never applies/, cls);
});

test("write seats may create the folder of an allowed file; case never decides (2026-10-10 miners)", () => {
  const allowed = ["skills/tool-door-patterns/skill.md", "skills/tool-door-patterns/tests/pairs.md"];
  const at = (file) => checkEditPath({ file: `${root}/${file}`, cls: "write", root, allowed });
  assert.equal(at("skills/tool-door-patterns/tests"), null);
  assert.equal(at("skills/tool-door-patterns"), null);
  assert.equal(at("skills/tool-door-patterns/SKILL.md"), null);
  assert.match(at("skills/door-verb-checklist"), /^out of packet paths/);
  assert.match(at("skills/tool-door-patterns/other.md"), /^out of packet paths/);
});

test("a file a Claude coder claimed is off limits to loop seats, even when the seat claims it too (2026-10-10)", () => {
  const seat = { cls: "code", packet: { id: "60-coder-vs4", standing: "builder-focus.md", fm: {} } };
  const at = new Date(now).toISOString().slice(0, 16) + "Z";
  const claims = [
    `claude-eg1-live-match | ${at} | EG1 crates/game/src/lib.rs crates/arena/src/live.rs`,
    `60-coder-vs4 | ${at} | VS4 crates/game/src/lib.rs crates/game/src/fighter_draw.rs`,
  ].join("\n");
  const call = (filePath) => checkCall({ tool: "edit", args: { filePath }, seat, root, claims, now });
  assert.match(call("C:/engine2040/crates/game/src/lib.rs"), /^held by Claude coder claude-eg1-live-match \(row EG1\)/);
  assert.equal(call("C:/engine2040/crates/game/src/fighter_draw.rs"), null);
  const stale = `claude-eg1-live-match | 2020-01-01T00:00Z | EG1 crates/game/src/lib.rs\n60-coder-vs4 | ${at} | VS4 crates/game/src/lib.rs`;
  assert.equal(checkCall({ tool: "edit", args: { filePath: "crates/game/src/lib.rs" }, seat, root, claims: stale, now }), null, "a stale Claude claim frees the file");
});

// ---- a refusal names who refused: the rule, and the claim line when one is the cause (2026-10-12: 487 blind retries) ----
test("a refusal ends with the rule that refused it, so a seat stops retrying blind", () => {
  const seat = { cls: "code", packet: { id: "gp18", fm: { paths: "crates/combat/src/feel.rs" } } };
  const why = checkCall({ tool: "edit", args: { filePath: "crates/net/src/lib.rs" }, seat, root, claims: "", now });
  assert.equal(why, "out of packet paths: crates/net/src/lib.rs; allowed: crates/combat/src/feel.rs [rule: packet-paths; no claim line names it]");
  const none = checkCall({ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat: { cls: "code", packet: null }, root, claims: "", now });
  assert.match(none, /\[rule: no-packet-paths; no claim line names it\]$/);
  assert.equal(checkCall({ tool: "edit", args: { filePath: "crates/combat/src/feel.rs" }, seat, root, claims: "", now }), null, "an allowed call has no reason");
});

test("a refusal names the claim line that holds the file: another seat's, a stale own one, or a Claude coder's", () => {
  const at = new Date(now - 60_000).toISOString().slice(0, 16) + "Z";
  const seat = { cls: "write", packet: { id: "skill-hunter-loop-r3", standing: "skill-hunter-loop.md", fm: {} } };
  const shared = `skill-hunter-loop-r3 | ${at} | skills/shared-index.md`;
  const edit = (claims) => checkCall({ tool: "write", args: { filePath: "skills/gaffer-netcode/SKILL.md" }, seat, root, claims: `${shared}\n${claims}`, now });
  const theirs = `skill-hunter-loop-r2 | ${at} | skills/gaffer-netcode/SKILL.md`;
  assert.match(edit(theirs), new RegExp(`\\[rule: packet-paths; claim line "skill-hunter-loop-r2 \\| ${at} \\| skills/gaffer-netcode/SKILL\\.md" is another seat's \\(skill-hunter-loop-r2\\), not yours\\]$`));
  const stale = "skill-hunter-loop-r3 | 2026-10-09T01:00Z | skills/gaffer-netcode/SKILL.md";
  assert.match(edit(stale), /rule: packet-paths; your claim line "skill-hunter-loop-r3 \| 2026-10-09T01:00Z \| skills\/gaffer-netcode\/SKILL\.md" is older than 3 h: append a fresh one\]$/);
  assert.match(edit(""), /\[rule: packet-paths; no claim line names it\]$/);
  const coder = { cls: "code", packet: { id: "60-coder-vs4", fm: {} } };
  const claude = `claude-eg1-live-match | ${at} | EG1 crates/game/src/lib.rs`;
  const held = checkCall({ tool: "edit", args: { filePath: "crates/game/src/lib.rs" }, seat: coder, root, claims: claude, now });
  assert.match(held, /^held by Claude coder claude-eg1-live-match \(row EG1\): crates\/game\/src\/lib\.rs; take another row or file \[rule: claude-claim; claim line "claude-eg1-live-match \| /);
});

test("every kind of refusal carries its rule name, edits and shell alike", () => {
  const code = { cls: "code", packet: { id: "gp18", fm: { paths: "crates/combat/src/feel.rs" } } };
  const write = { cls: "write", packet: { id: "smith-2", fm: { paths: "tools/x.mjs" } } };
  const cases = [
    [{ tool: "edit", args: { filePath: "C:/empire/center/x.md" }, seat: code }, "repo-only"],
    [{ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat: { cls: "lead" } }, "lead-no-rust"],
    [{ tool: "edit", args: { filePath: "docs/x.md" }, seat: { cls: "research" } }, "research-lane"],
    [{ tool: "edit", args: { filePath: "target/sprint-handoff.md" }, seat: code }, "lead-owns-sprint-files"],
    [{ tool: "edit", args: { filePath: "docs/x.md" }, seat: { cls: "read" } }, "target-only"],
    [{ tool: "edit", args: { filePath: ".opencode/seat-guard.mjs" }, seat: write }, "machinery"],
    [{ tool: "edit", args: { filePath: "crates/a/src/b.rs" }, seat: write }, "no-rust"],
    [{ tool: "edit", args: { filePath: "docs/FOCUS.md" }, seat: code }, "board-lead-only"],
    [{ tool: "bash", args: { command: "git add -A" }, seat: code }, "bash-deny-list"],
    [{ tool: "bash", args: { command: "cargo test -p net" }, seat: { cls: "research" } }, "cargo-lane"],
    [{ tool: "bash", args: { command: "git commit -m x -- tools/x.mjs" }, seat: write }, "git-verbs"],
    [{ tool: "bash", args: { command: "Set-Content tools/y.mjs hi" }, seat: write }, "packet-paths"],
    [{ tool: "bash", args: { command: "echo hi > docs/x.md" }, seat: { cls: "run" } }, "target-only"],
  ];
  for (const [call, rule] of cases) {
    const why = checkCall({ ...call, root, claims: "", now });
    assert.match(why ?? "", new RegExp(`\\[rule: ${rule}[;\\]]`), `${call.tool} ${JSON.stringify(call.args)} -> ${why}`);
    assert.ok(!/\n/.test(why), "still one line");
  }
});
