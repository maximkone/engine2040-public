Fight as a gladiator in the arena duel, reading each tell and answering every attack to take best-of-5 rounds
Does this bout read clearly and decide fairly, yes or no?
- Experience: readable decisive duels where every swing commits and every hit lands on one tick, see `crates/combat/src/fight.rs` and `cargo test -p engine-combat --lib input_buffer`
- Theme: Rome at war in a sunlit Colosseum with sand stands gate tunnel and torches, see `docs/plan/rome-spec.md` and `content/arena/placement.json`
- Perspective: third-person arena duel under the hero-cam preset eye 170 cm fov 55, see `docs/plan/rome-spec.md` and `crates/game/src/camera.rs`
- Challenge: tiered opponents T1 recruit to T4 primus that punish spam and reward answers, see `content/design/enemies.rows` and `cargo test -p engine-playtest --lib tiers`
- Decisions: close distance attack defend or dodge with block parry spacing as answers, see `crates/combat/src/exchange.rs` and `crates/combat/src/sweep.rs`
- Mastery: ladder rungs with at most 3 new elements per rung into glory rank, see `crates/game/src/ladder.rs` and `node --test tools/beat-lint.test.mjs`
- Context: the arena duel first then district siege field and ludus hub streamed in cells, see `docs/FOCUS.md` and `docs/VISUAL-TARGET.md`
- Emotion: spectacle serves information in crowd camera blood and light never hiding a tell, see `docs/VISUAL-TARGET.md` and `crates/game/src/windowed.rs`
Slice: vertical, one scene proves all four, the arena duel, so the duel finishes to bar (look, feel, opponents, HUD, round loop per `docs/plan/gameplay.md` section 0) before district siege or ludus get design rows.
