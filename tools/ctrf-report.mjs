// HX12: the CI publishes a structured test report (the CTRF JSON shape), so a
// run's result is read without opening the log.
//   node tools/ctrf-report.mjs [--in <junit.xml>] [--out <ctrf.json>] [--log <nextest.log>]
// The input is a nextest JUnit receipt: one <testcase> per test the run
// printed. The report is a pure function of the receipt bytes: the same
// receipt writes the same file, byte for byte. The tool never reads the clock,
// so two runs over the same tree give the same bytes.
// With --log the tool also reads the nextest summary line and fails when the
// run printed a different number of tests than the receipt holds.
// Exit 0 the report is written, 1 the receipt is not a test run (or the counts
// disagree), 2 the receipt is missing or unreadable.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SPEC_VERSION = "0.2.0";
// Where a receipt lands: the CI profile first, the proof lane second.
export const DEFAULT_INPUTS = ["target/nextest-junit/ci.xml", "target/nextest-junit/proof.xml"];
export const DEFAULT_OUTPUT = "target/reports/ctrf.json";

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
      stack[stack.length - 1].text += text.slice(lt + 9, end < 0 ? text.length : end);
      i = end < 0 ? text.length : end + 3;
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

function readCase(node, suiteName) {
  const failure = child(node, "failure") ?? child(node, "error");
  const skipped = child(node, "skipped");
  const flakyFailure = child(node, "flakyFailure");
  const reruns = node.children.filter((c) => c.name === "rerun").length;
  let status = "passed";
  if (failure) status = "failed";
  else if (skipped) status = "skipped";
  const flaky = Boolean(flakyFailure) || node.attrs.type === "flaky failure";
  const stdout = child(node, "system-out");
  const stderr = child(node, "system-err");
  const start = epochMs(node.attrs.timestamp);
  const duration = secondsToMs(node.attrs.time);
  return {
    name: node.attrs.name ?? "",
    classname: node.attrs.classname ?? suiteName,
    suiteName,
    status,
    duration,
    start,
    stop: start ? start + duration : 0,
    flaky,
    retries: reruns,
    message: failure?.attrs.message ?? skipped?.attrs.message ?? flakyFailure?.attrs.message ?? "",
    trace: (failure ?? flakyFailure)?.text.trim() ?? "",
    stdout: stdout?.text ?? "",
    stderr: stderr?.text ?? "",
  };
}

// Read a JUnit receipt into the cases it holds.
export function readReceipt(xmlText) {
  const doc = parseXml(xmlText);
  const root =
    doc.children.find((c) => c.name === "testsuites") ??
    doc.children.find((c) => c.name === "testsuite");
  if (!root) throw new Error("not a JUnit receipt: no <testsuites> root");
  const suites = [];
  const cases = [];
  for (const suiteNode of root.children.filter((c) => c.name === "testsuite")) {
    const suiteName = suiteNode.attrs.name ?? "";
    suites.push(suiteName);
    for (const caseNode of suiteNode.children.filter((c) => c.name === "testcase")) {
      cases.push(readCase(caseNode, suiteName));
    }
  }
  return {
    declared: root.attrs.tests === undefined ? null : Number(root.attrs.tests),
    start: epochMs(root.attrs.timestamp),
    suites,
    cases,
  };
}

// The nextest summary line: "Summary [   0.123s] 42 tests run: 41 passed, ...".
// The total is read on its own, so a changed breakdown order still counts.
export function readSummaryLine(logText) {
  const line = logText.split(/\r?\n/).find((l) => /tests run/.test(l));
  if (!line) return null;
  const total = /(\d+) tests run/.exec(line);
  if (!total) return null;
  const passed = /(\d+) passed/.exec(line);
  const failed = /(\d+) failed/.exec(line);
  const skipped = /(\d+) skipped/.exec(line);
  return {
    tests: Number(total[1]),
    passed: passed ? Number(passed[1]) : 0,
    failed: failed ? Number(failed[1]) : 0,
    skipped: skipped ? Number(skipped[1]) : 0,
  };
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

export function pickInput(explicit) {
  if (explicit) return explicit;
  for (const candidate of DEFAULT_INPUTS) {
    const path = join(ROOT, candidate);
    if (existsSync(path)) return path;
  }
  return null;
}

// A path is taken as given when it is absolute, else it hangs off the repo.
export function resolvePath(path) {
  return isAbsolute(path) ? path : join(ROOT, path);
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

export function main(argv, log = console.log, fail = console.error) {
  const opts = parseArgs(argv);
  const input = pickInput(opts.in ? resolvePath(opts.in) : null);
  if (!input) {
    fail(`no test receipt: looked for ${DEFAULT_INPUTS.join(", ")} (run the tests first, or pass --in <file>)`);
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
  const report = buildCtrf(receipt, { tool: opts.tool ?? undefined });
  // The receipt names its own count. A receipt that disagrees with its cases
  // is named, not refused: the report still holds what the receipt holds, and
  // the run log below is the strict gate on the count the run printed.
  if (receipt.declared !== null && receipt.declared !== receipt.cases.length) {
    fail(`warn: the receipt ${input} says ${receipt.declared} tests but holds ${receipt.cases.length} testcases`);
  }
  if (opts.log) {
    const logPath = resolvePath(opts.log);
    let logText;
    try {
      logText = readFileSync(logPath, "utf8");
    } catch (err) {
      fail(`cannot read the run log ${logPath}: ${err.message}`);
      return 2;
    }
    const summary = readSummaryLine(logText);
    if (summary) {
      // nextest prints the total either with or without the skipped tests.
      const total = report.results.summary.tests;
      const ran = total - report.results.summary.skipped;
      if (summary.tests !== total && summary.tests !== ran) {
        fail(`the run printed ${summary.tests} tests but the receipt holds ${total} testcases`);
        return 1;
      }
    }
  }
  const outPath = resolvePath(opts.out ?? DEFAULT_OUTPUT);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, renderCtrf(report), "utf8");
  const s = report.results.summary;
  log(`ctrf: ${s.tests} tests (${s.passed} passed, ${s.failed} failed, ${s.skipped} skipped) -> ${opts.out ?? DEFAULT_OUTPUT}`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exit(main(process.argv.slice(2)));
}
