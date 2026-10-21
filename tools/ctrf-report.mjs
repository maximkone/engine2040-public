// HX12: the CI publishes a structured test report (the CTRF JSON shape), so a
// run's result is read without opening the log.
//   node tools/ctrf-report.mjs [--in <junit.xml>] [--out <ctrf.json>] [--log <nextest.log>]
// The input is a nextest JUnit receipt: one <testcase> per test the run
// printed. The report is a pure function of the receipt bytes: the same
// receipt writes the same file, byte for byte. The tool never reads the clock,
// so two runs over the same tree give the same bytes.
// The receipt is found by itself: nextest 0.9 writes it under its store dir
// (`target/nextest/<profile>/`), so a hardcoded path breaks. The run log names
// the run and the receipt carries the same run id, so the two are matched.
// With --log the tool also reads the nextest summary line and fails when the
// run and the receipt disagree on any status.
// Exit 0 the report is written, 1 the receipt is not a test run (or the counts
// disagree), 2 the receipt is missing or unreadable.
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SPEC_VERSION = "0.2.0";
// Where a receipt lands. Nextest writes each profile's receipt under its
// store dir (`target/nextest/<profile>/`), so the configured path rides
// inside it. Older nextest put the path straight under the workspace root.
// The tool also scans the store dirs, so a nextest that moves the file again
// still finds it.
export const DEFAULT_INPUTS = [
  "target/nextest/ci/target/nextest-junit/ci.xml",
  "target/nextest/ci/ci.xml",
  "target/nextest-junit/ci.xml",
  "target/nextest/proof/target/nextest-junit/proof.xml",
  "target/nextest/proof/proof.xml",
  "target/nextest-junit/proof.xml",
];
// The dirs nextest and the repo config name. The scan below is bounded: a
// deep tree and a long file list never slow the tool down.
export const STORE_DIRS = ["target/nextest", "target/nextest-junit"];
const SCAN_DEPTH = 6;
const SCAN_FILES = 400;
// A setup script is not a test: nextest writes it in a suite of its own.
const SETUP_SUITE = "@setup-script:";

export function decodeEntities(text) {
  return text.replace(/&(#x[0-9a-fA-F]+|#\d+|lt|gt|amp|quot|apos);/g, (match, name) => {
    if (name === "lt") return "<";
    if (name === "gt") return ">";
    if (name === "amp") return "&";
    if (name === "quot") return '"';
    if (name === "apos") return "'";
    const code = name[1] === "x" ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
    return Number.isFinite(code) ? String.fromCodePoint(code) : match;
  });
}

export function parseAttrs(text) {
  const attrs = {};
  const re = /([A-Za-z_:][-A-Za-z0-9_:.]*)\s*=\s*("([^"]*)"|'([^']*)')/g;
  let m;
  while ((m = re.exec(text)) !== null) attrs[m[1]] = decodeEntities(m[3] ?? m[4] ?? "");
  return attrs;
}

// The '>' that closes a tag, ignoring one inside a quoted attribute value.
function tagEnd(text, start) {
  let quote = null;
  for (let i = start + 1; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === quote) quote = null;
    } else if (c === '"' || c === "'") {
      quote = c;
    } else if (c === ">") {
      return i;
    }
  }
  return -1;
}

