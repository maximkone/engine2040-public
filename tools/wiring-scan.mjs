// wiring-scan.mjs - list pub items with no production user across crates.
// Regex first slice: pub fn|struct|enum|trait|const lines, test code skipped.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const PUB_RE = /^\s*pub\s+(fn|struct|enum|trait|const)\s+([A-Za-z_][A-Za-z0-9_]*)/;

// Shared run lock and HEAD+dirty verdict cache (also used by workspace-check.mjs).
// One run per name holds the lock, so a concurrent caller waits instead of scanning.
const LOCK_TTL_MS = 10 * 60 * 1000; // a holder file older than this is stale
const LOCK_MAX_WAIT_MS = 15 * 60 * 1000; // never wait on a lock forever

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

// { release }. A held lock is waited on (one wait line) until the holder releases.
export function acquireRunLock(repoDir, name, log = function () {}) {
  const path = join(repoDir, 'target', 'locks', name + '.lock');
  mkdirSync(dirname(path), { recursive: true });
  const started = Date.now();
  let told = false;
  for (;;) {
    try {
      writeFileSync(path, JSON.stringify({ pid: process.pid }), { flag: 'wx' });
      return { release: function () { try { unlinkSync(path); } catch { /* already released */ } } };
    } catch (err) {
      if (err?.code !== 'EEXIST') throw err;
    }
    let owner = NaN;
    try { owner = JSON.parse(readFileSync(path, 'utf8')).pid; } catch { owner = NaN; }
    if (owner === process.pid) return { release: function () {} };
    let stale = false;
    try { stale = Date.now() - statSync(path).mtimeMs > LOCK_TTL_MS; } catch { continue; }
    if (!stale && Number.isInteger(owner)) { try { process.kill(owner, 0); } catch (err) { stale = err?.code !== 'EPERM'; } }
    if (stale) { try { unlinkSync(path); } catch { /* raced with the holder */ } continue; }
    if (!told) {
      log('waiting for lock: ' + name + ' (held by pid ' + (Number.isInteger(owner) ? owner : '?') + ')');
      told = true;
    }
    if (Date.now() - started > LOCK_MAX_WAIT_MS) { log('lock wait timed out: ' + name); return { release: function () {} }; }
    sleepSync(100);
  }
}

// HEAD commit plus a sha256 over every dirty and untracked file's content plus the
// caller's own key (mode, root). null when git fails: run uncached, never stale.
export function treeKey(repoDir, extra = '') {
  try {
    const head = readFileSync(join(repoDir, '.git', 'HEAD'), 'utf8').trim();
    const ref = /^ref:\s*(.+)$/.exec(head)?.[1];
    const sha = ref ? readFileSync(join(repoDir, '.git', ...ref.split('/')), 'utf8').trim() : head;
    const git = (args) => execFileSync('git', ['-C', repoDir, '--no-optional-locks', ...args], { encoding: 'utf8' });
    const dirty = [...git(['diff-index', '--name-only', '-z', 'HEAD']).split('\0'),
      ...git(['ls-files', '--others', '--exclude-standard', '-z']).split('\0')].filter(Boolean);
    const sum = createHash('sha256');
    for (const rel of [...new Set(dirty)].sort()) {
      sum.update(rel).update('\0');
      try { sum.update(readFileSync(join(repoDir, rel))); } catch { sum.update('<gone>'); }
      sum.update('\0');
    }
    return sha + '|' + sum.digest('hex') + '|' + extra;
  } catch { return null; }
}

export function loadVerdict(repoDir, name, key) {
  try {
    const cached = JSON.parse(readFileSync(join(repoDir, 'target', name + '.verdict-cache.json'), 'utf8'));
    if (cached?.key === key && typeof cached.out === 'string') return cached;
  } catch { /* missing or corrupt: a full run below */ }
  return null;
}

export function saveVerdict(repoDir, name, key, entry) {
  try {
    mkdirSync(join(repoDir, 'target'), { recursive: true });
    writeFileSync(join(repoDir, 'target', name + '.verdict-cache.json'), JSON.stringify({ key, out: entry.out, err: entry.err, code: entry.code }));
  } catch { /* best-effort: the run already has its bytes */ }
}

