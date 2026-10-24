# The kit's dulled edge reaches a duel blow (2026-10-24)

Board row: `docs/SPRINT-35-book-gaps-4.md` BG15 (book gap, `Damage from
Weapons and Claws` / `[handsrust]`: steel comes to the field already used).
Code: `49f3615e4`.

`crates/arena/src/authority.rs` built every landed blade with
`wear_q16: 0`, so the scaling `crates/combat/src/cut.rs:317` already applies
(`energy * sharpness * (1 - wear)`) was dead: a pugio and a clava cut the
same. The kit now carries its edge condition into the blow.

## What moved

| fixture | before | after |
|---|---|---|
| arena match, `Fixture::STANDARD` (rules v2, kit walls), live path | `2b30084d1530cc78` | `2b30084d1530cc78` |
| the same match through the spine step (feature `spine`) | `2b30084d1530cc78` | `2b30084d1530cc78` |
| v1 golden, `GOLDEN_600` | `d57b5c8ae3096614` | `d57b5c8ae3096614` |

Nothing moved, and that is the point: the pinned fixtures arm no weapons, so
`heft::fold` returns the hash untouched and the bare-hand blades keep
`wear_q16: 0`. An **armed** match does move: tag `0xB5` now writes one more
u32 per active slot (the row's `edge_wear_permille`), which is what BG15 is
for - two bouts with different steel must not hash alike.

## The mechanism

- `content/design/weapons.rows` gained `edge_wear_permille` (`0..=1000`):
  pugio 80, gladius 120, spatha 90, pilum 150, clava 0 (a blunt weapon has no
  edge to dull). The key is optional, so a mod table written before BG15
  parses as a fresh edge.
- `crates/combat/src/heft.rs`: `Heft::edge_wear_permille` and
  `Heft::edge_wear_q16()`, the permille of the resolver's own
  `MAX_WEAR_Q16` (65,536). No float, no new cast.
- `crates/arena/src/authority.rs`: `resolve_cuts` and `resolve_cuts_bare` take
  `&FightState`, and the attacker's armed row sets the blade's `wear_q16`.
  Legacy step (`armour.rs`) and spine step (`spine_step.rs`, now through a
  named `extract_cuts` step) pass the same fight state, so both paths fold
  one number.
- `crates/arena/src/heft.rs`: `Sim::edge_wear_q16(slot)` and
  `Sim::mean_edge_wear_q16()` read the armed rows for the game and the receipt.
- `docs/design/FORMAT.md` documents the new key.

## The measurement

- The row's own Accept, `cargo test -p engine-arena --lib bg15` PASS, 4
  passed / 0 failed (run 40078011659). Readouts:
  - `arena wear cuts: fresh=49 quarter=25 half=0 over 8 swings on the plated
    kit` - the same geometry, one mid blade, only the wear changed.
  - `arena kit duel: rounds=5 kit=gladius mean_edge_wear_q16=7864 hits=5
    cuts=5` - five headless rounds, both slots armed.
  - Every cut the five rounds land carries `energy_q16 = 3,691,008`: the kit
    gladius's dulled number, never the fresh-edge `4,194,304` (the test
    compares both).
  - Proportionality is exact, not sampled: at half wear the eight swings carry
    half the delivered energy (`half_damage * 2 == fresh_damage`) and a fully
    dulled edge severs nothing and carries nothing.
- The rows: `cargo test -p engine-combat --lib edge_wear` PASS, 1 passed
  (run 40076698681) - every shipped row parses inside `0..=1000`, the
  conversion is the permille of `MAX_WEAR_Q16`, a row without the key reads 0
  and `edge_wear_permille=1001` is refused.
- Both digest pins on the BG15 tree:
  `cargo test -p engine-proof --features arena --lib digest_is_pinned` PASS
  (run 40077515405, prints `arena digest=2b30084d1530cc78 ticks=1200 hits=52
  blocks=3 parries=1 dodges=0 knockdowns=4 breaks=11`) and
  `cargo test -p engine-proof --features arena,spine --lib
  spine_arena_digest_is_pinned` PASS (run 40078134691, after the
  `extract_cuts` refactor).
- The v1 golden: `cargo test -p engine2040 --release --test proof_cli
  default_and_max_seed` PASS (run 40077271446), so `d57b5c8ae3096614` holds.
  Rules v1 never arms a row and never reaches `resolve_cuts`.
- The suite around it: `cargo test -p engine-arena --lib` on the same tree is
  322 passed / 3 failed / 7 ignored (run 40076462097). The three reds are not
  BG15's: see the limits below.
- `node tools/rust-quality.mjs --crate engine-combat --crate engine-arena` ->
  RESULT PASS, score 100 (A), 0 ratchet regressions, casts unchanged
  (combat 23, arena 36). `rustfmt --edition 2021 --check` clean on all five
  edited files.

## Limits a reader must know

- `cargo clippy -p engine-combat -p engine-arena --all-targets -- -D warnings`
  is red on master before BG15 and stays red: three
  `assertions_on_constants` errors at `crates/combat/src/design.rs:238,239`
  and `crates/arena/src/lib.rs:3201`, none in a BG15 file. Likewise 1 combat
  test (`heft::tests::the_gladius_row_is_the_shipped_table`) and 3 arena tests
  (`table_version_config_selects_the_rules_it_builds`,
  `live_cut_spawns_a_limb_and_repeats`,
  `heft::tests::a_gladius_bout_steps_as_the_table_and_only_the_hash_tag_differs`).
  Cause: the readable tell `cbced6a3d` (OPT59) raised the v2 light startup
  15 -> 30 ticks and did not rescale `weapons.rows` or those four pins, so the
  gladius row (15) no longer equals the table (30). Row: `docs/BLOCKERS.md`.
  The fix is a design call on the weapons table and belongs to that row, not
  to BG15, so nothing here edits it.
- `crates/arena/src/limbs.rs:366`, the row's second named site, keeps
  `wear_q16: 0`: it is the test `limbs_come_from_the_real_resolver_output`
  reading a blade it builds itself, not a producer of duel blows, and the
  row's P2P says the cut tests stay unchanged.
- Wear is the kit's **arrival** condition, per row. It does not grow during a
  bout: nothing counts strikes per slot, and `Limb` records only the victim,
  so a per-attacker wear state would need a new `Sim` field, a snapshot shape
  change and a fork audit. That is a separate row.
- The pin proves identity, not quality: `2b30084d1530cc78` says the pinned
  unarmed fixture still folds one number. The behaviour is carried by the four
  BG15 tests, not by the pin.

## What this receipt does not claim

No pixel was looked at: wear changes what a cut carries, not how it is drawn.
No armed-duel digest is pinned in-tree, so an armed match's new checkpoint
bytes are proven only by the fold's own test and by the duel's cut energies.