// A tiny XML reader for the JUnit subset: elements, attributes, text.
export function parseXml(text) {
  const root = { name: "#document", attrs: {}, children: [], text: "" };
  const stack = [root];
  let i = 0;
  while (i < text.length) {
    const lt = text.indexOf("<", i);
    if (lt < 0) {
      stack[stack.length - 1].text += decodeEntities(text.slice(i));
      break;
    }
    if (lt > i) stack[stack.length - 1].text += decodeEntities(text.slice(i, lt));
    if (text.startsWith("<!--", lt)) {
      const end = text.indexOf("-->", lt);
      i = end < 0 ? text.length : end + 3;
      continue;
    }
    if (text.startsWith("<![CDATA[", lt)) {
      const end = text.indexOf("]]>", lt);
      stack[stack.length - 1].text += decodeEntities(text.slice(lt + 9, end < 0 ? text.length : end));
      i = end < 0 ? text.length : end + 2;
      continue;
    }
    if (text.startsWith("<?", lt)) {
      const end = text.indexOf("?>", lt);
      i = end < 0 ? text.length : end + 2;
      continue;
    }
    if (text.startsWith("<!", lt)) {
      const end = text.indexOf(">", lt);
      i = end < 0 ? text.length : end + 1;
      continue;
    }
    const end = tagEnd(text, lt);
    if (end < 0) break;
    const body = text.slice(lt + 1, end);
    if (body.startsWith("/")) {
      const name = body.slice(1).trim();
      for (let s = stack.length - 1; s > 0; s--) {
        if (stack[s].name === name) {
          stack.length = s;
          break;
        }
      }
      i = end + 1;
      continue;
    }
    const selfClosing = body.endsWith("/");
    const inner = selfClosing ? body.slice(0, -1) : body;
    const m = /^([^\s/>]+)\s*([\s\S]*)$/.exec(inner);
    const node = {
      name: m ? m[1] : inner.trim(),
      attrs: m ? parseAttrs(m[2]) : {},
      children: [],
      text: "",
    };
    stack[stack.length - 1].children.push(node);
    if (!selfClosing) stack.push(node);
    i = end + 1;
  }
  return root;
}

// JUnit writes a test's runtime in seconds; CTRF wants milliseconds.
export function secondsToMs(value) {
  if (value === undefined || value === null || value === "") return 0;
  const seconds = Number.parseFloat(value);
  if (!Number.isFinite(seconds)) return 0;
  return Math.round(seconds * 1000);
}

export function epochMs(value) {
  if (!value) return 0;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : 0;
}

function child(node, name) {
  return node.children.find((c) => c.name === name);
}

// The elements nextest writes for an attempt before the last: a flaky test
// that passed (<flakyFailure>), a flaky test that failed (<flakyFailure> on a
// failed test), and a test that failed after a retry (<rerunFailure>).
const ATTEMPT_ELEMENTS = ["flakyFailure", "flakyError", "rerunFailure", "rerunError"];
const FLAKY_ELEMENTS = ["flakyFailure", "flakyError"];

function readCase(node, suiteName) {
  const failure = child(node, "failure") ?? child(node, "error");
  const skipped = child(node, "skipped");
  const attempts = node.children.filter((c) => ATTEMPT_ELEMENTS.includes(c.name));
  const flaky = attempts.some((c) => FLAKY_ELEMENTS.includes(c.name));
  let status = "passed";
  if (failure) status = "failed";
  else if (skipped) status = "skipped";
  // A test that passed but leaked handles is an error element nextest writes;
  // the run counts it as a pass.
  if (failure && failure.name === "error" && /passed but leaked/.test(failure.attrs.type ?? "")) {
    status = "passed";
  }
  const stdout = child(node, "system-out");
  const stderr = child(node, "system-err");
  const start = epochMs(node.attrs.timestamp);
  const duration = secondsToMs(node.attrs.time);
  const flakyFailure = child(node, "flakyFailure");
  return {
    name: node.attrs.name ?? "",
    classname: node.attrs.classname ?? suiteName,
    suiteName,
    status,
    duration,
    start,
    stop: start ? start + duration : 0,
    flaky,
    retries: attempts.length,
    message: failure?.attrs.message ?? skipped?.attrs.message ?? flakyFailure?.attrs.message ?? "",
    trace: (failure ?? flakyFailure)?.text.trim() ?? "",
    stdout: stdout?.text ?? "",
    stderr: stderr?.text ?? "",
  };
}

