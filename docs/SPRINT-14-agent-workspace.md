### Agent workspace (owner 2026-09-26, `docs/plan/seats.md`)

The OpenCode loop itself: smaller contexts, shell-safe research,
cross-family review, halt lifecycle, destructive-command and secret
guards, tests for every loop tool. Rows are orchestrator-landed unless
they name another lane; none touches `crates/`.

- [x] AW9 tools: secret guard — `packet-gate` and the commit path scan Progress 2026-10-17: `tools/secret-gate.mjs` scans the staged diff (12 tests in secret-gate.test.mjs pass; `--self-test` detects a seeded key). No hook calls it (docs/book/ref-process.md:230); packet-gate and the plaintext-key-file handoff flag are gone. Left: wire it into the commit path and run the 50-commit false-positive check. Landed 0324e09c. node --test tools/secret-gate.test.mjs 12 of 12 pass; --self-test PASS; a seeded key in a scratch index is refused by the installed pre-commit hook (exit 1), a clean index passes (exit 0); the 50-commit check scanned 91 file diffs with 0 violations; the tool and its test are tracked now (were under .git/info/exclude), and tools/hooks/pre-commit carries the scan.
  staged diffs for key and token patterns and refuse; the handoff flags
  plaintext key files in the OpenCode config folder for the owner. Tier
  A. Size S. Owns the scanner and its test. Accept: a seeded key in a
  diff is refused; no false positive on the last 50 commits. needs: none.
