 // Run: node --test tools/board-memo.test.mjs
 // Pins memoized board parse identical to direct parse, cache hits, ms before -> after.
 import assert from "node:assert/strict";
 import test from "node:test";
 import { parseBoard } from "./board-slice.mjs";
 import { clearBoardMemo, getBoardMemoStats, makeBoardFixture, parseBoardDirect, parseBoardMemo } from "./board-memo.mjs";
 test("memo parse equals direct parse and returns same ref", () => {
   clearBoardMemo();
   const text = makeBoardFixture(50, 2040);
   const direct = parseBoardDirect(text);
   const a = parseBoardMemo(text);
   const b = parseBoardMemo(text);
   assert.equal(b, a);
   assert.deepEqual([...a.tasks.keys()], [...direct.tasks.keys()]);
   assert.deepEqual(a.order, direct.order);
   console.log(`identical: true order ${a.order.length} rows match direct parse`);
 });
 test("repeated parses hit cache: parses 2->1", () => {
   clearBoardMemo();
   const text = makeBoardFixture(500, 2040);
   parseBoardMemo(text);
   parseBoardMemo(text);
   const s = getBoardMemoStats();
   assert.equal(s.parses, 1);
   console.log(`parse-count 2->1: parses 2 -> ${s.parses}, hits ${s.hits}`);
 });
 test("repeated parses are faster: ms before -> after", () => {
   clearBoardMemo();
   const text = makeBoardFixture(2000, 2040);
   const runs = 30;
   const t0 = performance.now();
   for (let i = 0; i < runs; i += 1) parseBoard(text);
   const directMs = performance.now() - t0;
   parseBoardMemo(text);
   const t1 = performance.now();
   for (let i = 0; i < runs; i += 1) parseBoardMemo(text);
   const memoMs = performance.now() - t1;
   console.log(`benchmark 600KB x${runs}: direct ${directMs.toFixed(2)}ms -> memo ${memoMs.toFixed(2)}ms`);
   assert.deepEqual(parseBoardMemo(text).order, parseBoard(text).order);
   assert.ok(memoMs < directMs, `expected memo ${memoMs}ms < direct ${directMs}ms`);
 });