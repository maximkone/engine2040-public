---
role: smith
seat: smith
title: Example packet: one skill edit with a proof
row: loop:orchestrate-template
paths: skills/engine-orchestrate/packet-template.md
test: node --test tools/queue-cap.test.mjs
max_diff: 160
max_lines: 160
wave: 1
source: skills/engine-orchestrate/SKILL.md (Dispatch section)
---
Goal: one outcome in one sentence, naming the file it changes.

Scope: the owned paths in the front matter and no others. Say what must not change.

Proof: the command and the exact output line that means done.

Stop: 25 minutes, or the second identical failure. Then report PARTIAL with the error text.

End with `RESULT: DONE|PARTIAL|BLOCKED|NOOP - <what> | proof: <where>`.
