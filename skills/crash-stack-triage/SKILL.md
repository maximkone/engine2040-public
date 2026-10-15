---
name: crash-stack-triage
description: Use when a run crashes and a stack or panic log names frames to map before any fix starts
version: 0.1.0
author: skillworks
---

# Crash stack triage

Complements bug-report-writing packet gates with diagnosis. Map the stack to one owner, shrink the repro, file one packet. Fail one check, send the packet back.

1. **Top frames list.** Ask: are the top 10 frames copied word for word with crate plus file plus line? Flag: trimmed summary or only the panic message. Crate: core.
2. **Crate owner map.** Ask: is each top frame mapped to one owning crate via the docs/CRATE-MAP.md head table? Flag: guessed owner or whole workspace blamed. Crate: core.
3. **Env split.** Ask: is env ruled out first with build hash plus seed plus OS plus device stated? Flag: code blamed while build or device is unknown. Crate: proof.
4. **Smallest repro.** Ask: is there one smallest command with seed and tick count that repeats the crash? Flag: full suite run or no repeat rate. Crate: proof.
5. **One bug packet.** Ask: is there one bug packet with a fingerprint of the top 3 frames plus owner crate plus repro command? Flag: two packets for one stack or no fingerprint. Crate: arena.
6. **No code revert on env crash.** Ask: on an env-only crash does the packet ask for a rerun on a clean build, never a code revert? Flag: code revert proposed with no local frame. Crate: perf.

## Do / Don't
- Do: name one owner crate and one repro command so any seat can redo it. Don't: edit code, revert, or file a second packet for the same fingerprint.

## Test note
- Paste a 10-frame stack, map the top frames, file the packet.
- Rerun the repro command twice, same fingerprint both times.
- Env-only stack ends in a rerun request, no code diff.

source: Composio sentry-triage, ported shape; diagnosis complements Finska, Modern Game Testing, ch. 8
