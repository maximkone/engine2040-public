---
name: wound-map-checks
description: Use when a builder adds wound paint maps for fighter cuts and a reviewer checks the queue before it ships.
---

# Wound map checks

Live wounds paint per-fighter maps, never the tick. Nine checks. Each names the file to open. PASS needs capped drain in order plus eviction proof, never a hash move.

1. **One map per fighter.** Ask: does `WoundMaps` keep one map per `char_id` like `DecalAtlas` in `crates/render/src/decals.rs` keeps one tile set per decal? Flag: one combined map that smears cuts across fighters. (render)
2. **Rect clamp.** Ask: does `queue` clamp `uv_rect` to 0..1 and drop empties like `depth_reject` in `crates/render/src/decals.rs` drops air or buried prints? Flag: rects outside 0..1 painting outside the map. (render)
3. **Facing discard.** Ask: does `queue` drop paints on backfaces like `crates/render/src/decals.rs::depth_reject` drops lifts over `DEPTH_REJECT_M`? Flag: paints on hidden skin that never show. (render)
4. **Seam fill.** Ask: does `drain` fill a `DILATE_PX` ring around each rect so seams share texels, mirroring closed joins in `crates/render/src/decals.rs`? Flag: cracks at UV seams with no dilate step. (render)
5. **Capped drain.** Ask: does `drain` return at most `paints_per_frame` in `(tick, char_id)` order like `DecalPool::place` order in `crates/render/src/decals.rs` uses `next_seq`? Flag: uncapped drain or hash-order walk. (render)
6. **Oldest evicted.** Ask: when over `cap_bytes` does it evict the oldest character map and report it like `DecalPool::evict_oldest` in `crates/render/src/decals.rs`? Flag: newest dropped or silent loss. (render)
7. **Same bytes twice.** Ask: does the same queue give the same drain bytes twice like `DecalPool::state_bytes` in `crates/render/src/decals.rs`? Flag: map iteration that drifts run to run. (render)
8. **Byte count.** Ask: does it track `used_bytes` against `cap_bytes` like `DecalAtlas::used_bytes` in `crates/render/src/decals.rs`? Flag: no byte total or cap ignored. (render)
9. **Never in hash.** Ask: do wound bytes stay out of `state_hash` per `docs/HASH-LAW.md` like `crates/render/src/decals.rs` keeps wall clock in receipts only? Flag: wound bytes in tick or replay. (render)

## Do / Don't
- Do: keep per-fighter maps capped, drain in tick order with dilated seams, and prove the golden hash unchanged.
- Don't: merge fighters into one map, drain uncapped, or let wound bytes reach the tick.

source: Lengyel, Foundations of Game Engine Development Vol 2, ch. 10.1
