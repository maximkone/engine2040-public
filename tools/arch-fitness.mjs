// Arch fitness: acyclic crate graph (docs/ARCHITECTURE.md:9).
// Richards ch6: a rule people must remember becomes a check that fails
// on a seeded violation. DFS over the engine-crate edges.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Find one cycle in engine-crate edges. Edges is a Map or a plain
// object from crate name to a list of crate names it may depend on.
// Returns an array like ["a", "b", "a"] or null when acyclic.
export function findCycle(edges) {
  const adj = new Map();
  if (edges instanceof Map) {
    for (const [from, to] of edges) adj.set(from, [...(to ?? [])]);
  } else {
    for (const from of Object.keys(edges)) adj.set(from, [...(edges[from] ?? [])]);
  }
  for (const to of adj.values()) {
    for (const name of to) {
      if (!adj.has(name)) adj.set(name, []);
    }
  }
  const state = new Map();
  const stack = [];
  let cycle = null;
  function visit(name) {
    state.set(name, 1);
    stack.push(name);
    for (const next of adj.get(name) ?? []) {
      if (cycle) break;
      const seen = state.get(next) ?? 0;
      if (seen === 0) visit(next);
      else if (seen === 1) cycle = [...stack.slice(stack.indexOf(next)), next];
    }
    stack.pop();
    state.set(name, 2);
  }
  for (const name of adj.keys()) {
    if ((state.get(name) ?? 0) === 0 && !cycle) visit(name);
  }
  return cycle;
}

// Read the real engine graph from the allow-list in workspace-check.mjs.
// The test uses this so the PASS proof tracks the shipped edges.
export function loadAllowedEdges(repoDir) {
  const text = readFileSync(join(repoDir, "tools", "workspace-check.mjs"), "utf8");
  const edges = new Map();
  const row = /\["(engine-[a-z0-9-]+)",\s*\[([^\]]*)\]\]/g;
  let hit;
  while ((hit = row.exec(text)) !== null) {
    const from = hit[1];
    const to = [...hit[2].matchAll(/"(engine-[a-z0-9-]+)"/g)].map((x) => x[1]);
    if (!edges.has(from)) edges.set(from, to);
  }
  return edges;
}

if (import.meta.main) {
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  const cycle = findCycle(loadAllowedEdges(repoDir));
  if (cycle) console.log(`FAIL cycle: ${cycle.join(" -> ")}`);
  else console.log("PASS dependency cycle");
}
