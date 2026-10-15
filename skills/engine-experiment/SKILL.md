---
name: engine-experiment
description: Use when running an engine2040 EXPERIMENT spike - smallest reproduction plus one ablation, like-for-like numbers, negative-result preservation, adopt/reject verdict. Loaded by the builder for EXPERIMENT packets.
---

# Engine experiment (from builder)

One EXPERIMENT packet: paper/version, accepted hypothesis, baseline, fixture/seed, metric, resource budget, stop rule, real engine consumer, owned files. No new framework, golden changes, paid service or focus changes.

- **Spike:** smallest runnable reproduction on the named fixture plus one discriminating ablation. Same build, same seed, baseline vs candidate on the one deciding metric.
- **Like-for-like:** compare identical inputs/builds; state non-transferable assumptions and code/data/weight licenses. No cherry-picking, no unsupported speed claims.
- **Three numbers stay distinct:** paper claim vs reproduced effect vs integrated gain. Preserve negative results; they prevent repeated work.
- **Return:** patch, commands, baseline/candidate outputs, uncertainty, consumer, and adopt/reject/insufficient-evidence with the LADDER line: `SPIKE | P<n> | <idea> | <source> | <fixture, baseline -> candidate, numbers> | ROW proposal or REJECTED because <numbers>`.
