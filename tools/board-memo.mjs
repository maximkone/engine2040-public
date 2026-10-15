 // board-memo: memoized board parse. Cuts repeated board re-reads.
 // Same input text gives same output as parseBoard in board-slice.mjs.
 // Cache is exact-text hits plus file stat checks. No behavior change.
 import { readFileSync, statSync } from "node:fs";
 import { dirname, join } from "node:path";
 import { fileURLToPath } from "node:url";
 import { archiveDoneFiles, archivedDoneRows, boardShardFiles, loadBoardText, parseBoard } from "./board-slice.mjs";
 const HERE = dirname(fileURLToPath(import.meta.url));
 const ROOT = dirname(HERE);
 const textCache = new Map();
 const fileCache = new Map();
 let parseCount = 0;
 let fileReadCount = 0;
 let memoHits = 0;
 function statKey(root = ROOT) {
   const dir = root === ROOT ? join(ROOT, "docs") : join(root, "docs");
   const files = [join(dir, "SPRINT.md"), ...boardShardFiles(root), ...archiveDoneFiles(root)];
   return files.map((f) => {
     const st = statSync(f);
     return `${f}:${st.mtimeMs}:${st.size}`;
   }).join("|");
 }
 export function parseBoardMemo(text, archived = null) {
   const key = archived && archived.size ? `${text}\u0000${[...archived].join("\u0001")}` : text;
   const hit = textCache.get(key);
   if (hit !== undefined) {
     memoHits += 1;
     return hit;
   }
   parseCount += 1;
   const value = parseBoard(text, archived);
   textCache.set(key, value);
   return value;
 }
 export function loadBoardMemo(root = ROOT) {
   const key = statKey(root);
   const hit = fileCache.get(root);
   if (hit !== undefined && hit.key === key) {
     memoHits += 1;
     return hit.value;
   }
   fileReadCount += 1;
   const text = loadBoardText(root);
   const parsed = parseBoardMemo(text, archivedDoneRows(root));
   const value = { text, parsed };
   fileCache.set(root, { key, value });
   return value;
 }
 export function getBoardMemoStats() {
   return { parses: parseCount, reads: fileReadCount, hits: memoHits };
 }
 export function clearBoardMemo() {
   textCache.clear();
   fileCache.clear();
   parseCount = 0;
   fileReadCount = 0;
   memoHits = 0;
 }
 // Deterministic fixture: N board rows with stable ids and needs.
// About 300 bytes per row, so 2000 rows is about 600KB like docs/SPRINT.md.
const GOLDEN_SEED = 2040;
export function makeBoardFixture(rows = 2000, seed = GOLDEN_SEED) {
   let state = seed >>> 0;
   const out = ["### Fixture (owner test, `docs/plan/fixture.md`)", ""];
   for (let i = 0; i < rows; i += 1) {
     state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
     const id = `T${1000 + i}`;
     const done = state % 5 === 0 ? "x" : " ";
     const need = state % 3 === 0 ? "needs: none." : `needs: T${1000 + (state % (i + 1))}.`;
     out.push(`- [${done}] ${id} fixture row ${state % 100000} \`tools/fixture-${state % 97}.mjs\` ${need}`);
     out.push(`  detail line ${state % 1000} for row ${id}.`);
     out.push("");
   }
   return out.join("\n");
 }
 // Direct parse used only to measure the unmemoized cost.
 export function parseBoardDirect(text) {
   return parseBoard(text);
 }