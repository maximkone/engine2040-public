#!/usr/bin/env node
// The build-task suite: ten real engine jobs, each with a referee that runs the
// checks itself (no model, no judge), plus a scorer that reads a run log and
// prints pass@1 and pass@k per model, plus two arms that solve the suite for
// real and print their own pass@1.
//
//   node tools/tasks/run.mjs --selftest
//   node tools/tasks/run.mjs --log <run-log.jsonl> [--k 3]
//   node tools/tasks/run.mjs --solve --arm scripted|mutation
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
//
// `--solve` is the calibration of that score, not another canned table: for
// every task it writes the start tree, lays one arm's patch over it as a solver
// would, and asks the referee to judge the tree that results. The scripted arm
// lays each task's own reference fix, so it must print pass@1 1.000; the
// mutation arm lays each task's committed known-bad patch, so it must print
// pass@1 0.000. A referee that judges nothing would show up as the mutation arm
// scoring above zero. The arms are oracles, not models: they say what the
// referee can tell apart, which is the floor a real model is scored against.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const CANNED_LOG = path.join(HERE, 'logs', 'canned.jsonl');
export const VARIANTS = ['start', 'fixed', 'mutation'];
export const EXPECTED = { start: 1, fixed: 0, mutation: 1 };

/** The two solve arms, and the variant each one lays over the start tree. */
export const ARMS = { scripted: 'fixed', mutation: 'mutation' };

/** The pass@1 each arm must reach: the scripted arm solves, the mutant does not. */
export const ARM_EXPECTED = { scripted: 1, mutation: 0 };

