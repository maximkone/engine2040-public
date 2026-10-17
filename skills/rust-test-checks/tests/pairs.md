# Skill-on vs skill-off pairs: rust-test-checks

Three situations. Off is what an agent says from memory. On is what the skill makes it say. Pass means the On answer cites the rule, names the crate and run line, and says what proves it.

## P1: bare panic test hides the wrong branch

Input: Review a range guard in crates/combat that must reject values over 100. The test is marked should_panic with no expected text. Later the two panic branches get swapped and the test still passes.

Answer with the skill: Rule 5 fails (crates: combat). Pin the over-100 branch with an expected substring, then run cargo test -p engine-combat greater_than_100. A bare should_panic passes on any panic, even the wrong guard.

What a skill-less answer misses: that green does not mean the right guard fired. Without the substring the swapped message stays hidden.

## P2: integration test reaches into private code

Input: Review crates/combat where a tests/it module calls a private helper directly, and someone adds crates/combat/tests/extra.rs as a second test executable that runs with zero tests.

Answer with the skill: Rule 9 fails (crates: combat, game). Keep integration tests on the public API as modules of crates/combat/tests/it/ (one `mod extra;` line in main.rs), so the crate links one test binary, and keep a thin binary over a lib so tests can use it. Run cargo test -p engine-combat --test it extra::.

What a skill-less answer misses: that a stray file under tests/ is its own crate. A direct private call will not match how users call the lib, a top-level file fails the workspace-check test-layout guard once the crate merged to tests/it, and a helper file adds an empty test section that hides real coverage.

## P3: parallel tests share one output file

Input: Review crates/perf where two tests write to the same test-output file on disk and read it back. The suite passes alone but flakes in the full run.

Answer with the skill: Rule 7 fails (crates: perf, physics). Tests run parallel by default, so give each test its own file or pin the run to one thread for diagnosis. Run cargo test -p engine-perf -- --test-threads=1 to confirm the interference, then split the files.

What a skill-less answer misses: that parallelism is the default. A pass in isolation does not prove the code, only that the shared file was not raced that time.
