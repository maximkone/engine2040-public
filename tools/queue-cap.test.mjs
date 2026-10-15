// queue-cap: packet front matter, role, max_diff, path and test-line checks, and the ready count.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { capStatus, cargoTestArgs, checkPacket, countReady, parsePathEntries, verifyOne } from "./queue-cap.mjs";

const TOOLS = dirname(fileURLToPath(import.meta.url));

// A fake repo: one crate with one test, plus whatever the test adds.
function fixture(extra = {}) {
  const root = mkdtempSync(join(tmpdir(), "queue-cap-"));
  const files = {
    "crates/combat/Cargo.toml": "[package]\nname = \"engine-combat\"\n",
    "crates/combat/src/feel.rs": "#[test]\nfn feel_bar() {}\n",
    ...extra,
  };
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(join(root, dirname(path)), { recursive: true });
    writeFileSync(join(root, path), body);
  }
  return root;
}

// A packet with the given front matter; a key set to undefined is left out.
function packet(fm = {}) {
  const all = {
    role: "builder",
    title: "GP18 feel",
    row: "GP18",
    paths: "crates/combat/src/feel.rs",
    test: "cargo test -p engine-combat --lib feel",
    max_diff: "120",
    source: "https://github.com/o/r/blob/abc/x.rs",
    ...fm,
  };
  const head = Object.entries(all).filter(([, v]) => v !== undefined).map(([k, v]) => `${k}: ${v}`).join("\n");
  return `---\n${head}\n---\nGoal: x\nScope: y\nProof: z\nStop: S 20 min\n`;
}

