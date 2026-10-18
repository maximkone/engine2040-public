STEAL CARD: OPENXPLANE (wave 2026-10-18, block 7, position 7). Read-only pass; donor files read live
2026-10-18, paths taken from the pinned tree. No bytes vendored.
Repo: https://github.com/OpenXPlane/openXplane @ 39dd2af67b12 (main, updated 2026-10-18T16:12Z, 1 star).
Licence: GPL-3.0 (LICENSE:1 "GNU GENERAL PUBLIC LICENSE", + GitHub licenseInfo gpl-3.0, read 2026-10-18).
Copyleft: steal the method, write our own code; no GPL bytes into the tree.
What it is: an independent X-Plane 12-compatible flight engine in Rust + wgpu that needs the user's own copy of
X-Plane 12. The project is a port with a proof: each ported function is checked bit-for-bit against the retail
binary by running the original machine code in a CPU emulator (Unicorn) on synthetic inputs. 24 crates, ~180
vector generators in tools/, and one 4,600-line `tests/original_vectors.rs`.

=== 1. WE HAVE ===
- Golden hash `d57b5c8ae3096614` and a hash law (docs/HASH-LAW.md); per-system hashes with the first diverging
  system named (crates/arena/src/sync.rs:29-46, :65-85, :168-186).
- Our own vector tools: tools/rollback-vectors.mjs, tools/rng-vectors.mjs, tools/tape-checksum.mjs, tools/skin-vectors.mjs.
- A cooked-table contract: crates/assets/src/retarget.rs:1178-1184 (`RetargetedClip { root: RootTable, contacts: ContactTable }`).
- A mutation gate over a patch: tools/mutants.mjs (cargo-mutants in-diff, `--max-missed`, RESULT PASS|FAIL).

=== 2. STEALS (ranked by value for proof discipline per hour) ===
1. Oracle-run bit-exact vectors, seed and case count inside the test | tools/emulate_xp.py:1-14, tools/gen_atmosphere_vectors.py:1-8
   + :27-40, crates/xp-airfoil/tests/delta_blend_exact.rs:3-33, crates/xp-app/tests/original_vectors.rs:1639-1673,
   :1838-1848 (read 2026-10-18) | GPL-3.0 | SIM/TOOLING (proof only, never in the tick) | lands crates/proof +
   a `tools/*-vectors.mjs` of ours | M
   Idea (method, not the emulator): the vector file is produced by running the ORACLE (the reference binary/function),
   not by running our code; the generator prints f32 words as hex (`hx()` = `to_bits`), the test compares bits and
   ends by asserting the case count (delta_blend_exact.rs:33 `assert_eq!(count, 2228)`), so a shrunken fixture fails.
   The harness even splits the two claims: exact bits, and `words_close` for the last bits of libm sin/cos/atan2
   (original_vectors.rs:1838-1848). Every generator carries its seed (gen_atmosphere_vectors.py:28 `Random(20261018)`).
   Maps: our golden hash is produced by our own build; nothing forces an independent oracle. For the parts of the sim
   that must match a recorded tape (rollback, RNG, skin weights) we already ship vectors - add the two missing columns:
   the seed of the generator and the case count asserted in the test.
   Moves: `-vectors.mjs` gains `--record` (write the fixture from the oracle: the committed tape / a frozen build) and
   `--check` (compare bits, print `first bad case N`), and the test asserts the count. Proof: `node tools/rollback-vectors.mjs
   --check` reports the same count twice, and dropping one line from the fixture turns it red.
   Kill: kill a fixture whose generator cannot run offline from a pinned source (a hand-edited table is a snapshot, not
   an oracle, and it stays green forever). Also kill any bit-compare of a libm-trig value - use the libm-tolerance lane.

