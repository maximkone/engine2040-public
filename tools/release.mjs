#!/usr/bin/env node
// release: the version, changelog and tag plumbing (TG5).
//   node tools/release.mjs --check   the workspace version in Cargo.toml equals the
//                                    newest `## <version>` heading in CHANGELOG.md,
//                                    and Cargo.lock's root package entry agrees
//   node tools/release.mjs --tag     an annotated `v<version>` tag on HEAD, only
//                                    when --check passes and the tree is clean
// No network, no push: the tag stays local until a person pushes it.
// Exit: 0 ok, 1 a red check or a refused tag, 2 bad arguments.
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const ROOT_PACKAGE = "engine2040";

const HELP = `usage: node tools/release.mjs --check | --tag

  --check   Cargo.toml [workspace.package] version == newest CHANGELOG.md
            heading == Cargo.lock's ${ROOT_PACKAGE} entry
  --tag     annotated v<version> tag on HEAD (only when --check passes and
            git status is clean); never pushes
  --help    print this text, exit 0`;

const SEMVER = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

/** One `[section]` body of a TOML file, or "". */
function tomlSection(text, name) {
  const lines = String(text).split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === `[${name}]`);
  if (start === -1) return "";
  const body = [];
  for (const line of lines.slice(start + 1)) {
    if (/^\s*\[/.test(line)) break;
    body.push(line);
  }
  return body.join("\n");
}

/** The workspace version: `[workspace.package] version = "x"`. */
export function workspaceVersion(toml) {
  const m = /^\s*version\s*=\s*"([^"]+)"/m.exec(tomlSection(toml, "workspace.package"));
  return m ? m[1] : null;
}

/** The first `## <version>` heading of a changelog: the newest release. */
export function newestChangelogVersion(markdown) {
  for (const line of String(markdown).split(/\r?\n/)) {
    const m = /^##\s+v?(\S+)\s*$/.exec(line.trim());
    if (m && SEMVER.test(m[1])) return m[1];
  }
  return null;
}

/** The `version` of one `[[package]]` block in Cargo.lock. */
export function lockVersion(lock, name = ROOT_PACKAGE) {
  for (const block of String(lock).split(/^\s*\[\[package\]\]\s*$/m).slice(1)) {
    const named = new RegExp(`^\\s*name\\s*=\\s*"${name}"\\s*$`, "m");
    if (!named.test(block)) continue;
    const m = /^\s*version\s*=\s*"([^"]+)"/m.exec(block);
    if (m) return m[1];
  }
  return null;
}

/** The three-way version check. Every disagreement is a named problem. */
export function check({ toml, changelog, lock }) {
  const version = workspaceVersion(toml);
  const changelogVersion = newestChangelogVersion(changelog);
  const locked = lockVersion(lock);
  const problems = [];
  if (!version) problems.push(`Cargo.toml has no [workspace.package] version`);
  if (!changelogVersion) problems.push(`CHANGELOG.md has no '## <version>' heading`);
  if (!locked) problems.push(`Cargo.lock has no ${ROOT_PACKAGE} package entry`);
  if (version && changelogVersion && version !== changelogVersion) {
    problems.push(`CHANGELOG.md newest heading is ${changelogVersion}, workspace version is ${version}`);
  }
  if (version && locked && version !== locked) {
    problems.push(`Cargo.lock ${ROOT_PACKAGE} is ${locked}, workspace version is ${version}`);
  }
  return { ok: problems.length === 0, version, changelogVersion, locked, problems };
}

/** May a tag be made? A red check, a dirty tree or an existing tag refuses it. */
export function tagDecision({ result, clean, tagExists, tag }) {
  if (!result.ok) return { ok: false, why: `the check is red: ${result.problems.join("; ")}` };
  if (!clean) return { ok: false, why: "the working tree is dirty" };
  if (tagExists) return { ok: false, why: `tag ${tag} already exists` };
  return { ok: true, why: `tagging HEAD as ${tag}` };
}

function git(root) {
  return (args) => {
    const r = spawnSync("git", args, { cwd: root, encoding: "utf8" });
    return { status: r.status ?? 1, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
  };
}

function read(root) {
  return (name) => readFileSync(join(root, name), "utf8");
}

export function main(argv = process.argv.slice(2), deps = {}) {
  const root = deps.root ?? ROOT;
  const say = deps.say ?? console.log;
  const readFile = deps.read ?? read(root);
  const run = deps.git ?? git(root);
  const mode = argv[0];
  if (mode === "--help" || mode === "-h") {
    say(HELP);
    return 0;
  }
  if (argv.length !== 1 || (mode !== "--check" && mode !== "--tag")) {
    say(`unknown arguments: ${argv.join(" ")}`);
    say(HELP);
    return 2;
  }
  let result;
  try {
    result = check({
      toml: readFile("Cargo.toml"),
      changelog: readFile("CHANGELOG.md"),
      lock: readFile("Cargo.lock"),
    });
  } catch (error) {
    say(`release: cannot read the version files: ${error.message}`);
    return 2;
  }
  say(`release: workspace ${result.version ?? "?"}, changelog ${result.changelogVersion ?? "?"}, lock ${result.locked ?? "?"}`);
  for (const problem of result.problems) say(`RED ${problem}`);
  if (mode === "--check") {
    say(result.ok ? "release: check OK" : "release: check FAILED");
    return result.ok ? 0 : 1;
  }
  const version = result.version;
  const tag = `v${version}`;
  const status = run(["status", "--porcelain"]);
  const decision = tagDecision({
    result,
    clean: status.status === 0 && status.stdout.trim() === "",
    tagExists: run(["tag", "--list", tag]).stdout.trim() !== "",
    tag,
  });
  if (!decision.ok) {
    say(`release: ${decision.why}`);
    return 1;
  }
  const made = run(["tag", "-a", tag, "-m", `${ROOT_PACKAGE} ${version}`]);
  if (made.status !== 0) {
    say(`release: git tag failed: ${(made.stderr || made.stdout).trim().split("\n")[0]}`);
    return 1;
  }
  say(`release: ${decision.why}`);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
