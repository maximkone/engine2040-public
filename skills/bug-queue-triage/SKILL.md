---
name: bug-queue-triage
description: Use when more than one bug packet is open in engine2040 and complements bug-report-writing with repro-first rank plus golden-hash impact before any assign
---

# Bug queue triage

Rank the queue before a builder starts. Complements bug-report-writing packet gates with rank order. Top rank builds first.

1. **Repro first.** Ask: is there a redo with steps plus rate x/y over 5 tries and a capture or log? Flag: rate 1/1 or no capture ranks last.
2. **Hash impact.** Ask: does the bug touch tick state, replay bytes, or the golden hash? Flag: hash-moving bugs rank above visual-only bugs.
3. **Smallest scope first.** Ask: are owned paths few and inside one crate? Flag: whole-crate or multi-crate packets rank after small ones.
4. **Max 2 asks.** Ask: does the packet need at most 2 walker asks to start? Flag: more than 2 open asks goes back to the walker first.
5. **Rank not age.** Ask: is build order by rank score, not oldest first? Flag: keeper picking by age over rank breaks the queue.
6. **Ready to build.** Ask: do all bug-report-writing gates pass with row open and test command named? Flag: failed gate means no rank, send back first.

## Do / Don't

- Do: score repro plus hash plus scope, then assign top rank first. Don't: assign by age or loudest voice.

## Test

- Open 3 bug packets with mixed repro rates, rank them, top has a redo plus hash note.
- Swap in one hash-moving bug, it outranks a visual-only bug with a better rate.
- Send one packet with 3 open asks back to the walker, assign only ranked-ready packets.

source: Composio, issue-triage (donor shape)
