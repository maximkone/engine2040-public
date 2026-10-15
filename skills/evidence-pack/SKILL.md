---
name: evidence-pack
description: "Use before writing DONE, green, a number, or a score in any report, packet, or review. Bundles one claim into a 5-part pack (command, exit, hash, excerpt, verdict) a second seat can replay. Read-only: it bundles evidence, never reruns builds."
---

# Evidence pack

Pack law: NO CLAIM WITHOUT A PACK. A claim of green, a number, or a score ships with a checkable 5-part pack, never as bare prose. This skill bundles evidence. It never reruns builds, edits code, or blesses a claim it cannot check.

## The 7 checks

1. **Pack law.** Each claim carries one pack. No pack means no claim, whatever the prose says.
2. **Command line.** Record the exact command verbatim: pwsh form, workdir, flags, seed. A second seat must rerun it as-is.
3. **Exit code and output hash.** Record the exit code plus the SHA hash of the exact output bytes. Hash what the command printed, not a summary of it.
4. **Log excerpt.** Quote at most 20 lines that prove the verdict: the lines showing pass or fail and the number. Add file:line where it applies.
5. **One-line verdict.** One line only: PASS or FAIL plus the measured number. No adjectives, no trust-me words.
6. **Replay pointer.** State how a second seat reruns the command line and matches the SHA. If the replay cannot match, the claim stays DRAFT.
7. **Stop rule.** Any missing part keeps the claim at DRAFT, never DONE. A bare claim without a pack is not a result.

## Report line

`EVIDENCE: <command> | exit <code> | sha <hash> | <verdict>`

## Friends

Pairs with engine-review and test-truth-rules by name only: review sets the evidence bar, test-truth sets what counts as proof, this skill sets the bundle shape.