test("a full packet passes, a thin one names what is missing", () => {
  const root = fixture();
  try {
    assert.deepEqual(checkPacket(packet(), { root }), []);
    assert.deepEqual(checkPacket("no front matter", { root }), ["no front matter (--- block)"]);
    const thin = checkPacket("---\nrole: builder\ntitle: t\n---\nGoal: x\n", { root });
    assert.ok(thin.includes("no paths:") && thin.includes("no Proof line"));
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("an unknown role fails; max_diff over 220 fails", () => {
  const root = fixture();
  try {
    assert.ok(checkPacket(packet({ role: "coder" }), { root }).includes("role coder is not a seat"));
    assert.ok(checkPacket(packet({ max_diff: "800" }), { root }).includes("max_diff 800 is over 220"));
    assert.deepEqual(checkPacket(packet({ max_diff: "220" }), { root }), []);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("paths must exist, or be (new) and absent in a folder the packet can create", () => {
  const root = fixture({ "crates/combat/src/taken.rs": "" });
  try {
    assert.match(checkPacket(packet({ paths: "crates/combat/src/nope.rs" }), { root }).join(" "), /crates\/combat\/src\/nope\.rs: does not exist/);
    assert.deepEqual(checkPacket(packet({ paths: "crates/combat/src/new.rs (new)" }), { root }), []);
    assert.match(checkPacket(packet({ paths: "crates/combat/src/taken.rs (new)" }), { root }).join(" "), /taken\.rs: \(new\) but it already exists/);
    assert.deepEqual(checkPacket(packet({ paths: "skills/new-skill/SKILL.md (new), skills/new-skill/tests/pairs.md (new)" }), { root }), []);
    assert.match(checkPacket(packet({ paths: "tools/gone/x.mjs (new)" }), { root }).join(" "), /folder tools\/gone does not exist/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("parsePathEntries splits on commas and on whitespace between paths", () => {
  assert.deepEqual(parsePathEntries("crates/a.rs crates/b.rs").map((e) => e.path), ["crates/a.rs", "crates/b.rs"]);
  assert.deepEqual(parsePathEntries("crates/a.rs, crates/b.rs").map((e) => e.path), ["crates/a.rs", "crates/b.rs"]);
  const mixed = parsePathEntries("a, b c (new)");
  assert.deepEqual(mixed.map((e) => e.path), ["a", "b", "c"]);
  assert.equal(mixed[2].isNew, true);
  assert.equal(mixed[0].isNew, false);
  assert.deepEqual(parsePathEntries("crates/a.rs").map((e) => e.path), ["crates/a.rs"]);
});

test("the test line must run a real cargo filter or an existing node test file", () => {
  const root = fixture();
  try {
    assert.match(checkPacket(packet({ test: "cargo test -p engine-combat --lib nothing_here" }), { root }).join(" "), /test filter nothing_here matches no test name in crates\/combat/);
    assert.match(checkPacket(packet({ test: "cargo test -p engine-missing --lib feel" }), { root }).join(" "), /unknown package engine-missing/);
    assert.match(checkPacket(packet({ test: "cargo check -p engine-combat" }), { root }).join(" "), /must start with cargo test -p/);
    assert.match(checkPacket(packet({ test: "node --test tools/none.test.mjs" }), { root }).join(" "), /test file tools\/none\.test\.mjs does not exist/);
    assert.deepEqual(checkPacket(packet({ test: "cargo test -p engine-combat --lib --features rapier -- feel" }), { root }), []);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("cargoTestArgs finds the package and the first positional filter", () => {
  assert.deepEqual(cargoTestArgs(["-p", "engine-net", "--lib"]), { pkg: "engine-net", filter: "" });
  assert.deepEqual(cargoTestArgs(["-p", "engine-physics", "--lib", "--features", "rapier", "--", "snapshot_64_fighters"]), { pkg: "engine-physics", filter: "snapshot_64_fighters" });
  assert.deepEqual(cargoTestArgs(["--package", "engine-render", "--test", "gpu", "v1_temporal"]), { pkg: "engine-render", filter: "v1_temporal" });
});

test("countReady counts coder and smith packets only", () => {
  const dir = mkdtempSync(join(tmpdir(), "queue-cap-"));
  try {
    writeFileSync(join(dir, "a.md"), packet());
    writeFileSync(join(dir, "b.md"), packet({ role: "smith" }));
    writeFileSync(join(dir, "c.md"), packet({ role: "porter" }));
    const knobs = { smiths: { value: 1, seat: "smith", class: "write" }, porters: { value: 1, seat: "porter", class: "research" } };
    assert.equal(countReady(dir, knobs), 2);
    assert.equal(countReady(join(dir, "none"), knobs), 0);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

const seatKnobs = {
  porters: { seat: "porter", class: "research" },
  runners: { seat: "runner", class: "run" },
  smiths: { seat: "smith", class: "write" },
};

test("research, run and write seats prove with a done-when line; a code seat cannot", () => {
  const root = fixture({ "tools/check.mjs": "// check\n" });
  const paths = "crates/combat/src/feel.rs, crates/combat/src/notes.md (new)";
  const check = (fm) => checkPacket(packet({ paths, ...fm }), { root, knobs: seatKnobs }).join(" | ");
  try {
    assert.equal(check({ role: "porter", test: "report crates/combat/src/notes.md exists" }), "");
    assert.match(check({ role: "porter", test: "report crates/combat/src/other.md exists" }), /report crates\/combat\/src\/other\.md is not in paths/);
    assert.equal(check({ role: "smith", test: 'rg -n -i "feel|bar" crates/combat/src/feel.rs' }), "");
    assert.equal(check({ role: "smith", test: '! rg -n "no such" crates/combat/src/feel.rs' }), "");
    assert.match(check({ role: "smith", test: "rg -n feel crates/combat/src/nope.rs" }), /rg file crates\/combat\/src\/nope\.rs is neither/);
    assert.equal(check({ role: "smith", test: "node tools/check.mjs" }), "");
    assert.match(check({ role: "smith", test: "node tools/gone.mjs" }), /node script tools\/gone\.mjs does not exist/);
    assert.match(check({ role: "smith", test: "cargo test -p engine-combat --lib feel" }), /must start with node --test/);
    assert.match(check({ role: "runner", test: "cargo test -p engine-combat --lib feel" }), /must start with an existing \.exe/);
    assert.match(check({ role: "builder", test: "report crates/combat/src/notes.md exists" }), /must start with cargo test -p/);
    assert.match(check({ role: "smith", test: "node tools/check.mjs --check <each packet>" }), /placeholder/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("agentlint is the check of a skill packet only", () => {
  const root = fixture({ "scripts/agentlint.mjs": "// lint\n" });
  const skill = ".opencode/skills/x/SKILL.md (new), .opencode/skills/x/tests/pairs.md (new)";
  const check = (fm) => checkPacket(packet({ role: "smith", ...fm }), { root, knobs: seatKnobs }).join(" | ");
  try {
    assert.equal(check({ paths: skill, test: "node scripts/agentlint.mjs engine2040" }), "");
    assert.match(check({ paths: "crates/combat/src/feel.rs", test: "node scripts/agentlint.mjs engine2040" }), /agentlint is the skill check/);
    assert.match(check({ paths: skill, test: "node C:/nowhere/agentlint.mjs engine2040" }), /agentlint script C:\/nowhere\/agentlint\.mjs does not exist/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

// ---- one packet per row (2026-10-12 flow audit: 5 rows had 2 ready packets, EG8 was planned twice) ----
function aged(root, rel, minutes, now) {
  const file = join(root, rel);
  const t = (now - minutes * 60_000) / 1000;
  utimesSync(file, t, t);
}

test("a second ready packet for a row is refused and names the first; the oldest stands", () => {
  const now = Date.now();
  const root = fixture({ "target/queue/ready/0-gp18-feel.md": packet(), "target/queue/ready/0-gp18-feel-b.md": packet({ title: "GP18 feel b" }) });
  try {
    aged(root, "target/queue/ready/0-gp18-feel.md", 30, now);
    aged(root, "target/queue/ready/0-gp18-feel-b.md", 5, now);
    const second = checkPacket(packet({ title: "GP18 feel b" }), { root, self: "0-gp18-feel-b.md", now }).join(" | ");
    assert.match(second, /row GP18 is already named by ready\/0-gp18-feel\.md/);
    assert.deepEqual(checkPacket(packet(), { root, self: "0-gp18-feel.md", now }), [], "the older packet is the one that stands");
    assert.match(checkPacket(packet(), { root, now }).join(" "), /row GP18 is already named by ready\/0-gp18-feel/, "a draft outside ready/ meets the queued twin");
    assert.deepEqual(checkPacket(packet({ row: "GP19" }), { root, self: "0-gp19.md", now }), [], "another row is free");
    assert.deepEqual(checkPacket(packet({ row: "none" }), { root, self: "x.md", now }), [], "no row id, no clash");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("rows match on every id of a row: line; category tags name no row", () => {
  const now = Date.now();
  const root = fixture({
    "target/queue/ready/a.md": packet({ row: "MP6, MP17" }),
    "target/queue/ready/b.md": packet({ row: "rot:unwired-testonly" }),
    "target/queue/ready/c.md": packet({ row: "LEAD2" }),
  });
  try {
    const check = (row) => checkPacket(packet({ row }), { root, self: "n.md", now });
    assert.match(check("MP17").join(" "), /row MP17 is already named by ready\/a\.md/);
    assert.deepEqual(check("MP1"), [], "MP1 is not MP17");
    assert.deepEqual(check("rot:unwired-testonly"), [], "a rot-hunter category tag is shared by design");
    assert.deepEqual(check("bug:x"), []);
    // with a board to read, only ids that are rows count: LEAD2 tags a class of packets (2026-10-12: 4 lead2 packets, row LEAD2)
    assert.match(check("LEAD2").join(" "), /row LEAD2 is already named by ready\/c\.md/, "no board in this tree: the id shape decides");
    mkdirSync(join(root, "docs"), { recursive: true });
    writeFileSync(join(root, "docs", "SPRINT.md"), "# SPRINT\n### Finish\n- [ ] MP17 net: a row, needs: none.\n- [ ] GP18 combat: another, needs: none.\n");
    assert.deepEqual(check("LEAD2"), [], "LEAD2 is no board row");
    assert.match(check("MP17").join(" "), /row MP17 is already named by ready\/a\.md/, "MP17 is a board row");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("a row with a done/ packet written in the last 2 h waits; an older one, a review and a record without that row do not", () => {
  const now = Date.now();
  const root = fixture({
    "target/queue/done/0-eg8-gate.md": packet({ row: "EG8", title: "EG8 gate" }),
    "target/queue/done/0-eg9-gate.md": packet({ row: "EG9", title: "EG9 gate" }),
    "target/queue/done/0-eg10-gate-review.md": "Review of 0-eg10-gate\nROW: EG10\nVERDICT: PASS\n",
    "target/queue/done/builder-focus-r4.md": "# FOCUS Now coder #4 (@builder, standing)\n\n## Result (completed)\n\nROW: EG13 | OWNED: crates/render/src/forward.rs | CRATE: engine-render\nRESULT: DONE - x | proof: node --test a.test.mjs\n",
  });
  try {
    aged(root, "target/queue/done/0-eg8-gate.md", 35, now);
    aged(root, "target/queue/done/0-eg9-gate.md", 200, now);
    aged(root, "target/queue/done/0-eg10-gate-review.md", 5, now);
    aged(root, "target/queue/done/builder-focus-r4.md", 20, now);
    const eg8 = checkPacket(packet({ row: "EG8" }), { root, self: "n.md", now }).join(" | ");
    assert.match(eg8, /row EG8 has done\/0-eg8-gate\.md, written 35 min ago \(under 2 h\)/);
    assert.deepEqual(checkPacket(packet({ row: "EG9" }), { root, self: "n.md", now }), [], "3 h 20 min old: free again");
    assert.deepEqual(checkPacket(packet({ row: "EG10" }), { root, self: "n.md", now }), [], "a review record names no row");
    assert.match(checkPacket(packet({ row: "EG13" }), { root, self: "n.md", now }).join(" "), /row EG13 has done\/builder-focus-r4\.md, written 20 min ago/, "a standing run's ROW: line counts");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("--check on a file in ready/ prints the row clash and exits 1; the older twin still passes", () => {
  const root = fixture({ "target/queue/ready/old.md": packet(), "target/queue/ready/new.md": packet() });
  try {
    mkdirSync(join(root, "tools"), { recursive: true });
    mkdirSync(join(root, ".opencode"), { recursive: true });
    for (const f of ["queue-cap.mjs", "board-slice.mjs"]) copyFileSync(join(TOOLS, f), join(root, "tools", f));
    copyFileSync(join(TOOLS, "..", ".opencode", "seat-guard.mjs"), join(root, ".opencode", "seat-guard.mjs"));
    aged(root, "target/queue/ready/old.md", 10, Date.now());
    const check = (name) => spawnSync(process.execPath, [join(root, "tools", "queue-cap.mjs"), "--check", join(root, "target", "queue", "ready", name)], { encoding: "utf8", cwd: root });
    const bad = check("new.md");
    assert.equal(bad.status, 1, bad.stdout + bad.stderr);
    assert.match(bad.stdout, /^FAIL .*row GP18 is already named by ready\/old\.md/);
    const good = check("old.md");
    assert.equal(good.status, 0, good.stdout + good.stderr);
    assert.match(good.stdout, /^PASS /);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("verifyOne chains cap plus check: ADD + PASS ok, FAIL or VERIFY not ok", () => {
  const root = fixture({ "target/queue/ready/old.md": packet(), "pkt-good.md": packet({ row: "GP19" }), "pkt-thin.md": "---\nrole: builder\ntitle: t\n---\nGoal: x\n" });
  try {
    const knobs = { coders: { value: 10, seat: "builder", class: "code" }, smiths: { value: 0, seat: "smith", class: "write" } };
    const good = verifyOne(join(root, "pkt-good.md"), { root, knobs });
    assert.match(good.capLine, /-> ADD/);
    assert.deepEqual(good.miss, []);
    assert.equal(good.ok, true);
    const thin = verifyOne(join(root, "pkt-thin.md"), { root, knobs });
    assert.ok(thin.miss.length > 0, "thin packet must list its missing keys");
    assert.equal(thin.ok, false);
    const full = verifyOne(join(root, "pkt-good.md"), { root, knobs: { coders: { value: 0, seat: "builder", class: "code" }, smiths: { value: 0, seat: "smith", class: "write" } } });
    assert.match(full.capLine, /-> VERIFY/);
    assert.equal(full.ok, false, "a good packet at the cap still waits");
    const cap = capStatus(root, knobs);
    assert.match(cap.line, /-> ADD/);
    assert.equal(cap.add, true);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("--verify-one prints cap line then PASS/FAIL in one call", () => {
  const root = fixture({ "pkt-good.md": packet(), "pkt-bad.md": "no front matter" });
  try {
    mkdirSync(join(root, "tools"), { recursive: true });
    mkdirSync(join(root, ".opencode"), { recursive: true });
    for (const f of ["queue-cap.mjs", "board-slice.mjs"]) copyFileSync(join(TOOLS, f), join(root, "tools", f));
    copyFileSync(join(TOOLS, "..", ".opencode", "seat-guard.mjs"), join(root, ".opencode", "seat-guard.mjs"));
    const run = (name) => spawnSync(process.execPath, [join(root, "tools", "queue-cap.mjs"), "--verify-one", join(root, name)], { encoding: "utf8", cwd: root });
    const good = run("pkt-good.md");
    assert.equal(good.status, 0, good.stdout + good.stderr);
    assert.match(good.stdout, /ready \d+ of cap \d+ -> ADD/);
    assert.match(good.stdout, /^PASS /m);
    const bad = run("pkt-bad.md");
    assert.equal(bad.status, 1, bad.stdout + bad.stderr);
    assert.match(bad.stdout, /ready \d+ of cap \d+ -> ADD/);
    assert.match(bad.stdout, /^FAIL /m);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