2. A data-driven record schema with a version tag, per-field hash and index arity | crates/xp-acf/data/object_schema.tsv:1-30,
   crates/xp-acf/src/object.rs, tools/gen_acf_object_vectors.py (read 2026-10-18) | GPL-3.0 | CONTENT/ASSETS |
   lands crates/assets (a cockpit-style schema beside retarget.rs) | S-M
   Idea: every record field is one TSV row - path with its index slots (`_blad/{}/_ailn1/{}`), type, a data-version tag
   (R6010), the hex offset, index arity (`0,4`), a transform (`raw`, `scale:3e9c0ebf`, `int`, `string`) and a per-field
   hash. The generator writes the table from the schema, so an offset move is a one-line diff that names the field.
   Maps: our cooked tables are structs (retarget.rs:1178-1184). A hash hides WHICH field changed; a per-field row plus a
   per-field hash names it, which is exactly what "cook twice equal" needs to be actionable.
   Moves: emit a `schema.tsv` from the cook (path, type, arity, transform, hash) and compare it in the existing cook test;
   a changed row is the receipt. Proof: cook twice, `schema.tsv` byte-equal; flip one constant, exactly one row changes.
   Kill: kill it if the cooked data is not a flat record - for nested data the row hides the parent and the schema becomes
   a second, drifting source of truth. Keep those fields in code.

3. Exact float semantics pinned per helper, with the NaN and tie branches named | crates/xp-airfoil/src/scalar.rs:1-3, :5-14,
   :21-25, :27-35, :37-41, :82-87 + crates/xp-app/tests/original_vectors.rs:1639-1673 (read 2026-10-18) | GPL-3.0 |
   SIM (ordered f32, hashed) | lands engine2040 scalar helpers used by the fixed tick | S
   Idea: each helper documents the machine's exact behaviour, not a rounded intention: `clamp` tests the low bound first
   (scalar.rs:5-14), `max3` lets the second operand win ties AND NaN (":21-25"), `snap` sends the midpoint to `hi`
   (":27-35"), `within` lets NaN pass (":82-87"). Each names its oracle address in the doc comment and has a case file.
   Maps: docs/HASH-LAW.md already fixes which float ops are in the hash, but our small helpers (`clamp`, `snap`, `lerp`)
   do not state their NaN/tie rule. Two builds agree only if the rule is the same on both.
   Moves: for every hash-feeding helper, write the comparison order and the NaN outcome in the doc and add the NaN and
   exact-midpoint cases to its test. Proof: a test with NaN and midpoint inputs passes on both a debug and a release build
   and the golden is unchanged.
   Kill: kill for render-only or view-only math (reordering it cannot move the hash) - pinning those branches is dead weight.

4. One mutant, one named test, and "MISSING" as a first-class outcome | tools/mutcheck.py:1-20 (read 2026-10-18) | GPL-3.0 |
   TOOLING (research/proof harness) | lands a one-line form used in research cards + tools/mutants.mjs docs | S
   Idea: the reader asserts a fact ("the delta blend uses the clamped index"), then runs one named test against one
   textual mutant; the tool prints `killed`, `SURVIVED` or `MISSING` and restores the file in `finally` (mutcheck.py:8-19).
   It is the per-claim form, run while reading, not a whole-crate campaign.
   Maps: tools/mutants.mjs covers a patch (in-diff, score, PASS/FAIL). We have no cheap per-claim form, so a research card's
   load-bearing fact ("cut authority is integer") can rest on a test that does not actually guard it.
   Moves: a research/proof card may carry a `mutant:` line - the exact replacement plus the test that must fail - so the
   next reader can re-run one command. Proof: the named test fails with the mutant applied and passes after restore.
   Kill: kill the per-claim form if the mutant is not re-run with the code (a rename turns it into `MISSING`, which reads
   like a pass). Keep whole-crate truth in tools/mutants.mjs.

=== 3. NOT WORTH IT ===
- The X-Plane UI port (crates/xp-app/src/ui_*.rs, ~35 modules plus research/UI_*.md): a faithful port of one retail
  simulator's menus. No consumer in engine2040.
- The flight model itself (xp-airfoil: airfoils, ISA atmosphere, stall, wing elements): strong work, but our sim has no
  flight and no aerofoil; it would be a second physics model beside Rapier.
- The Unicorn-based emulator as infrastructure: it needs a retail PE and `pip install unicorn`; our oracles are our own
  tapes and builds. Take the discipline (oracle, seed, bit-compare, case count), not the emulator.
- The GPL tree as a donor for copy-paste: ideas only.

DONE 4 steals, 9 donor files read.
