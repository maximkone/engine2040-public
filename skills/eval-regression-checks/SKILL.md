---
name: eval-regression-checks
description: Use when a paired regression suite plus frozen baseline replay ships and complements ai-door-rules, never fires alone
version: 0.1.0
author: skillworks
---

# Eval regression checks

Use when an AI-door eval or win-rate claim ships. This complements ai-door-rules and never replaces its 8 rules. No claim lands without a frozen baseline plus paired runs and a green regression suite.
Complements ai-door-rules, never fires alone.

1. **Frozen baseline pin.** Pin model id plus prompt text plus cassette version plus seed 2040 before the first run. Rerun against the same pin, never a moving latest. Flag a baseline named latest or a prompt edited mid-run.
2. **Paired comparable runs.** Run baseline and change on the same map plus same seeds plus same tick count plus same build. Keep N equal on both sides. Flag different seeds or tick counts or builds per side.
3. **Regression suite green.** Run the full prompt regression suite before the claim and keep every case green. A new behavior adds a case first and shows it FAIL then PASS. Flag a skipped suite or a case deleted to make it pass.
4. **No win-rate without proof.** Apply the ai-door-rules bar and report wins plus N plus sides swapped plus binomial p, about 1000 games for a close call. Small N proves nothing. Flag a rate with no N or no p or one-sided maps only.
5. **Offline replay.** Replay every eval from recorded cassettes without the live model. The same inputs give the same outputs and the same verdict twice. Flag a number that needs the live model to reproduce.
6. **Triage the delta.** List changed cases by name, keep unrelated greens untouched, and file one bug row per real regression with seed and repro. Flag a mean with no per-case table.

## Test note
- Add one regression case for each new AI behavior and show it FAIL before the fix.
- Rerun baseline plus change paired on seed 2040 and paste both hashes.
- Keep this skill green with `node tools/workspace-check.mjs` after any edit.