// Read a JUnit receipt into the cases it holds. A setup script is not a test,
// so its suite is left out of the counts.
export function readReceipt(xmlText) {
  const doc = parseXml(xmlText);
  const root =
    doc.children.find((c) => c.name === "testsuites") ??
    doc.children.find((c) => c.name === "testsuite");
  if (!root) throw new Error("not a JUnit receipt: no <testsuites> root");
  const suites = [];
  const cases = [];
  let setupScripts = 0;
  for (const suiteNode of root.children.filter((c) => c.name === "testsuite")) {
    const suiteName = suiteNode.attrs.name ?? "";
    const isSetup = suiteName.startsWith(SETUP_SUITE);
    if (!isSetup) suites.push(suiteName);
    for (const caseNode of suiteNode.children.filter((c) => c.name === "testcase")) {
      if (isSetup) {
        setupScripts += 1;
        continue;
      }
      cases.push(readCase(caseNode, suiteName));
    }
  }
  return {
    declared: root.attrs.tests === undefined ? null : Number(root.attrs.tests),
    uuid: root.attrs.uuid ?? "",
    start: epochMs(root.attrs.timestamp),
    suites,
    setupScripts,
    cases,
  };
}

// Strip the colours nextest paints its output with, so a log read from a
// terminal reads the same as one read from a file.
export function stripAnsi(text) {
  return text.replace(/\u001b\[[0-9;]*m/g, "");
}

// Split a breakdown on its top-level commas. The commas inside a parenthesis
// ("5 passed (1 slow, 1 flaky)") belong to the note, not to the list.
export function splitTopLevel(text) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "(") depth += 1;
    else if (c === ")") depth -= 1;
    else if (c === "," && depth === 0) {
      parts.push(text.slice(start, i).trim());
      start = i + 1;
    }
  }
  const rest = text.slice(start).trim();
  if (rest) parts.push(rest);
  return parts.filter(Boolean);
}

// The nextest summary line: "Summary [   0.123s] 42 tests run: 41 passed, ...".
// Nextest prints the singular for one test ("1 test run"), so both are read.
// A cancelled run prints the tests that finished over the tests it started
// ("8/10 tests run"), so the first number is the tests that ran.
// The breakdown names every status apart: a run that timed out or failed to
// execute says so, so "1 exec failed" is never read as "1 failed".
export function readSummaryLine(logText) {
  const text = stripAnsi(logText);
  let line = null;
  for (const candidate of text.split(/\r?\n/)) {
    if (/\bSummary\b/.test(candidate) && /\brun:/.test(candidate)) line = candidate;
  }
  if (!line) return null;
  const count = /(\d+)(?:\/(\d+))?\s+tests?\s+run:/.exec(line);
  if (!count) return null;
  const summary = {
    finished: Number(count[1]),
    initial: count[2] === undefined ? Number(count[1]) : Number(count[2]),
    passed: 0,
    failed: 0,
    execFailed: 0,
    timedOut: 0,
    skipped: 0,
    flaky: 0,
    slow: 0,
    leaky: 0,
    leakyFailed: 0,
  };
  const breakdown = line.slice(line.indexOf("run:") + 4);
  for (const part of splitTopLevel(breakdown)) {
    const note = /\(([^)]*)\)\s*$/.exec(part);
    const head = note ? part.slice(0, part.indexOf("(")).trim() : part;
    const read = /^(\d+)\s+(.+)$/.exec(head);
    if (!read) continue;
    const amount = Number(read[1]);
    const what = read[2].trim();
    if (what === "passed") summary.passed = amount;
    else if (what === "failed") summary.failed = amount;
    else if (what === "exec failed") summary.execFailed = amount;
    else if (what === "timed out") summary.timedOut = amount;
    else if (what === "skipped") summary.skipped = amount;
    if (!note) continue;
    for (const piece of note[1].split(",")) {
      const value = /^(\d+)\s+(.+)$/.exec(piece.trim());
      if (!value) continue;
      const amount2 = Number(value[1]);
      const label = value[2].trim();
      if (label === "slow") summary.slow = amount2;
      else if (label === "flaky") summary.flaky = amount2;
      else if (label === "leaky") summary.leaky = amount2;
      else if (label === "due to being leaky") summary.leakyFailed = amount2;
    }
  }
  return summary;
}