function captureOutput(run) {
  const out = [];
  const err = [];
  const stdout = process.stdout.write;
  const stderr = process.stderr.write;
  process.stdout.write = (chunk) => { out.push(String(chunk)); return true; };
  process.stderr.write = (chunk) => { err.push(String(chunk)); return true; };
  try { run(); } finally { process.stdout.write = stdout; process.stderr.write = stderr; }
  return { out: out.join(''), err: err.join(''), code: process.exitCode ?? 0 };
}

// One run at a time: hold the lock, replay a keyed verdict, or run and save it.
export function runWithVerdictCache({ repoDir, name, extra = '', enabled = true, run }) {
  if (!enabled) { run(); return; }
  const lock = acquireRunLock(repoDir, name, (line) => process.stderr.write(line + '\n'));
  const emit = (entry) => {
    if (entry.out) process.stdout.write(entry.out);
    if (entry.err) process.stderr.write(entry.err);
    process.exitCode = entry.code ?? 0;
  };
  try {
    const key = treeKey(repoDir, extra);
    const hit = key ? loadVerdict(repoDir, name, key) : null;
    if (hit) { emit(hit); return; }
    const entry = captureOutput(run);
    if (key) saveVerdict(repoDir, name, key, entry);
    emit(entry);
  } finally {
    lock.release();
  }
}

// Committed baseline pin: total + per-crate island counts. --check fails on rise.
const BASELINE_FILE = 'wiring-scan.baseline.json';
const LEGACY_BASELINE_FILE = 'wiring-baseline.json';

function listRsFiles(root) {
  const out = [];
  const stack = [root];
  while (stack.length > 0) {
    const dir = stack.pop();
    let entries = [];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of entries) {
      const full = join(dir, e.name);
      if (e.isDirectory()) {
        if (e.name === 'tests') {
          continue;
        }
        stack.push(full);
      } else if (e.isFile() && e.name.endsWith('.rs')) {
        const rel = relative(root, full);
        const parts = rel.split(/[\\/]/);
        if (parts.includes('tests')) {
          continue;
        }
        out.push(full);
      }
    }
  }
  out.sort();
  return out;
}

function prodOf(src) {
  return src.split('#[cfg(test)]')[0];
}

export function scanIslands(rootDir) {
  if (!existsSync(rootDir)) {
    return [];
  }
  try {
    if (!statSync(rootDir).isDirectory()) {
      return [];
    }
  } catch {
    return [];
  }
  const files = listRsFiles(rootDir);
  const prods = new Map();
  for (const f of files) {
    let text = '';
    try {
      text = readFileSync(f, 'utf8');
    } catch {
      text = '';
    }
    prods.set(f, prodOf(text));
  }
  const defs = [];
  for (const f of files) {
    const prod = prods.get(f) || '';
    const rel = relative(rootDir, f).split('\\').join('/');
    const segs = rel.split('/');
    const crate = segs.length > 1 ? segs[0] : segs[0].replace(/\.rs$/, '');
    const lines = prod.split('\n');
    for (let i = 0; i < lines.length; i += 1) {
      const m = PUB_RE.exec(lines[i]);
      if (m) {
        defs.push({ crate: crate, file: rel, line: i + 1, kind: m[1], name: m[2], abs: f });
      }
    }
  }
  // Narrow index: one token set per file, then one name -> users map.
  // The old loop ran one RegExp per def over every other file's full text
  // (defs x files full-text scans). A def name matches /\bname\b in a file
  // exactly when the file's maximal identifier-token run equals the name
  // (same ASCII word class both sides), so a single map lookup per def
  // gives the same answer in one pass.
  const WORD_RE = /\b[A-Za-z_][A-Za-z0-9_]*\b/g;
  const usersOf = new Map();
  for (const entry of prods) {
    const seen = new Set();
    for (const tok of entry[1].matchAll(WORD_RE)) {
      const name = tok[0];
      if (seen.has(name)) continue;
      seen.add(name);
      const e = usersOf.get(name);
      if (e === undefined) usersOf.set(name, { n: 1, file: entry[0] });
      else {
        e.n += 1;
        if (e.file !== entry[0]) e.file = null;
      }
    }
  }
  const islands = [];
  for (const d of defs) {
    const e = usersOf.get(d.name);
    const used = e !== undefined && (e.file === null || e.file !== d.abs);
    if (!used) {
      islands.push({ crate: d.crate, file: d.file, line: d.line, kind: d.kind, name: d.name });
    }
  }
  islands.sort(function (a, b) {
    if (a.crate !== b.crate) {
      return a.crate < b.crate ? -1 : 1;
    }
    if (a.file !== b.file) {
      return a.file < b.file ? -1 : 1;
    }
    if (a.line !== b.line) {
      return a.line - b.line;
    }
    if (a.kind !== b.kind) {
      return a.kind < b.kind ? -1 : 1;
    }
    if (a.name !== b.name) {
      return a.name < b.name ? -1 : 1;
    }
    return 0;
  });
  return islands;
}

