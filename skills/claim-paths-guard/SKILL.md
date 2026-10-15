---
name: claim-paths-guard
description: "Use when a sprint helper call comes back seat guard blocked write no packet paths or out of packet paths before any edit or shell write in engine2040."
---

# Write only inside claimed packet paths

Seat guard blocks writes outside the packet paths. Rerunning the same
write blocks again. The second identical failure ends the step.

Cause (from target/loop-keeper.log): `no packet paths: <path> is not
claimed` means no `paths:` line and no claim line owns it.
`out of packet paths: <path>; allowed: <a>, <b>` means the path is
outside the claimed allow-list. Miner and smith runs hit both on
`skills/*` and `tools/*.test.mjs` writes.

## Claim first, then write (pwsh, engine2040)

- Read the allow-list before any Write, Edit, or shell redirect:
  `Get-Content target/queue/claims.txt -Tail 20` finds your run line
  and its allowed paths. No line for your run means stop.
- Write only a path the allow-list names. Never guess a sibling
  skill or test path. A sibling name (`sky-bake` vs `msaa`) is a stop,
  not a retry.
- A new path needs a claim first, append only:
  `Add-Content -Path target/queue/claims.txt -Value "<run> | <UTC yyyy-mm-ddTHH:MMZ> | <item> <exact files>"`.
  Never `Set-Content` there. Read claims first and skip paths claimed
  in the last 2 h.
- Shell writes count like edits. A redirect, `Set-Content`,
  `Copy-Item`, or `node` write to a path outside the allow-list is
  blocked the same way. Check the target path before the command.
- First block ends the step path: stop, re-read the allow-list, and
  either claim the path or park the item. Never repeat the same write
  unchanged. Report `RESULT: BLOCKED - no-owned-paths <id>` with the
  guard text quoted.

## When held

Do not retry the blocked path. Claim it, take an allowed path, or
take nothing. A run that takes nothing writes no claim line.
