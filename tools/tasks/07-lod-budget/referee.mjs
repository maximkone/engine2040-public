#!/usr/bin/env node
// Task 7 referee: the eval budget is met by the far ring, never by freezing the
// nearest fighters. The mutation puts the nearest eight on the far period: red.
//
//   node tools/tasks/07-lod-budget/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/07-lod-budget/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const FIGHTERS = 64;
const TICKS = 64;
const NEAREST = 8;
const BUDGET = 2000;
const FAR_EVERY = 8;

function table({ farEvery, nearFull = true }) {
  const lanes = [];
  for (let fighter = 0; fighter < FIGHTERS; fighter += 1) {
    const near = fighter < NEAREST;
    lanes.push({ fighter, tier: near && nearFull ? 'full' : 'far', every: near && nearFull ? 1 : farEvery });
  }
  return `${JSON.stringify({ fighters: FIGHTERS, ticks: TICKS, budget_evals: BUDGET, nearest_full: NEAREST, far_every: FAR_EVERY, lanes }, null, 2)}\n`;
}

const start = { 'tiers.json': table({ farEvery: 2 }) };
const fixed = { 'tiers.json': table({ farEvery: FAR_EVERY }) };
// The cheapest way to fit: the nearest fighters join the far ring.
const mutation = { 'tiers.json': table({ farEvery: FAR_EVERY, nearFull: false }) };

export const task = defineTask({
  id: '07-lod-budget',
  title: 'fit the LOD eval budget',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const out = json(files, 'tiers.json');
    const lanes = Array.isArray(out.lanes) ? out.lanes : [];
    const { fighters, ticks, budget_evals: budget, nearest_full: nearest, far_every: farEvery } = out;
    if (lanes.length !== fighters) failures.push(`${lanes.length} lane(s) for ${fighters} fighter(s)`);
    const ids = new Set();
    let evals = 0;
    for (const lane of lanes) {
      if (!Number.isInteger(lane.fighter) || lane.fighter < 0 || lane.fighter >= fighters) {
        failures.push(`lane has no valid fighter id: ${JSON.stringify(lane)}`);
        continue;
      }
      if (ids.has(lane.fighter)) failures.push(`fighter ${lane.fighter} has two lanes`);
      ids.add(lane.fighter);
      if (!Number.isInteger(lane.every) || lane.every < 1) {
        failures.push(`fighter ${lane.fighter} has every=${lane.every}`);
        continue;
      }
      evals += Math.ceil(ticks / lane.every);
      if (lane.fighter < nearest) {
        if (lane.tier !== 'full' || lane.every !== 1) failures.push(`nearest fighter ${lane.fighter} is ${lane.tier}/${lane.every}: the nearest ${nearest} read every tick`);
      } else if (lane.tier !== 'far' || lane.every !== farEvery) {
        failures.push(`far fighter ${lane.fighter} is ${lane.tier}/${lane.every}, the far period is ${farEvery}`);
      }
    }
    if (evals > budget) failures.push(`${evals} evals over ${ticks} ticks, the budget is ${budget}`);
    return failures;
  },
});

cli(task);
