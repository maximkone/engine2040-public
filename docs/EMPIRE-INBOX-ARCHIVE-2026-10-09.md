# Empire inbox archive 2026-10-09 — engine2040

16 rows moved from docs/EMPIRE-INBOX.md ## Open on 2026-10-09: 9 ticked-done rows plus 7 done-via-other/superseded/stale rows. Texts kept whole and verbatim below. Reopen by moving a line back under ## Open in docs/EMPIRE-INBOX.md.

## Rows (verbatim)

- [ ] EB-2026-10-08-S242 from center (lead2, 2026-10-08) [LEAD2-SPEED-heavycap2] | target/queue/batch.md: raise CPU-heavy cap from 3 to 6 while CPU stays under 80pct | why: J5 2026-10-08: builders 18pct of packets (bar 40pct), 19 heavy packets wait for 3 places, oldest ready 240min (bar 30min); S13/S28 closed but cap still reads 3 | done when done when: rg 'CPU-heavy at most 6' target/queue/batch.md prints 1

- [ ] EB-2026-10-08-S248 from center (center /sprint, 2026-10-08) | List playtest/src/spatial_shapes.rs in GE2 ledger or remove assumption | why: drift+workspace FAIL on same file | done when done when node empire.mjs check engine2040 drift workspace PASS

- [x] EB-2026-10-08-S249 [size 12848B 2026-10-09] from center (center /sprint, 2026-10-08) | Trim docs/EMPIRE-INBOX.md 126.1KB under 120KB cap to archive | why: size FAIL over cap | done when done when node empire.mjs check engine2040 size PASS

- [x] EB-2026-10-08-S258 [row E54 2026-10-09] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-ROOT-DEPS] | Remove 9 unused root deps from Cargo.toml (engine-core/ecs/render/physics/nav + 4, only engine-proof used) | why: root manifest carries 9 crates src/ never imports; smaller graph, faster metadata | done when done when: rg -n engine- src/ shows only engine-proof and cargo check -p engine2040 green

- [x] EB-2026-10-08-S259 [row E55 2026-10-09] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-WINDOWED-SPLIT] | Split windowed_dressing.rs out of crates/game/src/windowed.rs (:173-509, ~337 lines) | why: windowed.rs is 2267 lines; dressing block moves verbatim with identical signatures | done when done when: windowed.rs Measure-Object drops ~300 lines and cargo check -p engine-game green

- [x] EB-2026-10-08-S260 [row E56 2026-10-09] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-ARENA-SPLIT] | Split match_observer.rs out of crates/arena/src/lib.rs (7 fns + TickMatchEvents, ~145 lines) | why: arena lib.rs is 3828 lines; observer trio versioned/v1 shims move with re-export, zero signature change | done when done when: lib.rs drops ~145 lines and cargo check -p engine-arena green

- [x] EB-2026-10-08-S261 [row E49 2026-10-08] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-COMBAT-SPLIT] | Extract resolve_clashes + resolve_parries from resolve_tick (exchange.rs:484-582, 99 lines) | why: top combat fn is 99 lines; two helpers leave 3 fns under 55 lines, no row or event change | done when done when: rg shows resolve_clashes + resolve_parries defs and cargo test -p engine-combat --lib green

- [x] EB-2026-10-08-S262 [row E50 2026-10-08] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-NET-MERGE] | Merge net validate/dup pairs (delta validate x2 + replicate validate, wire lens x3, mask-diff x2) | why: same sorted-unique and mask checks in 3 places; one shared helper each, transports unchanged | done when done when: rg sorted-unique shows 1 def site and cargo test -p engine-net --lib green

- [x] EB-2026-10-08-S263 [row E51 2026-10-08] from center (campaign, 2026-10-08) [AUDIT-CAMPAIGN-SPEED] | Land measured speedup bundle: workspace double-scan (~495ms), rust-quality prefilter (~3300ms), plan-atlas reread | why: three timed hotspots with hunks ready; filter-before-scan and hoisted reads cut two full scans | done when done when: workspace + rust-quality --crate core medians drop 2x and plan-atlas tests 7/7 pass

- [x] EB-2026-10-09-S1 [row E52 2026-10-08] from center (campaign, 2026-10-09) [LEAD2-CAMPAIGN-unwired-schema] | Wire or delete crates/assets/src/schema_compile.rs (workspace FAIL: never compiled) | why: drift-check FAIL: schema_compile.rs is not declared by any module; 10 rg hits all self-file, no mod row in lib.rs | done when done when: node tools/workspace-check.mjs prints no schema_compile line

