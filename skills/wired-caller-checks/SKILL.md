---
name: wired-caller-checks
description: Use when a diff adds or changes a pub fn, helper, or module in crates before review is booked
version: 0.1.0
author: skillworks
---

# Wired caller checks

Seven checks for a new helper. A new pub item without a production caller is FAIL, not repair.

1. **Name the caller.** Every new pub item cites its production caller path and line, or its UNLOCKS row. Neither means FAIL.
2. **Grep the caller.** `git grep` on the new name hits outside `tests/` and `#[cfg(test)]`, or the packet says tests-only.
3. **Consume the value.** No `let _ =` on a value the row promised to use. A dropped value means unwired.
4. **Output is not wiring.** A test that only asserts the helper output proves nothing about callers. It never counts as wired.
5. **No flip for a caller.** Changing `pub` back to private never stands in for a caller. The call path must exist.
6. **Split oversize islands.** Oversize plus unwired means split and wire. A repair never resends the whole island.
7. **Stop repeating.** A second identical unwired FAIL ends the patch. The third try starts from the caller, never the helper.

## Test note

Pick a new pub name from the diff.
Run `git grep` on that name.
PASS needs one hit outside `tests/`.