const USAGE = `usage: node tools/tasks/run.mjs <command> [options]

  --selftest        every referee on its start, fixed and mutation tree, then
                    the canned log scored into a pass@1 / pass@k table, then the
                    two solve arms
  --log <file>      score one run log (JSONL) at --k attempts per task (default 3)
  --solve           solve the suite with one arm and print its real pass@1
  --arm <name>      one of ${Object.keys(ARMS).join('|')} (with --solve)
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

/** The table as an aligned text block: model, tasks, attempts, pass@1, pass@k.
 *  At `--k 1` the last column would repeat the first, so it is left out. */
export function renderTable(table, k = 3) {
  const head = k === 1 ? ['model', 'tasks', 'attempts', 'pass@1'] : ['model', 'tasks', 'attempts', 'pass@1', `pass@${k}`];
  const body = table.map((row) => {
    const cells = [row.model, String(row.tasks), String(row.attempts), row.pass1.toFixed(3)];
    if (k !== 1) cells.push(row.passK.toFixed(3));
    return cells;
  });
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

/** Run one referee with its arguments and keep the exit status it answered with. */
function refereeRun(referee, args, runner = execFileSync) {
  try {
    return { status: 0, output: String(runner(process.execPath, [referee, ...args], { encoding: 'utf8' })).trim() };
  } catch (error) {
    return {
      status: typeof error.status === 'number' ? error.status : 1,
      output: String(error.stdout ?? '').trim() || String(error.message).trim(),
    };
  }
}

/** The patch hook: write one variant tree under `dest`. */
export function applyVariant(referee, variant, dest, runner = execFileSync) {
  return refereeRun(referee, ['--apply', variant, '--dest', dest], runner);
}

/** The judgement: does this tree pass the referee? */
export function judgeTree(referee, dir, runner = execFileSync) {
  return refereeRun(referee, ['--tree', dir], runner);
}

/** Every file under `dir`, `/`-separated and relative, in name order. */
function treeFiles(dir) {
  const out = [];
  const walk = (current, prefix) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const abs = path.join(current, entry.name);
      const rel = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) walk(abs, rel);
      else if (entry.isFile()) out.push(rel);
    }
  };
  if (fs.existsSync(dir)) walk(dir, '');
  return out;
}

/** True when both paths hold the same bytes; a missing side never matches. */
function sameFile(one, other) {
  if (!fs.existsSync(one) || !fs.existsSync(other)) return false;
  return fs.readFileSync(one).equals(fs.readFileSync(other));
}

/** The files one patch changes or adds over the tree it is laid on. */
function patchFiles(start, patch) {
  return treeFiles(patch).filter((rel) => !sameFile(path.join(start, rel), path.join(patch, rel)));
}

/** A referee line, kept short enough for one table cell. */
function note(text) {
  const one = String(text).split('\n')[0];
  return one.length <= 72 ? one : `${one.slice(0, 69)}...`;
}

/**
 * Solve one task with one arm: write the start tree, lay the arm's patch over
 * it the way a solver's edit lands, then ask the referee to judge the tree that
 * results. Returns the task, the arm, whether it passed and how many files the
 * arm actually changed.
 */
export function solveOne(referee, arm, work, runner = execFileSync) {
  const task = path.basename(path.dirname(referee));
  const start = path.join(work, 'start');
  const patch = path.join(work, 'patch');
  const solved = path.join(work, 'solved');
  const wrote = applyVariant(referee, 'start', start, runner);
  if (wrote.status !== 0) return { task, arm, passed: false, patched: 0, note: note(`start tree: ${wrote.output}`) };
  const laid = applyVariant(referee, ARMS[arm], patch, runner);
  if (laid.status !== 0) return { task, arm, passed: false, patched: 0, note: note(`patch tree: ${laid.output}`) };
  fs.cpSync(start, solved, { recursive: true });
  const changed = patchFiles(start, patch);
  fs.cpSync(patch, solved, { recursive: true });
  const verdict = judgeTree(referee, solved, runner);
  const why = changed.length === 0 ? 'the arm laid no file the start tree did not already carry' : note(verdict.output);
  return { task, arm, passed: verdict.status === 0, patched: changed.length, note: why };
}

/** Solve the whole suite with one arm: one row per task, in task order. */
export function solveSuite(arm, root = HERE) {
  if (!Object.hasOwn(ARMS, arm)) throw new ScoreError(`unknown arm ${arm}: the arms are ${Object.keys(ARMS).join(', ')}`);
  const rows = [];
  for (const folder of taskFolders(root)) {
    const referee = path.join(root, folder, 'referee.mjs');
    if (!fs.existsSync(referee)) {
      rows.push({ task: folder, arm, passed: false, patched: 0, note: 'no referee.mjs in the task folder' });
      continue;
    }
    const work = fs.mkdtempSync(path.join(os.tmpdir(), `e2040-solve-${folder}-`));
    try {
      rows.push(solveOne(referee, arm, path.join(work, 'w')));
    } finally {
      fs.rmSync(work, { recursive: true, force: true });
    }
  }
  return rows;
}

/** The attempt rows one solve produced: every task at attempt 1. */
export function solveAttempts(rows, arm) {
  return rows.map((row) => ({ model: arm, task: row.task, attempt: 1, passed: row.passed }));
}

/** Score one solve: the pass@1 the referee answered with, and the tasks it took. */
export function scoreSolve(rows, arm) {
  const table = scoreAttempts(solveAttempts(rows, arm), 1);
  return {
    table,
    pass1: table.length === 1 ? table[0].pass1 : -1,
    solved: rows.filter((row) => row.passed).length,
  };
}

/** The solve table: task, how many files the arm patched, the verdict, its line. */
export function renderSolve(rows) {
  const head = ['task', 'patched', 'verdict', 'the referee'];
  const body = rows.map((row) => [row.task, String(row.patched), row.passed ? 'solved' : 'refused', row.note]);
  const widths = head.map((title, index) => Math.max(title.length, ...body.map((cells) => cells[index].length)));
  const line = (cells) => cells.map((cell, index) => cell.padEnd(widths[index])).join('  ').trimEnd();
  return [line(head), ...body.map(line)].join('\n');
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

  // The two arms solve the suite for real: every task's start tree gets the
  // arm's patch laid over it, and the referee judges the tree that results. The
  // scripted arm lays the reference fix, so it must reach pass@1 1.000; the
  // mutation arm lays the committed known-bad patch, so it must reach 0.000. A
  // referee that judged nothing would answer above zero for the mutant.
  for (const arm of Object.keys(ARMS)) {
    const solved = solveSuite(arm);
    const answer = scoreSolve(solved, arm);
    process.stdout.write(`${renderSolve(solved)}\n`);
    process.stdout.write(`solve ${arm}: pass@1 ${answer.pass1.toFixed(3)} (${answer.solved}/${solved.length} tasks solved)\n\n`);
    if (solved.length !== 10) return fail(`solve ${arm}: ${solved.length} task folders, expected 10`);
    if (answer.pass1 !== ARM_EXPECTED[arm]) {
      const wrong = solved.filter((row) => row.passed !== (ARM_EXPECTED[arm] === 1));
      return fail(`solve ${arm} expected pass@1 ${ARM_EXPECTED[arm].toFixed(3)}: ${wrong.map((row) => `${row.task} (${row.note})`).join('; ')}`);
    }
  }

  process.stdout.write(`PASS tasks selftest: ${rows.length} referees, ${log.length} logged attempts, ${table.length} models x 10 tasks, both solve arms at their pass@1\n`);
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

/** Solve the suite with one arm and print the pass@1 the referee gave it. */
function commandSolve(args) {
  const arm = args.arm;
  if (arm === undefined) return fail(`--solve needs --arm ${Object.keys(ARMS).join('|')}`);
  if (!Object.hasOwn(ARMS, arm)) return fail(`unknown arm ${arm}: the arms are ${Object.keys(ARMS).join(', ')}`);
  const rows = solveSuite(arm);
  process.stdout.write(`${renderSolve(rows)}\n\n`);
  const table = scoreAttempts(solveAttempts(rows, arm), 1);
  process.stdout.write(`${renderTable(table, 1)}\n`);
  const solved = rows.filter((row) => row.passed).length;
  const pass1 = table.length === 1 ? table[0].pass1 : -1;
  process.stdout.write(`solve ${arm}: pass@1 ${pass1.toFixed(3)} (${solved}/${rows.length} tasks solved)\n`);
  if (pass1 !== ARM_EXPECTED[arm]) {
    const wrong = rows.filter((row) => row.passed !== (ARM_EXPECTED[arm] === 1));
    return fail(`solve ${arm} expected pass@1 ${ARM_EXPECTED[arm].toFixed(3)}: ${wrong.map((row) => `${row.task} (${row.note})`).join(', ')}`);
  }
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
    else if (arg === '--solve') out.solve = true;
    else if (arg === '--arm') out.arm = argv[++index];
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
  if (args.solve) return commandSolve(args);
  if (args.arm !== undefined) return fail('--arm goes with --solve');
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
