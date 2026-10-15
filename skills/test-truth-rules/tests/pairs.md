# Skill-on vs skill-off pairs: test-truth-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the file, capture or test that proves it.

## P1: frame gate counts files (testing)

Situation: A builder writes a frame gate that counts the PNG files in the output folder and reports the gate green when 300 files exist.

Off: "The gate is green: 300 frames were written."

On: "Rule 1: a gate that trusts the producer's counts is fake. Check the pixels. Search `node tools/books.mjs search "quality criteria test"` and cite [testing ch.9]. Open two frames and check that a fighter is in them. The gate fails if the frames are blank."

What skill-less misses: that a file count does not prove a fighter is on screen. The first-picture walk in the testing review scored 0 of 10 for exactly this reason.

## P2: GPU test that returns early (testing)

Situation: A builder adds a GPU test that returns early when no adapter is found. The CI machines have no GPU, so the test passes on every run.

Off: "If there is no GPU, the test returns early and passes. That is fine for CI."

On: "Rule 2: a test that returns early passes by skipping. Mark it SKIPPED, count it, and fail it when the reference-GPU leg requires it. Search `node tools/books.mjs search "most important thing to test"` and cite [testing ch.1]. Print the SKIPPED count in the gate output."

What skill-less misses: that a skip counts as a pass. The GPU path never runs in CI, and no one sees the count.

## P3: wall-clock assert in the default suite (testing)

Situation: A builder adds `assert!(frame_ms < 16.0)` to a normal unit test. The test runs in the default suite on shared CI machines.

Off: "The assert guards performance. Keep it in the default suite so it always runs."

On: "Rule 3: never assert wall-clock time in the default suite. Move the assert to `#[ignore = "timing: ..."]` and run it under `--perf`. Search `node tools/books.mjs search "browser tests timing flaky"` and cite [rustwasm ch.9]. Keep the number of timing tests small."

What skill-less misses: the flaky red build on a busy machine. The timing assert fails at random, and the team learns to ignore red builds.
