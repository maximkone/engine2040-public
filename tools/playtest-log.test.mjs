// playtest-log.test.mjs: one sample log gives one pass and one fail line.

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { checkCsvText } from "./playtest-log.mjs";

const SAMPLE =
  "tester,in_audience,build_sha,minute,confusion_point,question,yes_no,what,where,why,conclusion,bug_id,severity\r\n" +
  "P1,yes,abc123,4,blocked once,What blocks you?,yes,tried block,arena center,saw no hint,add block hint,,B\r\n" +
  "P2,yes,abc123,6,lost,no,where,,,,,,\r\n";

describe("playtest-log", () => {
  it("parses one sample log into pass and fail lines", () => {
    const res = checkCsvText(SAMPLE);
    assert.equal(res.total, 2);
    assert.equal(res.pass, 1);
    assert.equal(res.fail, 1);
    assert.equal(res.lines.length, 2);
    assert.equal(res.lines[0].ok, true);
    assert.equal(res.lines[0].row, 2);
    assert.equal(res.lines[1].ok, false);
    assert.equal(res.lines[1].row, 3);
  });
});
