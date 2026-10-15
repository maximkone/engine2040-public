# Skill-on vs skill-off pairs: wound-map-checks

Three review situations. "Off" is what a reviewer says from general habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the evidence it asks for.

## P1: uncapped drain out of order (items 5, 7)

Situation: a diff queues 10 wound paints with `paints_per_frame` 4 in the new `wound_map.rs` but `drain` returns all 10 at once by walking a hash map in random order.

Off: "It paints every wound in one frame, faster is better. LGTM."

On: "Reject. Item 5: `drain` must return at most `paints_per_frame` in `(tick, char_id)` order like `DecalPool::place` order in `crates/render/src/decals.rs`; uncapped drain or hash-order walk is the flag. Item 7: the same queue must give the same drain bytes twice like `DecalPool::state_bytes`. Evidence needed: 4, 4, 2 drains in order plus x2 equal bytes."

## P2: one combined map with cracks (items 1, 4)

Situation: a diff keeps one shared wound texture for all fighters in `wound_map.rs` with no `DILATE_PX` seam fill. Cuts smear across bodies and UV islands show cracks.

Off: "One texture saves memory and seams are an art issue. Ship it."

On: "Rework. Item 1: keep one map per `char_id` like `DecalAtlas` tiles in `crates/render/src/decals.rs`; a combined map that smears cuts is the flag. Item 4: fill a `DILATE_PX` ring so seams share texels, mirroring closed joins in `decals.rs`. Evidence needed: per-fighter maps plus on-vs-off captures with no seam cracks."

## P3: silent eviction that leaks into the tick (items 6, 8, 9)

Situation: a diff exceeds `cap_bytes` in `wound_map.rs`, drops the newest paint with no report, tracks no `used_bytes`, and folds wound bytes into the tick hash.

Off: "Eviction is an edge case and the hash only moved a little. Accept."

On: "Reject. Item 6: over `cap_bytes` evict the oldest character map and report it like `DecalPool::evict_oldest` in `crates/render/src/decals.rs`; newest-dropped or silent loss is the flag. Item 8: track `used_bytes` like `DecalAtlas::used_bytes`. Item 9: wound bytes stay out of `state_hash` per `docs/HASH-LAW.md`. Evidence needed: oldest-evicted report, byte totals, plus golden hash unchanged x2."
