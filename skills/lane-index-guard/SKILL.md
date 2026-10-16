---
name: lane-index-guard
description: "Use before dispatch and land.mjs index check when another lane may own the same crate or the index"
version: 0.1.0
author: skillworks
---

# Lane index guard

Kills the 3-wave structural block. Foreign-dirt collisions on SP63-a1, LF4, EG11, DP1 plus oversize land REFUSED. Fires before dispatch and before land.mjs on index and claims overlap plus diffstat and ratchet caps. Coder-overlap-guard fires on a second coder; claim-paths-guard fires after a blocked write; this fires first on the index.

1. **Status clean.** `git status --porcelain -- <owned paths>` is empty or the packet waits. Never dispatch over foreign dirt.
2. **Claims scan.** No live line in `target/queue/claims.txt` owns an overlapping path or crate. Overlap means sequence, never two same-crate coders at once.
3. **Foreign files stay shut.** Another lane file such as quality.rs, sky.rs, rival.rs, or live.rs is never touched from outside its scope. Foreign breakage is owned, never fixed in passing.
4. **Diffstat cap.** `git diff --numstat` per slice stays at or under 200 changed lines. Over means split the packet.
5. **Ratchet file cap.** A production file over its ratchet cap such as post.rs at 1500 means split per CQ9 before any lift.
6. **Baseline rides along.** A land.mjs REFUSED on baseline means tighten and land in one commit. Never resend land until the baseline rides along.
7. **Restore is lead-only.** `git restore`, checkout, or reset never appears in a seat packet. That work belongs to the lead.

## Test note

- `git status --porcelain -- <owned paths>` shows clean before dispatch.
- `Select-String <crate or path> target/queue/claims.txt` shows no live overlap.
- `git diff --numstat` shows 200 lines or fewer per slice.
