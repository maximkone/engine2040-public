#!/usr/bin/env node
// Task 9 referee: every docs/ page is in docs/INDEX.md with a purpose. The
// mutation adds the page and skips the index line: red.
//
//   node tools/tasks/09-doc-index/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/09-doc-index/referee.mjs --tree <dir>

import { cli, defineTask } from '../lib.mjs';

const README = `# Docs

The map of this crate's documents.
`;

const INDEX = `# Docs index

| path | purpose |
| --- | --- |
| docs/README.md | the docs map |
`;

const VERBS = `# Door verbs

\`scene.read\` reads the revision. \`tx.preview\` previews a change.
`;

const INDEX_LINE = '| docs/verbs.md | the door verbs |\n';

const start = { 'docs/README.md': README, 'docs/INDEX.md': INDEX };
const fixed = { 'docs/README.md': README, 'docs/INDEX.md': `${INDEX}${INDEX_LINE}`, 'docs/verbs.md': VERBS };
const mutation = { 'docs/README.md': README, 'docs/INDEX.md': INDEX, 'docs/verbs.md': VERBS };

export const task = defineTask({
  id: '09-doc-index',
  title: 'add a doc and its index line',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const index = files['docs/INDEX.md'];
    if (index === undefined) return ['docs/INDEX.md is missing'];
    const pages = Object.keys(files).filter((rel) => rel.startsWith('docs/') && rel.endsWith('.md') && rel !== 'docs/INDEX.md');
    for (const page of pages) {
      const line = index.split('\n').find((row) => row.includes(`| ${page} |`));
      if (line === undefined) {
        failures.push(`${page} has no index line in docs/INDEX.md`);
        continue;
      }
      const purpose = line.split('|')[2]?.trim() ?? '';
      if (purpose === '') failures.push(`the index line for ${page} has no purpose`);
    }
    const verbs = files['docs/verbs.md'];
    if (verbs === undefined) failures.push('docs/verbs.md is missing');
    else if (!verbs.trimStart().startsWith('# ')) failures.push('docs/verbs.md does not start with a # title');
    return failures;
  },
});

cli(task);