export function countByCrate(islands) {
  const counts = {};
  for (const it of islands) {
    counts[it.crate] = (counts[it.crate] || 0) + 1;
  }
  return counts;
}

export function checkRise(counts, baseline) {
  const risen = [];
  for (const name of Object.keys(counts)) {
    const now = counts[name] || 0;
    const was = baseline[name] || 0;
    if (now > was) {
      risen.push(name);
    }
  }
  risen.sort();
  return risen;
}

export function baselineOf(islands) {
  return { total: islands.length, by_crate: countByCrate(islands) };
}

export function baselineCounts(baseline) {
  if (!baseline || typeof baseline !== 'object') {
    return {};
  }
  if (baseline.by_crate && typeof baseline.by_crate === 'object') {
    return baseline.by_crate;
  }
  const out = {};
  for (const [k, v] of Object.entries(baseline)) {
    if (typeof v === 'number') {
      out[k] = v;
    }
  }
  return out;
}

export function baselineTotal(baseline) {
  if (!baseline || typeof baseline !== 'object') {
    return 0;
  }
  if (typeof baseline.total === 'number') {
    return baseline.total;
  }
  let sum = 0;
  for (const v of Object.values(baseline)) {
    if (typeof v === 'number') {
      sum += v;
    }
  }
  return sum;
}

export function readBaselineFile(toolDir) {
  const primary = join(toolDir, BASELINE_FILE);
  const legacy = join(toolDir, LEGACY_BASELINE_FILE);
  for (const p of [primary, legacy]) {
    try {
      return { path: p, baseline: JSON.parse(readFileSync(p, 'utf8')) };
    } catch {
      continue;
    }
  }
  return { path: primary, baseline: null };
}

export function formatReport(islands) {
  let out = 'islands: ' + islands.length + '\n';
  for (const it of islands) {
    out += it.crate + ' ' + it.file + ':' + it.line + ' ' + it.kind + ' ' + it.name + '\n';
  }
  const counts = countByCrate(islands);
  const ordered = Object.entries(counts).sort(function (a, b) {
    if (b[1] !== a[1]) {
      return b[1] - a[1];
    }
    return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0;
  });
  const limit = ordered.length < 10 ? ordered.length : 10;
  for (let i = 0; i < limit; i += 1) {
    out += 'top ' + ordered[i][1] + ' ' + ordered[i][0] + '\n';
  }
  return out;
}

// AD5 diff gate: a pub item added in the diff needs a production user in
// another file, or the caller names the open board row that will consume it.

const DIFF_KINDS = new Set(['fn', 'struct', 'enum']);

