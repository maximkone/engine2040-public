#!/usr/bin/env node
// The build-task suite: ten real engine jobs, each with a referee that runs the
// checks itself (no model, no judge), plus a scorer that reads a run log and
// prints pass@1 and pass@k per model.
//
//   node tools/tasks/run.mjs --selftest
//   node tools/tasks/run.mjs --log <run-log.jsonl> [--k 3]
//   node tools/tasks/run.mjs --list
//
// A task folder holds a `task.md` (the job for a solver) and a `referee.mjs`
// (exported from lib.mjs): `--apply start|fixed|mutation --dest <dir>` is the
// patch hook that writes the tree, `--tree <dir>` judges one. Selftest proves
// each referee green on its fixed tree and red on its mutation and start tree.
//
// A run log is JSONL: {"model": "...", "task": "...", "attempt": 1, "passed": true}
// One row per recorded attempt, in order. pass@k counts a task as solved when
// any of its first k attempts passed; pass@1 is attempt 1 only.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const CANNED_LOG = path.join(HERE, 'logs', 'canned.jsonl');
export const VARIANTS = ['start', 'fixed', 'mutation'];
export const EXPECTED = { start: 1, fixed: 0, mutation: 1 };

const USAGE = `usage: node tools/tasks/run.mjs <command> [options]

  --selftest        every referee on its start, fixed and mutation tree, then
                    the canned log scored into a pass@1 / pass@k table
  --log <file>      score one run log (JSONL) at --k attempts per task (default 3)
  --list            the ten tasks and their titles
  --k <n>           attempts per task for pass@k (default 3)
  --help            this text`;

export class ScoreError extends Error {}

/** The task folders, sorted by name: the suite is exactly these ten. */
export function taskFolders(root = HERE) {
  return fs
    .readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && /^\d\d-[a-z0-9-]+$/.test(entry.name))
    .map((entry) => entry.name)
    .sort();
}

/** Parse a run log into rows; a malformed line is a refusal, not a skip. */
export function parseLog(text, where = 'log') {
  const rows = [];
  for (const [index, line] of text.split('\n').entries()) {
    const trimmed = line.trim();
    if (trimmed === '') continue;
    let row;
    try {
      row = JSON.parse(trimmed);
    } catch (error) {
      throw new ScoreError(`${where}:${index + 1}: not JSON (${error.message})`);
    }
    if (typeof row.model !== 'string' || row.model.trim() === '') throw new ScoreError(`${where}:${index + 1}: no model`);
    if (typeof row.task !== 'string' || row.task.trim() === '') throw new ScoreError(`${where}:${index + 1}: no task`);
    if (!Number.isInteger(row.attempt) || row.attempt < 1) throw new ScoreError(`${where}:${index + 1}: attempt must be an integer >= 1`);
    if (typeof row.passed !== 'boolean') throw new ScoreError(`${where}:${index + 1}: passed must be true or false`);
    rows.push({ model: row.model, task: row.task, attempt: row.attempt, passed: row.passed });
  }
  return rows;
}

/** pass@1 and pass@k per model: a task counts once, at its first k attempts. */
export function scoreAttempts(rows, k = 3) {
  if (!Number.isInteger(k) || k < 1) throw new ScoreError(`k must be an integer >= 1, got ${k}`);
  const models = new Map();
  for (const row of rows) {
    if (!models.has(row.model)) models.set(row.model, new Map());
    const tasks = models.get(row.model);
    if (!tasks.has(row.task)) tasks.set(row.task, []);
    tasks.get(row.task).push(row);
  }
  const table = [];
  for (const [model, tasks] of models) {
    let first = 0;
    let within = 0;
    let attempts = 0;
    for (const runs of tasks.values()) {
      const ordered = [...runs].sort((a, b) => a.attempt - b.attempt);
      attempts += ordered.length;
      const at1 = ordered.find((row) => row.attempt === 1);
      if (at1?.passed === true) first += 1;
      if (ordered.some((row) => row.attempt <= k && row.passed)) within += 1;
    }
    table.push({
      model,
      tasks: tasks.size,
      attempts,
      pass1: tasks.size === 0 ? 0 : first / tasks.size,
      passK: tasks.size === 0 ? 0 : within / tasks.size,
    });
  }
  return table.sort((a, b) => b.passK - a.passK || a.model.localeCompare(b.model));
}

/** The table as an aligned text block: model, tasks, attempts, pass@1, pass@k. */
export function renderTable(table, k = 3) {
  const head = ['model', 'tasks', 'attempts', 'pass@1', `pass@${k}`];
  const body = table.map((row) => [row.model, String(row.tasks), String(row.attempts), row.pass1.toFixed(3), row.passK.toFixed(3)]);
  const widths = head.map((title, index) => Math.max(title.length, ...body.map((cells) => cells[index].length)));
  const line = (cells) => cells.map((cell, index) => cell.padEnd(widths[index])).join('  ').trimEnd();
  return [line(head), ...body.map(line)].join('\n');
}

/** One referee verdict: its exit code and the line it printed. */
export function refereeVerdict(referee, variant, dir, runner = execFileSync) {
  let status = 0;
  let output = '';
  try {
    output = String(runner(process.execPath, [referee, '--apply', variant, '--dest', dir], { encoding: 'utf8' })).trim();
    output = String(runner(process.execPath, [referee, '--tree', dir], { encoding: 'utf8' })).trim();
  } catch (error) {
    status = typeof error.status === 'number' ? error.status : 1;
    output = String(error.stdout ?? '').trim() || String(error.message).trim();
  }
  return { status, output };
}

