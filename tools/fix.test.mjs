// tools/fix.test.mjs drills: the fix tool keeps the edits the agent owns and
// puts back every file outside --only that a fixer touched. Each drill builds a
// tiny no-dep crate under a temp dir (cargo fix wants a VCS, so the fixture gets
// its own git repo), runs tools/fix.mjs with cwd pointed at it, and then reads
// the owned file and the foreign file back.
//   node --test tools/fix.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
const FIX = join(REPO, "tools", "fix.mjs");
// The fixture lives outside the repo, so it gets the repo's toolchain pin:
// the drill must use the same rustc, clippy and rustfmt the repo uses.
const TOOLCHAIN = readFileSync(join(REPO, "rust-toolchain.toml"), "utf8");
const hasCargo = spawnSync("cargo", ["--version"], { encoding: "utf8", shell: false }).status === 0;
const hasGit = spawnSync("git", ["--version"], { encoding: "utf8", shell: false }).status === 0;
const noToolchain = !hasCargo || !hasGit;
const skip = noToolchain && "no cargo or git on this machine";

// The owned file carries a seeded unused import: cargo fix removes it.
const LIB = `use std::collections::HashMap;

pub mod other;

pub fn value() -> u32 {
    7
}
`;
// The foreign file is a child module of the owned one, so the fixers see it too.
const OTHER_LINTED = `use std::collections::BTreeMap;

pub fn other() -> u32 {
    3
}
`;
const OTHER_CLEAN = `pub fn other() -> u32 {
    3
}
`;

const fixture = (otherSrc) => {
  const dir = join(tmpdir(), `fix-drill-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "crates", "tiny", "src"), { recursive: true });
  writeFileSync(join(dir, "Cargo.toml"), '[workspace]\nmembers = ["crates/tiny"]\nresolver = "2"\n');
  writeFileSync(join(dir, "rust-toolchain.toml"), TOOLCHAIN);
  writeFileSync(
    join(dir, "crates", "tiny", "Cargo.toml"),
    '[package]\nname = "tiny"\nversion = "0.1.0"\nedition = "2021"\n',
  );
  writeFileSync(join(dir, "crates", "tiny", "src", "lib.rs"), LIB);
  writeFileSync(join(dir, "crates", "tiny", "src", "other.rs"), otherSrc);
  const g = spawnSync("git", ["init", "-q"], { cwd: dir, encoding: "utf8" });
  assert.equal(g.status, 0, `git init in the fixture: ${g.stderr ?? ""}`);
  return dir;
};

const fix = (dir, args) => {
  const r = spawnSync(process.execPath, [FIX, ...args], { encoding: "utf8", cwd: dir });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

test("an owned edit stays and a foreign file the fixer touched comes back byte for byte", { skip }, () => {
  const dir = fixture(OTHER_LINTED);
  try {
    const foreign = join(dir, "crates", "tiny", "src", "other.rs");
    const before = readFileSync(foreign);
    const r = fix(dir, ["-p", "tiny", "--only", "crates/tiny/src/lib.rs"]);
    assert.equal(r.status, 0, `exit 0 on a clean run, got: ${r.out}`);
    const lib = readFileSync(join(dir, "crates", "tiny", "src", "lib.rs"), "utf8");
    assert.ok(!lib.includes("HashMap"), `the owned fix stays in lib.rs, got: ${lib}`);
    assert.deepEqual(readFileSync(foreign), before, "the foreign file is byte-equal to its snapshot");
    assert.match(r.out, /fix: 1 owned files changed, 1 foreign files restored/, `receipt line, got: ${r.out}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a seeded unused import in an owned file is removed and no foreign file is touched", { skip }, () => {
  const dir = fixture(OTHER_CLEAN);
  try {
    const foreign = join(dir, "crates", "tiny", "src", "other.rs");
    const before = readFileSync(foreign);
    const r = fix(dir, ["-p", "tiny", "--only", "crates/tiny/src/lib.rs"]);
    assert.equal(r.status, 0, `exit 0 on a clean run, got: ${r.out}`);
    const lib = readFileSync(join(dir, "crates", "tiny", "src", "lib.rs"), "utf8");
    assert.ok(!lib.includes("use std::collections::HashMap;"), `the unused import is gone, got: ${lib}`);
    assert.deepEqual(readFileSync(foreign), before, "the foreign file is byte-equal to its snapshot");
    assert.match(r.out, /fix: 1 owned files changed, 0 foreign files restored/, `receipt line, got: ${r.out}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a dry run prints the plan and changes nothing", { skip }, () => {
  const dir = fixture(OTHER_LINTED);
  try {
    const lib = join(dir, "crates", "tiny", "src", "lib.rs");
    const before = readFileSync(lib);
    const r = fix(dir, ["-p", "tiny", "--only", "crates/tiny/src/lib.rs", "--dry-run"]);
    assert.equal(r.status, 0, `exit 0 on a dry run, got: ${r.out}`);
    assert.match(r.out, /would run cargo fix --allow-dirty --lib -p tiny/, `the plan is printed, got: ${r.out}`);
    assert.match(r.out, /would run rustfmt --edition 2021/, "the rustfmt leg is printed");
    assert.deepEqual(readFileSync(lib), before, "a dry run changes nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an unknown crate exits 2 and changes nothing", { skip }, () => {
  const dir = fixture(OTHER_CLEAN);
  try {
    const lib = join(dir, "crates", "tiny", "src", "lib.rs");
    const before = readFileSync(lib);
    const r = fix(dir, ["-p", "nope", "--only", "crates/tiny/src/lib.rs"]);
    assert.equal(r.status, 2, `exit 2 on an unknown crate, got: ${r.out}`);
    assert.match(r.out, /unknown crate/, "the error names the crate");
    assert.deepEqual(readFileSync(lib), before, "a usage error changes nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing owned path exits 2 and changes nothing", { skip }, () => {
  const dir = fixture(OTHER_CLEAN);
  try {
    const r = fix(dir, ["-p", "tiny", "--only", "crates/tiny/src/nope.rs"]);
    assert.equal(r.status, 2, `exit 2 on a missing owned path, got: ${r.out}`);
    assert.match(r.out, /not found/, "the error names the path");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
