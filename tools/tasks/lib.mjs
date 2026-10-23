// One home for the build-task suite: a task is a starting tree, a fix, a
// known-bad mutation and a referee that judges a tree by **running checks**.
// No model, no judge: a referee prints PASS or FAIL and exits 0 or 1.
//
// A task module exports one `defineTask({...})` value and calls `cli(task)`:
//
//   node tools/tasks/<task>/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/<task>/referee.mjs --tree <dir>
//
// `--apply` is the patch hook: it writes the variant's files under `--dest`.
// `--tree` is the referee: it reads the tree and runs the checks.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const VARIANTS = ['start', 'fixed', 'mutation'];

/** A refusal from a task definition, never from a solver tree. */
export class TaskError extends Error {}

/**
 * Define one task. `variants` maps each of `start`, `fixed` and `mutation` to
 * a `{ relpath: text }` tree. `checks(files)` returns the failure strings for
 * a tree; an empty list means the tree is fixed. It may be async.
 */
export function defineTask(task) {
  const { id, title, variants, checks } = task;
  if (typeof id !== 'string' || !/^[a-z0-9-]+$/.test(id)) {
    throw new TaskError(`task id must be lower case kebab-case: ${id}`);
  }
  if (typeof title !== 'string' || title.trim() === '') throw new TaskError(`${id}: no title`);
  if (typeof checks !== 'function') throw new TaskError(`${id}: no checks`);
  for (const variant of VARIANTS) {
    const files = variants?.[variant];
    if (!files || typeof files !== 'object' || Object.keys(files).length === 0) {
      throw new TaskError(`${id}: variant ${variant} has no files`);
    }
  }
  return { id, title, variants, checks };
}

/** Write a variant tree under `dir`; returns the absolute dir. */
export function writeTree(dir, files) {
  for (const [rel, text] of Object.entries(files)) {
    const target = path.join(dir, rel);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, text);
  }
  return path.resolve(dir);
}

/** Read every file under `dir`, keyed by its `/`-separated relative path. */
export function readTree(dir) {
  const out = {};
  const walk = (current, prefix) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const abs = path.join(current, entry.name);
      const rel = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) walk(abs, rel);
      else if (entry.isFile()) out[rel] = fs.readFileSync(abs, 'utf8');
    }
  };
  walk(dir, '');
  return out;
}

/** Parse a JSON file from a tree; throws a failure string when it will not. */
export function json(files, rel) {
  const text = files[rel];
  if (text === undefined) throw new TaskError(`${rel} is missing`);
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new TaskError(`${rel} is not JSON: ${error.message}`);
  }
}

/** Import one fixture module from a tree (as an ES module in a temp dir). */
export async function importFrom(files, rel) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'e2040-task-import-'));
  try {
    const file = path.join(dir, path.basename(rel));
    fs.writeFileSync(file, files[rel]);
    return await import(`${pathToFileURL(file).href}?v=${Date.now()}`);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/** Run one task's checks against a tree; returns the failure list. */
export async function judge(task, dir) {
  let files;
  try {
    files = readTree(dir);
  } catch (error) {
    return [`cannot read tree ${dir}: ${error.message}`];
  }
  try {
    return await task.checks(files);
  } catch (error) {
    // A refused tree names its own file; any other throw is a referee fault.
    return [error instanceof TaskError ? `bad tree: ${error.message}` : `check threw: ${error.message}`];
  }
}

function parseArgs(argv) {
  const out = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') out.help = true;
    else if (arg === '--apply') out.apply = argv[++index];
    else if (arg === '--dest') out.dest = argv[++index];
    else if (arg === '--tree') out.tree = argv[++index];
    else out._.push(arg);
  }
  return out;
}

/** The referee CLI shared by every task module. */
export function cli(task) {
  const usage = `usage: node referee.mjs --tree <dir>
       node referee.mjs --apply <${VARIANTS.join('|')}> --dest <dir>`;
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (error) {
    process.stdout.write(`referee ${task.id}: FAIL: bad arguments: ${error.message}\n`);
    process.exitCode = 1;
    return;
  }
  if (args.help) {
    process.stdout.write(`${usage}\n\ntask ${task.id}: ${task.title}\n`);
    return;
  }
  if (args.apply !== undefined) {
    if (!VARIANTS.includes(args.apply) || args.dest === undefined) {
      process.stdout.write(`referee ${task.id}: FAIL: --apply needs one of ${VARIANTS.join('|')} and --dest <dir>\n`);
      process.exitCode = 1;
      return;
    }
    const dir = writeTree(args.dest, task.variants[args.apply]);
    process.stdout.write(`referee ${task.id}: wrote ${args.apply} tree to ${dir}\n`);
    return;
  }
  if (args.tree === undefined) {
    process.stdout.write(`referee ${task.id}: FAIL: no --tree\n`);
    process.exitCode = 1;
    return;
  }
  judge(task, args.tree).then((failures) => {
    if (failures.length === 0) {
      process.stdout.write(`referee ${task.id}: PASS\n`);
      return;
    }
    process.stdout.write(`referee ${task.id}: FAIL: ${failures[0]}${failures.length > 1 ? ` (+${failures.length - 1} more)` : ''}\n`);
    process.exitCode = 1;
  });
}

/** The folder this referee file lives in, for discovery in run.mjs. */
export function taskDir(importMetaUrl) {
  return path.dirname(fileURLToPath(importMetaUrl));
}