/** Run every referee on every variant; returns one row per task. */
export function runReferees(root = HERE) {
  const folders = taskFolders(root);
  const rows = [];
  for (const folder of folders) {
    const referee = path.join(root, folder, 'referee.mjs');
    if (!fs.existsSync(referee)) {
      rows.push({ task: folder, verdicts: { start: 'missing', fixed: 'missing', mutation: 'missing' } });
      continue;
    }
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), `e2040-task-${folder}-`));
    const verdicts = {};
    try {
      for (const variant of VARIANTS) {
        const { status } = refereeVerdict(referee, variant, path.join(dir, variant));
        verdicts[variant] = status === EXPECTED[variant] ? (EXPECTED[variant] === 0 ? 'green' : 'red') : `got ${status}`;
      }
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
    rows.push({ task: folder, verdicts });
  }
  return rows;
}

/** The referee matrix as a table: task, start, fixed, mutation. */
export function renderReferees(rows) {
  const head = ['task', ...VARIANTS];
  const body = rows.map((row) => [row.task, ...VARIANTS.map((variant) => row.verdicts[variant])]);
  const widths = head.map((title, index) => Math.max(title.length, ...body.map((cells) => cells[index].length)));
  const line = (cells) => cells.map((cell, index) => cell.padEnd(widths[index])).join('  ').trimEnd();
  return [line(head), ...body.map(line)].join('\n');
}

function fail(message) {
  process.stdout.write(`tasks: FAIL: ${message}\n`);
  return 1;
}

function commandSelftest() {
  const rows = runReferees();
  process.stdout.write(`${renderReferees(rows)}\n\n`);
  if (rows.length !== 10) return fail(`expected ten task folders, found ${rows.length}`);
  const wrong = rows.filter((row) => VARIANTS.some((variant) => row.verdicts[variant] !== (EXPECTED[variant] === 0 ? 'green' : 'red')));
  if (wrong.length > 0) return fail(`${wrong.length} referee(s) gave the wrong verdict: ${wrong.map((row) => row.task).join(', ')}`);
  const green = rows.filter((row) => row.verdicts.fixed === 'green').length;
  const redMutation = rows.filter((row) => row.verdicts.mutation === 'red').length;
  process.stdout.write(`referees: ${green}/${rows.length} green on fixed, ${redMutation}/${rows.length} red on mutation, ${rows.length}/${rows.length} red on start\n\n`);

  // The scorer has one hand-computed answer, independent of the canned log.
  const unit = scoreAttempts(
    [
      { model: 'x', task: 'a', attempt: 1, passed: false },
      { model: 'x', task: 'a', attempt: 2, passed: true },
      { model: 'x', task: 'b', attempt: 1, passed: true },
    ],
    2,
  );
  if (unit.length !== 1 || unit[0].tasks !== 2 || unit[0].pass1 !== 0.5 || unit[0].passK !== 1) {
    return fail(`scorer unit check: ${JSON.stringify(unit)}`);
  }

  const log = parseLog(fs.readFileSync(CANNED_LOG, 'utf8'), 'canned.jsonl');
  const k = 3;
  const table = scoreAttempts(log, k);
  process.stdout.write(`${renderTable(table, k)}\n\n`);
  if (table.length !== 3) return fail(`canned log: expected 3 models, got ${table.length}`);
  for (const row of table) {
    if (row.tasks !== 10) return fail(`canned log: ${row.model} has ${row.tasks} tasks, expected 10`);
    if (row.pass1 > row.passK) return fail(`canned log: ${row.model} pass@1 ${row.pass1} > pass@${k} ${row.passK}`);
  }
  process.stdout.write(`PASS tasks selftest: ${rows.length} referees, ${log.length} logged attempts, ${table.length} models x 10 tasks\n`);
  return 0;
}

function commandScore(args) {
  if (args.log === undefined) return fail('--log needs a path (or use --selftest)');
  const k = args.k === undefined ? 3 : Number(args.k);
  if (!Number.isInteger(k) || k < 1) return fail(`--k must be an integer >= 1, got ${args.k}`);
  const log = parseLog(fs.readFileSync(path.resolve(args.log), 'utf8'), path.basename(args.log));
  const table = scoreAttempts(log, k);
  process.stdout.write(`${renderTable(table, k)}\n`);
  process.stdout.write(`scored: ${log.length} attempts, ${table.length} model(s), pass@${k}\n`);
  return 0;
}

function commandList() {
  for (const folder of taskFolders()) {
    const taskPath = path.join(HERE, folder, 'task.md');
    const title = fs.existsSync(taskPath) ? fs.readFileSync(taskPath, 'utf8').split('\n')[0].replace(/^#\s*/, '') : '(no task.md)';
    process.stdout.write(`${folder}  ${title}\n`);
  }
  return 0;
}

function parseArgs(argv) {
  const out = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--selftest') out.selftest = true;
    else if (arg === '--list') out.list = true;
    else if (arg === '--help' || arg === '-h') out.help = true;
    else if (arg === '--log') out.log = argv[++index];
    else if (arg === '--k') out.k = argv[++index];
    else out._.push(arg);
  }
  return out;
}

function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    process.stdout.write(`${USAGE}\n`);
    return 0;
  }
  if (args.selftest) return commandSelftest();
  if (args.list) return commandList();
  if (args.log !== undefined) return commandScore(args);
  process.stdout.write(`${USAGE}\n`);
  return 1;
}

const invoked = process.argv[1] ? path.resolve(process.argv[1]) === fileURLToPath(import.meta.url) : false;
if (invoked) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`tasks: refuse: ${error.message}\n`);
    process.exitCode = 1;
  }
}
