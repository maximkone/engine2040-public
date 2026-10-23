#!/usr/bin/env node
// Task 4 referee: a board row is well formed only when its needs are real and
// its Accept names a tool in the tree. The mutation needs NOPE9 and accepts a
// missing tool: red.
//
//   node tools/tasks/04-board-row/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/04-board-row/referee.mjs --tree <dir>

import { cli, defineTask } from '../lib.mjs';

const ROW_RE = /^- \[([ x])\] ([A-Z]{1,3}\d+) (.*)$/;
const NEEDS_RE = /needs:\s*([A-Z]{1,3}\d+)/;
const ACCEPT_RE = /Accept:\s*node (tools\/[^\s.]+(?:\.mjs)?)/;

const SLICE = `#!/usr/bin/env node
// Fixture stand-in for tools/board-slice.mjs: prints one row.
process.stdout.write('board-slice: fixture\\n');
`;

const BOARD = `# Board (fixture)

- [x] EG48 agent: publish the gap list. Landed 1a2b3c4.
- [ ] EG49 agent: project memory and a build-task suite.
  needs: EG48. Accept: node tools/board-slice.mjs EG49. Owns tools/memory/**.
`;

const ROW = `- [ ] EG50 agent: serve the frame verb over the door.
  needs: EG49. Accept: node tools/board-slice.mjs EG50. Owns crates/endpoint/**.
`;

const start = { 'board.md': BOARD, 'tools/board-slice.mjs': SLICE };
const fixed = { 'board.md': `${BOARD}${ROW}`, 'tools/board-slice.mjs': SLICE };
// Needs an id nobody has, and accepts with a tool that is not in the tree.
const MUTATION_ROW = `- [ ] EG50 agent: serve the frame verb over the door.
  needs: ZZ9. Accept: node tools/no-such-tool.mjs EG50.
`;
const mutation = { 'board.md': `${BOARD}${MUTATION_ROW}`, 'tools/board-slice.mjs': SLICE };

/** Board rows as blocks: the row line plus the lines under it. */
function rows(text) {
  const lines = text.split('\n');
  const blocks = [];
  for (let index = 0; index < lines.length; index += 1) {
    const match = ROW_RE.exec(lines[index]);
    if (!match) continue;
    const body = [];
    for (let next = index + 1; next < lines.length && !ROW_RE.test(lines[next]); next += 1) body.push(lines[next]);
    blocks.push({ ticked: match[1] === 'x', id: match[2], text: match[3].trim(), body: body.join('\n'), line: index + 1 });
  }
  return blocks;
}

export const task = defineTask({
  id: '04-board-row',
  title: 'add a board row',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const board = files['board.md'];
    if (board === undefined) return ['board.md is missing'];
    const blocks = rows(board);
    const ids = new Set(blocks.map((row) => row.id));
    const added = blocks.filter((row) => row.id === 'EG50');
    if (added.length === 0) return ['the board has no EG50 row'];
    if (added.length > 1) failures.push(`the board lists EG50 ${added.length} times`);
    for (const row of added) {
      if (row.ticked) failures.push(`EG50 is ticked but it has no landed evidence`);
      if (row.text === '' || row.text === 'agent:') failures.push('the EG50 row has no text');
      const needs = NEEDS_RE.exec(row.body);
      if (!needs) failures.push('the EG50 row has no needs: line');
      else if (!ids.has(needs[1])) failures.push(`the EG50 row needs ${needs[1]}, which is not on the board`);
      const accept = ACCEPT_RE.exec(row.body);
      if (!accept) failures.push('the EG50 row has no Accept: node <tool> command');
      else if (files[accept[1]] === undefined) failures.push(`the EG50 row accepts with ${accept[1]}, which is not in the tree`);
    }
    return failures;
  },
});

cli(task);
