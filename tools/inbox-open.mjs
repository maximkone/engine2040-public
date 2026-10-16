// inbox-open: the open part of docs/EMPIRE-INBOX.md, so the lead reads a few KB, not the file.
//   node tools/inbox-open.mjs   open count, the 10 newest open items cut at 240 chars, and where the full text is
// Open items are the "- [ ]" lines under "## Open". Newest = last in the file (new items are appended).
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const INBOX = "docs/EMPIRE-INBOX.md";
export const NEWEST = 10;
export const CUT = 240;

// The "- [ ]" lines between "## Open" and the next "## " heading, or null when there is no Open section.
export const openItems = (text) => {
  const lines = String(text).replace(/\r\n/g, "\n").split("\n");
  const start = lines.findIndex((l) => /^## Open\s*$/.test(l));
  if (start < 0) return null;
  let end = lines.findIndex((l, i) => i > start && /^## /.test(l));
  if (end < 0) end = lines.length;
  return lines.slice(start + 1, end).filter((l) => /^- \[ \]/.test(l));
};

export const render = (items) => {
  const cut = (s) => (s.length > CUT ? `${s.slice(0, CUT - 3)}...` : s);
  const newest = items.slice(-NEWEST).reverse();
  return [
    `open ${items.length} item(s) in ${INBOX} (## Open)`,
    `newest ${newest.length}, newest first, cut at ${CUT} chars:`,
    ...newest.map(cut),
    `full text: ${INBOX}`,
  ].join("\n");
};

if (import.meta.main) {
  const file = join(ROOT, INBOX);
  const items = existsSync(file) ? openItems(readFileSync(file, "utf8")) : null;
  if (!items) {
    console.error(`RESULT FAIL: no "## Open" section in ${INBOX}`);
    process.exitCode = 2;
  } else {
    console.log(render(items));
  }
}
