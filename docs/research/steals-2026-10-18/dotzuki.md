STEAL SCOUT: DETERMINISTIC EVENT ORDER, SCRIPTED RNG, AND DATA-AUTHORED RULES (liuyanghejerry/dotzuki)   scout date 2026-10-18, read-only run
Repo: https://github.com/liuyanghejerry/dotzuki  sha e5028624e789c06ddf867096d74126d0a01e5bb2 (2026-10-18T18:55:41+08:00, "Merge pull request #103 from liuyanghejerry/fix/perfect-pitch-base-1155")
Files read live 2026-10-18 by shallow clone; no bytes vendored. Sources sit under workspace/crates/.
LICENCE: MIT OR Apache-2.0 (both LICENSE-MIT and LICENSE-APACHE are present in the tree; the GitHub API's single-choice answer reads Apache-2.0). Pick either arm; Apache-2.0 is
  the safe read.
What it is: a from-scratch JRPG engine (Pokemon-Red-lineage, Game Boy look) with a real engine/game split: dotzuki-engine is game-agnostic and never links `rand`, dotzuki-rules loads
  a declarative rules.ron into a closed primitive vocabulary, dotzuki-engine-dsl compiles a small script language, and dotzuki-audio is a GB APU + music/SFX sequencer. The parts we
  want are all about *order*: a total, tie-broken comparator for simultaneous effects, randomness that only enters through a trait the game owns, and mechanics written as data.

== 1. WE HAVE ==
- Combat is integer and table-driven already: crates/combat/{hitsim.rs,fight.rs,tables.rs,exchange.rs}; a fight resolves hit windows and parry timing on the fixed tick.
- Randomness is counter-based and forked (crates/core/src/counter_rng.rs:32-145) and reaches the sim only through submitted actions; nothing hand-rolls a float RNG.
- Rules live as data (crates/rules/{rules.rs,host.rs,sim.rs,script.rs}) with fixtures and a tape.
- Audio has explicit voice budgets and a director (crates/audio/{voice.rs,director.rs,mix.rs}); ducking around cues exists in the mixer, not as a documented channel rule.

== 2. STEALS (ranked by value per hour) ==
1. A total, fully tie-broken comparator for simultaneous events, with a monotonic creation counter as the last key | workspace/crates/dotzuki-engine/src/battle/stack/dispatch.rs:12-66 | MIT OR Apache-2.0 | SIM (order) | lands crates/combat (exchange/fight ordering) and crates/ecs/src/schedule.rs | S
   Pattern: every collected handler carries `order` (LOW first), `priority` (HIGH first), `speed` (HIGH first), `sub_order` (LOW first) and `effect_order` (a monotonic creation
   counter, LOW first), and `compare` is exactly `order -> priority -> speed -> sub_order -> effect_order` (:51-66). The last key is a *creation* counter, so no pair is ever a tie
   by construction, and the file states why: it is "the final deterministic tiebreak" (:23-24). Distinct from our risk case, this is what replaces `sort_unstable` on tied keys.
   Maps onto: any two fighters, cuts or gore events landing on the same tick need one order that is the same on every peer. Our exchange path must not depend on sort order of tied
   keys (a live determinism risk this repo closes with one counter). The counter must come from the tick, not from insertion into a HashMap.
   Moves: simultaneous events carry a dense sequence number from the tick; F2P a test that two handlers with equal order/priority/speed resolve in submission order on every run.
   Proof: `cargo test -p engine-combat --lib order` new case (five-tier compare, plus 1,000 permutations give one order) x2; golden d57b5c8ae3096614 unchanged.
   Risk: a wider key costs bytes per event. Kill: if the sequence number cannot come from the tick state (a hidden global or an atomic counter breaks replay).
2. Randomness behind a trait the game owns, plus a byte-script RNG that also counts draws | workspace/crates/dotzuki-engine/src/battle/rng.rs:1-109 | MIT OR Apache-2.0 | SIM (test) | lands crates/proof (beside tape.rs/chaos.rs) | S
   Pattern: the engine "must never link the `rand` crate"; all randomness flows through a `BattleRng` trait with one required method `next_u8` and defaults for `range`/`chance`
   (:1-6, 27-68), so the *game* owns the exact draw sequence and can reproduce quirks (:8-11). `ScriptedRng` replays a fixed byte list then repeats the last byte, and exposes
   `consumed()` "useful for asserting draw-order parity" (:76-108).
   Maps onto: our proof crate needs a tape that pins *how many* draws a fight consumed, not only the outcome. A scripted source plus a draw count turns "this fight is
   deterministic" into "this fight drew the stream in this exact order", which catches an added or removed draw that leaves the final hash equal by luck.
   Moves: a fight test asserts the draw count and the exact bytes; F2P an extra `next_range` call moves `consumed()` and fails the test while the winner is unchanged.
   Proof: `cargo test -p engine-proof --lib scripted_rng` x2; golden unchanged.
   Risk: a script that runs out repeats the last byte, which can hide a missing draw; assert `consumed()` equals an expected constant. Kill: if the scripted source is reachable
   from production code rather than tests only.
3. Mechanics as data: a RON file compiled into a closed primitive op set, not new code paths | workspace/crates/dotzuki-rules/src/lib.rs:1-20 (+ interp.rs, registry.rs, model.rs) | MIT OR Apache-2.0 | SIM (content) | lands crates/rules (rules.rs/host.rs plus a data loader) | M
   Pattern: a game-side loader turns a declarative rules.ron into runtime `Effect`s, dispatched through ONE zero-capture interpreter bridge plus a *closed* primitive-op
   interpreter; the crate depends on the game-agnostic engine only, has no `rand`, and "amortizes content" (one `InflictStatus` covers every secondary-status move) without
   extending mechanics (:1-15).
   Maps onto: our rules crate already carries mechanics as data with a host and a tape; the additional discipline worth copying is the explicit *closed* op vocabulary and the
   refusal to let content add a mechanic. That keeps the tick surface fixed while move data grows.
   Moves: a new move is a data row only; F2P a rules.ron naming an op outside the closed set is refused with the op name, not silently ignored.
   Proof: `cargo test -p engine-rules` new refusal case x2; golden unchanged.
   Risk: the closed set becomes the bottleneck and authors want a new op. Kill: if a content-only change ever needs a code diff to take effect, the closure has already leaked.
4. Fixed music/SFX channel budget with SFX overriding music and a resume state | workspace/crates/dotzuki-audio/src/sequencer.rs:1-10 (+ manager.rs) | MIT OR Apache-2.0 | PRESENTATION (audio) | lands crates/audio/src/{voice.rs,director.rs,mix.rs} | S
   Pattern: a fixed set of logical channels (4 music + 4 SFX) where an SFX channel *overrides the music channel on the same hardware output*, each channel keeping its own command
   pointer, note delay, volume and pitch-slide state, and the manager tracks master volume, music fade-out and cross-track resume states (lib.rs:11-15, sequencer.rs:1-10).
   Maps onto: our combat cues, crowd and music already meet in the mixer with a voice budget; the donor's rule is smaller and testable: a bounded channel table, an explicit
   override priority, and a resume state so a cue ducking music does not lose the music's place.
   Moves: at most K sounding voices by construction, a cue never silences music permanently, and ducking restores the exact prior music state.
   Proof: `cargo test -p engine-audio --lib voice_override` x2; a golden audio trace byte-equal.
   Risk: an override can mask a music line mid-phrase. Kill: if any cue leaves a channel unresumed, or if the audio trace gate moves.

== 3. NOT WORTH IT ==
- dotzuki-renderer's indexed/packed-2bpp framebuffer (workspace/crates/dotzuki-renderer/src/indexed_framebuffer.rs:1-25) and the GB tile pipeline: a 2D palette renderer for a
  160x144 screen. Our render path is 3D wgpu; the storage trick does not transfer. (The *idea* of a CPU rasterizer as a GPU-free pixel oracle is worth its own card, not this one.)
- dotzuki-engine-dsl (parser.rs 100 KB, compiler.rs, interpreter.rs, codegen): a whole script language with JS/JSON codegen. Our rules crate already loads data; a second language
  is more surface than the duel needs.
- The Showdown effect-stack arena itself (stack/mod.rs 74 KB, ctx.rs, driver.rs): a full status/ability engine for Pokemon mechanics, not our melee exchange. Read only for the
  comparator and the closed event taxonomy.

DONE 4 steals, 8 donor files read (battle/stack/dispatch.rs, battle/rng.rs, dotzuki-rules/lib.rs, audio/sequencer.rs + lib.rs, indexed_framebuffer.rs, battle/stack/mod.rs, LICENSE pair).