- [x] EB-2026-10-09-S3 [row E53 2026-10-08] from center (campaign, 2026-10-09) [LEAD2-CAMPAIGN-pool-eta] | Pool-wait ETA plus auto-retry in tools/lanes.mjs (pool-starvation x12 of newest 30 done) | why: wave-4 recount: keeper 2h time-cap mass-cut plus 3-session refuse; newest running all salvage-resume | done when done when: rg pool-starvation class in 24h done records prints 0

- [ ] EB-2026-10-09-S45 from center (campaign, 2026-10-09) [LEAD2-CAMPAIGN-sprint-readme] | sprint/ folder README 5-15 lines (STRUCT folders 5/6, only missing part) | why: repo-standard folders check is the one red part beside protected bigdocs; every other listed folder has one | done when done when: node C:/empire/center/repo-standard.mjs engine2040 prints folders 10/10

- [ ] EB-2026-10-09-S48 from center (campaign, 2026-10-09) [LEAD2-CAMPAIGN-skill-cut] | Cut skills/engine-orchestrate/SKILL.md 8688 bytes under 8KB (move coverage table lines 45-86 to references) | why: heaviest first-read doc agents load; agents/*.md all under 8KB, this one skill is over | done when done when: Measure-Object skills/engine-orchestrate/SKILL.md prints under 8192 bytes

- [ ] EB-2026-10-09-S62 from center (campaign, 2026-10-09) [LEAD2-CAMPAIGN-ORPHANS] | Wire or delete 2 orphan modules endpoint packs/gym-observe.rs plus net snapshot.rs with GE2 ledger line, workspace FAIL | why: drift board-valid red from fresh untracked files | done when done when: node tools/workspace-check.mjs prints PASS

- [ ] EB-2026-10-09-S140 from center (Maxim, 2026-10-09) | Add heavy_max knob value 6 min 1 max 6 to knobs.json at wave boundary then set it | why: set refuses unknown knob, hand-edit banned while loop runs, S13 S28 closed but cap still reads 3: knob never existed, HEAVY_MAX 3 wins | done when knobs.json carries heavy_max 6 with history line, rg 'CPU-heavy at most 6' target/queue/batch.md prints 1, closes S242 S13 S28 for real

- [ ] EB-2026-10-09-S143 from center (Maxim, 2026-10-09) | Keep heavy cap at 3, supersedes S140 and S242: RAM wall | why: box 2.9GB free, one Rust build 3.6GB, 6 parallel builds do not fit, Maxim: make it 3 | done when knobs.json carries no heavy_max, batch.md reads CPU-heavy at most 3, S140 S242 closed as superseded

## Why each row is dead

- EB-2026-10-08-S242: SUPERSEDED by EB-2026-10-09-S143 (Maxim 2026-10-09: keep heavy cap at 3, RAM wall; S143 text closes S242 as superseded).
- EB-2026-10-08-S248: STALE: playtest/src/spatial_shapes.rs deleted (Test-Path false), 0 refs repo-wide outside archive/, workspace-check PASS, drift FAIL is the unrelated rust ratchet.
- EB-2026-10-08-S249: DONE ticked [x] 2026-10-09 (size trim landed 12848B).
- EB-2026-10-08-S258: DONE ticked [x] [row E54 2026-10-09].
- EB-2026-10-08-S259: DONE ticked [x] [row E55 2026-10-09].
- EB-2026-10-08-S260: DONE ticked [x] [row E56 2026-10-09].
- EB-2026-10-08-S261: DONE ticked [x] [row E49 2026-10-08] (resolve_clashes/resolve_parries present in crates/combat/src/exchange.rs).
- EB-2026-10-08-S262: DONE ticked [x] [row E50 2026-10-08].
- EB-2026-10-08-S263: DONE ticked [x] [row E51 2026-10-08].
- EB-2026-10-09-S1: DONE ticked [x] [row E52 2026-10-08]: file deleted, workspace-check PASS (all 587 source files compiled, no schema_compile line).
- EB-2026-10-09-S3: DONE ticked [x] [row E53 2026-10-08].
- EB-2026-10-09-S45: DONE on disk: sprint/README.md exists with 8 lines (done-when wants 5-15).
- EB-2026-10-09-S48: DONE on disk: skills/engine-orchestrate/SKILL.md is 8159 bytes, under the 8192 cap.
- EB-2026-10-09-S62: DONE on disk: gym_observe wired (mod in crates/endpoint/src/packs/gym.rs), snapshot wired (pub mod in crates/net/src/lib.rs), workspace-check PASS.
- EB-2026-10-09-S140: SUPERSEDED by EB-2026-10-09-S143 (Maxim 2026-10-09: keep heavy cap at 3, RAM wall; S143 text closes S140 as superseded).
- EB-2026-10-09-S143: DONE on disk: knobs.json carries no heavy_max knob and target/queue/batch.md reads CPU-heavy at most 3.
