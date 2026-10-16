// inbox-open: only the open items, newest last in the file, cut at 240 characters.
import { test } from "node:test";
import assert from "node:assert/strict";
import { openItems, render } from "./inbox-open.mjs";

const doc = [
  "# Inbox",
  "",
  "## Open",
  "",
  ...Array.from({ length: 12 }, (_, i) => `- [ ] EB-${i + 1} from center | item ${i + 1} ${i === 11 ? "x".repeat(300) : "text"}`),
  "- [x] already ticked, not open",
  "",
  "## Done",
  "- [ ] under Done, not open",
  "",
].join("\n");

test("openItems keeps only the unticked lines under ## Open", () => {
  const items = openItems(doc);
  assert.equal(items.length, 12);
  assert.ok(items.every((l) => l.startsWith("- [ ] EB-")));
  assert.equal(openItems("# no open section\n- [ ] x\n"), null);
});

test("render prints the count, the 10 newest first, cut at 240, and the full-text pointer", () => {
  const out = render(openItems(doc)).split("\n");
  assert.equal(out[0], "open 12 item(s) in docs/EMPIRE-INBOX.md (## Open)");
  const shown = out.filter((l) => l.startsWith("- [ ] "));
  assert.equal(shown.length, 10);
  assert.ok(shown[0].startsWith("- [ ] EB-12 "), "newest first");
  assert.ok(shown.at(-1).startsWith("- [ ] EB-3 "), "the 10th newest is EB-3");
  assert.ok(shown.every((l) => l.length <= 240), "every item is cut at 240 characters");
  assert.equal(out.at(-1), "full text: docs/EMPIRE-INBOX.md");
});
