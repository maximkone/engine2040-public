---
name: link-env-triage
description: Use when a crate test binary fails to link with rust-lld undefined symbol while check fmt and quality stay green
version: 0.1.0
author: skillworks
---

# Link env triage

Seven checks to stop innocent code churn. A green check plus a named link symbol means toolchain env, never a code bug. Say so in the first reply.

1. **Fingerprint first.** Ask: is `cargo check -p <crate>` green while link names one symbol? Flag as toolchain fingerprint, not a code bug.
2. **Hold owned code.** Ask: did anyone revert, replan, or abandon owned code on a link-red tree? Never touch owned code for env red.
3. **One bug packet.** Ask: is exactly one bug packet filed with the fingerprint line? Code packets wait until env clears.
4. **Quiet re-verify.** Ask: was re-verify run off-peak or after `cargo clean -p <crate>` when the tree is quiet? Clean needs owner or coder seat only.
5. **Two fails stop.** Ask: did the same link FAIL twice? End the step per loop rules. The third try is env-fix-first, never a third code try.
6. **Heavy runner is env.** Ask: did the run exit with code 124? Count it as env under the same rule, never as code fail.
7. **Box clock record.** Ask: does each timing bench record box versus reference clock? Re-pin or quarantine on drift, never gate a slice on another box threshold.

## Test note
Green check plus named symbol proves toolchain env, not code.
File one bug packet with the fingerprint line and hold code packets.
Re-verify off-peak or after clean when the tree is quiet.