/** New-side line numbers per file in `git diff -U0` text. */
export function parseAddedLines(diffText) {
  const added = new Map();
  let file = null;
  for (const line of diffText.split('\n')) {
    if (line.startsWith('+++ ')) {
      file = line.startsWith('+++ b/') ? line.slice(6) : null;
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (!hunk || file === null) continue;
    const lines = added.get(file) ?? new Set();
    const count = hunk[2] === undefined ? 1 : Number(hunk[2]);
    for (let i = 0; i < count; i += 1) lines.add(Number(hunk[1]) + i);
    added.set(file, lines);
  }
  return added;
}

/** The fn, struct and enum islands defined on added lines (consts and traits
 * are API shape, the report lists them). `fresh` holds untracked files (every
 * line is new); `prefix` turns a crates-relative path into a repo path. */
export function newIslands(islands, added, fresh, prefix) {
  return islands.filter(function (it) {
    const path = prefix + it.file;
    return DIFF_KINDS.has(it.kind) && (fresh.has(path) || added.get(path)?.has(it.line) === true);
  });
}

/** Ids of the rows still open (`- [ ] ID`) on the board index and its shards. */
export function openBoardRows(docsDir) {
  const rows = new Set();
  for (const name of readdirSync(docsDir)) {
    if (!/^SPRINT(-.*)?\.md$/.test(name)) continue;
    for (const m of readFileSync(join(docsDir, name), 'utf8').matchAll(/^- \[ \] ([A-Z][A-Z0-9]*\d+[a-z]?)\b/gm)) rows.add(m[1]);
  }
  return rows;
}

/** Verdict over the new islands: `unlocks` (open row ids) exempts them all. */
export function diffVerdict(items, unlocks, openRows) {
  const unknown = unlocks.filter(function (row) { return !openRows.has(row); });
  if (unknown.length > 0) {
    return { code: 1, text: 'FAIL wiring-scan --diff: --unlocks names a row that is not open on the board: ' + unknown.join(', ') + '\n' };
  }
  const lines = items.map(function (it) { return it.crate + ' ' + it.file + ':' + it.line + ' ' + it.kind + ' ' + it.name; });
  if (items.length === 0) return { code: 0, text: 'PASS wiring-scan --diff: every new pub item has a production user in another file\n' };
  if (unlocks.length > 0) {
    return { code: 0, text: lines.map(function (l) { return 'unlocked ' + l + '\n'; }).join('') + 'PASS wiring-scan --diff: ' + items.length + ' new pub item(s) wait for ' + unlocks.join(', ') + '\n' };
  }
  return { code: 1, text: lines.map(function (l) { return 'unwired ' + l + '\n'; }).join('') + 'FAIL wiring-scan --diff: ' + items.length + ' new pub item(s) with no production user outside their own file; wire a caller or pass --unlocks=<open row id>\n' };
}

function runDiffGate(root, islands, base, unlocks) {
  const git = function (args) { return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }); };
  const top = git(['rev-parse', '--show-toplevel']).trim();
  const where = relative(top, root).split('\\').join('/');
  const prefix = where === '' ? '' : where + '/';
  const added = parseAddedLines(git(['diff', '-U0', '--no-color', base, '--', root]));
  const fresh = new Set(git(['ls-files', '--others', '--exclude-standard', '--full-name', '--', root]).split('\n').filter(Boolean));
  const openRows = unlocks.length > 0 ? openBoardRows(join(top, 'docs')) : new Set();
  return diffVerdict(newIslands(islands, added, fresh, prefix), unlocks, openRows);
}

export function helpText() {
  return [
    'wiring-scan.mjs - list pub items with no production user.',
    'usage: node tools/wiring-scan.mjs [root] [--json] [--baseline] [--check] [--diff[=REV]] [--unlocks=ROW[,ROW]] [--orphans] [--help]',
    '  root        crates root to scan (default: crates).',
    '  --json      print islands as one JSON line; byte-equal x2 for same input.',
    '  --baseline  write tools/wiring-scan.baseline.json on first run,',
    '              else exit 1 when any crate island count rises.',
    '  --check     compare current scan to the baseline pin, exit 1 on rise.',
    '  --diff[=REV] exit 1 when a pub item added since REV (default HEAD, working tree',
    '              and untracked files included) has no production user in another file.',
    '  --unlocks=ROW[,ROW]  with --diff: the new items wait for these open board rows.',
    '  --orphans   the default report: every existing pub item with no production',
    '              user, report only, nothing is deleted.',
    '  --help      print this help and exit 0.',
    'report: prints island lines, top ten crates, writes target/reports/wiring-scan.json.',
    'nightly: run `node tools/wiring-scan.mjs --check` and attach the report.',
    ''
  ].join('\n');
}

