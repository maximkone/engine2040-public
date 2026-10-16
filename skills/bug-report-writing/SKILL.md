---
name: bug-report-writing
description: Use when a runner, perf, or walker seat files or reviews a bug queue packet before a builder starts; it gates steps, expected, actual, build and proof.
---
# Bug report writing

Nine packet gates. Each names its queue field. Fail one field, send the packet back.

1. **Title (title).** Ask: does one line name platform, area, symptom? Flag: vague "broken" or "does not work". Crate: playtest.
2. **Row (row).** Ask: is the board row id named and open? Flag: no row, closed row. Crate: arena.
3. **Paths (paths).** Ask: are owned files listed, none outside the packet? Flag: whole crate, missing file. Crate: render.
4. **Test (test).** Ask: is the failing command named with crate and filter? Flag: "run tests" with no command. Crate: proof.
5. **Build (source).** Ask: are build hash, date, device and OS named? Flag: "latest" with no version. Crate: perf.
6. **Goal (Goal).** Ask: are expected and actual stated in one line each? Flag: only actual, no expected. Crate: physics.
7. **Steps (Scope).** Ask: are numbered steps short, one action each, with preconditions? Flag: merged or skipped steps. Crate: net.
8. **Proof (Proof).** Ask: is there a capture or log plus repro rate x/y over 5 tries? Flag: no capture, rate 1/1. Crate: core.
9. **Stop (Stop).** Ask: is severity set by player impact, priority left to producer? Flag: "must fix" in text, severity as urgency. Crate: assets.

## Do / Don't
- Do: write facts plus one capture so any seat can redo it. Don't: write opinions, fixes, or "must fix" in the report.

source: Finska, Modern Game Testing, ch. 8