// The run's own words against the receipt. Every status must agree, so a
// receipt that does not match the run is named instead of published.
// Nextest counts a timed out failure and an execution failure apart from its
// failures, but the receipt writes both as a failed test, so the receipt's
// failed count is the sum of the three. A flaky test that passed counts as a
// pass in the run and as a pass in the receipt. A receipt may leave skipped
// tests out (the nextest default reports only some skips), so it may hold
// fewer skipped tests than the run printed.
export function compareSummary(receipt, printed) {
  const cases = receipt.cases;
  const passed = cases.filter((c) => c.status === "passed").length;
  const failed = cases.filter((c) => c.status === "failed").length;
  const skipped = cases.filter((c) => c.status === "skipped").length;
  const flakyPassed = cases.filter((c) => c.status === "passed" && c.flaky).length;
  const problems = [];
  if (passed !== printed.passed) {
    problems.push(`the run printed ${printed.passed} passed but the receipt holds ${passed}`);
  }
  const printedFailed = printed.failed + printed.timedOut + printed.execFailed;
  if (failed !== printedFailed) {
    problems.push(`the run printed ${printed.failed} failed but the receipt holds ${failed}`);
  }
  if (skipped > printed.skipped) {
    problems.push(`the run printed ${printed.skipped} skipped but the receipt holds ${skipped}`);
  }
  if (passed + failed !== printed.finished) {
    problems.push(
      `the run printed ${printed.finished} tests but the receipt holds ${passed + failed} testcases`,
    );
  }
  if (flakyPassed !== printed.flaky) {
    problems.push(`the run printed ${printed.flaky} flaky but the receipt holds ${flakyPassed}`);
  }
  return problems;
}

// The CTRF 0.2.0 report. Key order is fixed so the file is byte-stable.
export function buildCtrf(receipt, opts = {}) {
  const tests = receipt.cases.map((c) => {
    const entry = {
      name: c.name,
      status: c.status,
      duration: c.duration,
      suite: c.suiteName ? [c.suiteName] : [],
    };
    if (c.start) entry.start = c.start;
    if (c.stop) entry.stop = c.stop;
    if (c.flaky) entry.flaky = true;
    if (c.retries) entry.retries = c.retries;
    if (c.message) entry.message = c.message;
    if (c.trace) entry.trace = c.trace;
    if (c.stdout) entry.stdout = c.stdout;
    if (c.stderr) entry.stderr = c.stderr;
    return entry;
  });
  const count = (status) => tests.filter((t) => t.status === status).length;
  const starts = tests.map((t) => t.start).filter((s) => s > 0);
  const stops = tests.map((t) => t.stop).filter((s) => s > 0);
  const start = starts.length ? Math.min(...starts) : receipt.start;
  const stop = stops.length ? Math.max(...stops) : start;
  const summary = {
    tests: tests.length,
    passed: count("passed"),
    failed: count("failed"),
    skipped: count("skipped"),
    pending: 0,
    other: count("other"),
    flaky: tests.filter((t) => t.flaky).length,
    suites: receipt.suites.length,
    start,
    stop,
    duration: stop - start,
  };
  return {
    reportFormat: "CTRF",
    specVersion: SPEC_VERSION,
    results: {
      tool: { name: opts.tool ?? "cargo-nextest" },
      summary,
      tests,
    },
  };
}

export function renderCtrf(report) {
  return `${JSON.stringify(report, null, 2)}\n`;
}

// Every receipt the tool can see: the known paths first, then a bounded scan
// of the store dirs for anything nextest wrote there.
export function listReceipts(root = ROOT) {
  const found = [];
  const seen = new Set();
  const add = (path) => {
    if (seen.has(path)) return;
    seen.add(path);
    try {
      const stat = statSync(path);
      if (stat.isFile()) found.push({ path, mtime: stat.mtimeMs });
    } catch {
      // Not there: the caller moves on.
    }
  };
  for (const candidate of DEFAULT_INPUTS) add(join(root, candidate));
  const walk = (dir, depth) => {
    if (depth > SCAN_DEPTH || found.length > SCAN_FILES) return;
    let entries = [];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path, depth + 1);
      else if (entry.name.endsWith(".xml")) add(path);
    }
  };
  for (const dir of STORE_DIRS) walk(join(root, dir), 0);
  return found;
}