const isMain = import.meta.main;
if (isMain) {
  const raw = process.argv.slice(2);
  let wantJson = false;
  let wantBaseline = false;
  let wantCheck = false;
  let wantHelp = false;
  let wantDiff = false;
  let diffBase = 'HEAD';
  let unlocks = [];
  let arg = 'crates';
  let picked = false;
  for (const a of raw) {
    if (a === '--json') {
      wantJson = true;
    } else if (a === '--baseline') {
      wantBaseline = true;
    } else if (a === '--check') {
      wantCheck = true;
    } else if (a === '--diff' || a.startsWith('--diff=')) {
      wantDiff = true;
      if (a.length > 6) diffBase = a.slice(7);
    } else if (a.startsWith('--unlocks=')) {
      unlocks = a.slice(10).split(',').filter(Boolean);
    } else if (a === '--help' || a === '-h') {
      wantHelp = true;
    } else if (a.startsWith('--')) {
      continue;
    } else if (!picked) {
      arg = a;
      picked = true;
    }
  }
  if (wantHelp) {
    process.stdout.write(helpText());
    process.exitCode = 0;
  } else {
  const root = resolve(arg);
  const toolDir = dirname(fileURLToPath(import.meta.url));
  const mode = wantDiff ? 'diff' : wantCheck ? 'check' : wantBaseline ? 'baseline' : wantJson ? 'json' : 'report';
  runWithVerdictCache({ repoDir: dirname(toolDir), name: 'wiring-scan', extra: mode + '|' + root,
    enabled: mode !== 'baseline' && mode !== 'diff', // --baseline writes files and --diff reads git: never replay them
    run: function () {
  const islands = scanIslands(root);
  if (wantDiff) {
    const verdict = runDiffGate(root, islands, diffBase, unlocks);
    process.stdout.write(verdict.text);
    process.exitCode = verdict.code;
  } else if (wantCheck) {
    const found = readBaselineFile(toolDir);
    if (!found.baseline) {
      process.stdout.write('FAIL wiring-scan: missing baseline ' + found.path + ' islands: ' + islands.length + '\n');
      process.exitCode = 1;
    } else {
      const counts = countByCrate(islands);
      const baseCounts = baselineCounts(found.baseline);
      const baseTotal = baselineTotal(found.baseline);
      const risen = checkRise(counts, baseCounts);
      const totalRise = islands.length - baseTotal;
      if (totalRise > 0 || risen.length > 0) {
        for (const name of risen) {
          const was = baseCounts[name] || 0;
          process.stdout.write('rise ' + name + ' ' + was + ' -> ' + counts[name] + '\n');
        }
        process.stdout.write('FAIL wiring-scan: islands: ' + islands.length + ' baseline: ' + baseTotal + '\n');
        process.exitCode = 1;
      } else {
        process.stdout.write('PASS wiring-scan: islands: ' + islands.length + ' baseline: ' + baseTotal + '\n');
        process.exitCode = 0;
      }
    }
  } else if (wantBaseline) {
    const basePath = join(toolDir, BASELINE_FILE);
    const legacyPath = join(toolDir, LEGACY_BASELINE_FILE);
    const next = baselineOf(islands);
    if (!existsSync(basePath)) {
      const text = JSON.stringify(next, null, 2) + '\n';
      writeFileSync(basePath, text);
      // Keep the legacy accept-path name in sync on first write.
      writeFileSync(legacyPath, text);
      process.stdout.write('wrote baseline ' + basePath + ' islands: ' + next.total + '\n');
      process.exitCode = 0;
    } else {
      let baseline = {};
      try {
        baseline = JSON.parse(readFileSync(basePath, 'utf8'));
      } catch {
        baseline = {};
      }
      const counts = countByCrate(islands);
      const risen = checkRise(counts, baselineCounts(baseline));
      for (const name of risen) {
        const was = baselineCounts(baseline)[name] || 0;
        process.stdout.write('rise ' + name + ' ' + was + ' -> ' + counts[name] + '\n');
      }
      process.exitCode = risen.length > 0 ? 1 : 0;
    }
  } else if (wantJson) {
    const parts = root.split(/[\\/]/);
    if (parts[parts.length - 1] === 'crates') {
      const outPath = join(dirname(root), 'target', 'reports', 'wiring-scan.json');
      try {
        mkdirSync(dirname(outPath), { recursive: true });
        writeFileSync(outPath, JSON.stringify(islands) + '\n');
      } catch (err) { console.warn(`WARN wiring-scan: report cache write skipped: ${err?.message ?? err}`); }
    }
    process.stdout.write(JSON.stringify(islands) + '\n');
  } else {
    const parts = root.split(/[\\/]/);
    if (parts[parts.length - 1] === 'crates') {
      const outPath = join(dirname(root), 'target', 'reports', 'wiring-scan.json');
      try {
        mkdirSync(dirname(outPath), { recursive: true });
        writeFileSync(outPath, JSON.stringify(islands) + '\n');
      } catch (err) { console.warn(`WARN wiring-scan: report cache write skipped: ${err?.message ?? err}`); }
    }
    process.stdout.write(formatReport(islands));
  }
    },
  });
  }
}