// The receipt that belongs to the run in the log. Nextest prints the run id
// and the profile in its output, and the receipt carries the same run id, so
// the two are matched. Without a log the newest receipt wins.
export function pickReceipt(logText = null, root = ROOT) {
  const candidates = listReceipts(root);
  if (candidates.length === 0) return null;
  if (!logText) {
    candidates.sort((a, b) => b.mtime - a.mtime);
    return candidates[0].path;
  }
  const text = stripAnsi(logText);
  const runId = /Nextest run ID ([0-9a-fA-F-]{36})/.exec(text);
  const profile = /nextest profile:\s*(\S+)/.exec(text);
  const read = (path) => {
    try {
      return readReceipt(readFileSync(path, "utf8"));
    } catch {
      return null;
    }
  };
  if (runId) {
    const match = candidates.find((c) => read(c.path)?.uuid === runId[1]);
    if (match) return match.path;
    return null;
  }
  if (profile) {
    const name = profile[1];
    const match = candidates.find((c) => c.path.split(/[\\/]/).includes(name));
    if (match) return match.path;
  }
  candidates.sort((a, b) => b.mtime - a.mtime);
  return candidates[0].path;
}

// A path is taken as given when it is absolute, else it hangs off the repo.
export function resolvePath(path, root = ROOT) {
  return isAbsolute(path) ? path : join(root, path);
}

export function parseArgs(argv) {
  const opts = { in: null, out: null, log: null, tool: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--in") opts.in = argv[++i] ?? null;
    else if (arg === "--out") opts.out = argv[++i] ?? null;
    else if (arg === "--log") opts.log = argv[++i] ?? null;
    else if (arg === "--tool") opts.tool = argv[++i] ?? null;
    else throw new Error(`unknown argument: ${arg}`);
  }
  return opts;
}

export function main(argv, log = console.log, fail = console.error, root = ROOT) {
  const opts = parseArgs(argv);
  // The log is read first: it names the run, and the run names the receipt.
  let logText = null;
  if (opts.log) {
    try {
      logText = readFileSync(resolvePath(opts.log, root), "utf8");
    } catch (err) {
      fail(`cannot read the run log ${resolvePath(opts.log, root)}: ${err.message}`);
      return 2;
    }
  }
  const input = opts.in ? resolvePath(opts.in, root) : pickReceipt(logText, root);
  if (!input) {
    fail(
      `no test receipt: looked under ${STORE_DIRS.join(" and ")} and in ${DEFAULT_INPUTS.join(", ")} (run the tests first, or pass --in <file>)`,
    );
    return 2;
  }
  let xml;
  try {
    xml = readFileSync(input, "utf8");
  } catch (err) {
    fail(`cannot read the receipt ${input}: ${err.message}`);
    return 2;
  }
  let receipt;
  try {
    receipt = readReceipt(xml);
  } catch (err) {
    fail(err.message);
    return 1;
  }
  if (receipt.cases.length === 0) {
    fail(`the receipt ${input} holds no testcase`);
    return 1;
  }
  // The receipt names its own count. A receipt that disagrees with its cases
  // is named, not refused: the report still holds what the receipt holds, and
  // the run log below is the strict gate on the count the run printed.
  const declared = receipt.declared === null ? null : receipt.declared - receipt.setupScripts;
  if (declared !== null && declared !== receipt.cases.length) {
    fail(
      `warn: the receipt ${input} says ${declared} tests but holds ${receipt.cases.length} testcases`,
    );
  }
  if (logText !== null) {
    const printed = readSummaryLine(logText);
    if (!printed) {
      fail(`the run log ${opts.log} holds no nextest summary line`);
      return 1;
    }
    const problems = compareSummary(receipt, printed);
    if (problems.length > 0) {
      for (const problem of problems) fail(problem);
      fail(`the receipt ${input} does not match the run`);
      return 1;
    }
  }
  const report = buildCtrf(receipt, { tool: opts.tool ?? undefined });
  const outPath = resolvePath(opts.out ?? "target/reports/ctrf.json", root);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, renderCtrf(report), "utf8");
  const s = report.results.summary;
  log(
    `ctrf: ${s.tests} tests (${s.passed} passed, ${s.failed} failed, ${s.skipped} skipped) -> ${opts.out ?? "target/reports/ctrf.json"}`,
  );
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exit(main(process.argv.slice(2)));
}
