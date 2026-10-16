//! Wound paint budget queue: one paint list per character, capped drain.
//!
//! Holds one bounding paint per hit tagged with its owning character id so
//! a paint never leaks onto the wrong body. Drain returns at most
//! `paints_per_frame` paints per frame in stable `(tick, char_id)` order so
//! the same inputs give the same bytes twice. When resident bytes run over
//! `cap_bytes`, the whole oldest character map is evicted first and reported
//! through [`WoundMaps::last_evict`]. Std-only: no GPU, no wall clock, and
//! wound bytes never enter the simulation hash.

/// Intake companion (rect validation, facing gate, order key, seam dilate).
#[path = "wound_queue.rs"]
mod wound_queue;

pub use wound_queue::DILATE_PX;
use wound_queue::{clamp_rect, is_front_facing, paint_order};
pub use wound_queue::{dilate_uv, Paint, UvRect, WoundKind};

/// Fixed budget charge per queued paint in bytes.
///
/// The number is a logical budget charge (paint record plus atlas tile
/// bookkeeping reserve), not `size_of::<Paint>()`. A fixed charge keeps the
/// byte cap stable across platforms so the same queue evicts the same map
/// twice.
pub const PAINT_BYTES: usize = 64;

/// Eviction report: which character map was dropped and how many paints it held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EvictReport {
    /// Evicted character id (the oldest map).
    pub char_id: u32,
    /// Paints dropped with that map.
    pub dropped: usize,
}

/// Stored paint in arrival order; the drain sorts indices with a stable
/// sort so `(tick, char_id)` ties keep queue order.
struct StoredPaint {
    /// Queued paint.
    paint: Paint,
}

/// Per-character wound paint budget queue.
///
/// Keeps one paint list per `char_id` in first-seen order, drains at most
/// `paints_per_frame` paints per frame in `(tick, char_id)` order, and evicts
/// the whole oldest character map when resident bytes run over `cap_bytes`.
pub struct WoundMaps {
    /// All queued paints in arrival order.
    entries: Vec<StoredPaint>,
    /// Distinct character ids in first-seen order; front is the oldest map.
    char_order: Vec<u32>,
    /// Byte budget; each paint costs [`PAINT_BYTES`].
    cap_bytes: usize,
    /// Max paints returned per [`WoundMaps::drain`] call.
    paints_per_frame: usize,
    /// Most recent whole-map eviction, if any.
    last_evict: Option<EvictReport>,
}

impl WoundMaps {
    /// New empty queue with a byte budget and a per-frame drain cap.
    pub fn new(cap_bytes: usize, paints_per_frame: usize) -> Self {
        Self {
            entries: Vec::new(),
            char_order: Vec::new(),
            cap_bytes,
            paints_per_frame,
            last_evict: None,
        }
    }

    /// Queue one bounding paint for one hit on `char_id`.
    ///
    /// The rect is clamped to 0..1 UV; empty results are dropped and report
    /// `false`. Over `cap_bytes`, the whole oldest character map is evicted
    /// first (see [`WoundMaps::last_evict`]); when that map is our own, the
    /// just-pushed paint is gone too and the call reports `false`.
    pub fn queue(&mut self, char_id: u32, rect: UvRect, kind: WoundKind, tick: u32) -> bool {
        let Some(rect) = clamp_rect(rect) else {
            return false;
        };
        self.push(char_id, rect, kind, tick);
        self.entries.iter().any(|s| s.paint.char_id == char_id)
    }

    /// Queue one paint gated on facing: cosine of surface normal and view
    /// direction. Backface and non-finite facings are dropped (`false`),
    /// mirroring the atlas depth reject for hidden skin.
    pub fn queue_facing(
        &mut self,
        char_id: u32,
        rect: UvRect,
        kind: WoundKind,
        tick: u32,
        facing: f32,
    ) -> bool {
        if !is_front_facing(facing) {
            return false;
        }
        self.queue(char_id, rect, kind, tick)
    }

    /// Drain up to `paints_per_frame` paints in `(tick, char_id)` order.
    ///
    /// `frame` is informational only (the presenting frame number for order
    /// checks); it changes neither membership nor order. The same queue plus
    /// the same drains gives equal vecs twice.
    pub fn drain(&mut self, frame: u32) -> Vec<Paint> {
        let _ = frame;
        if self.paints_per_frame == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        // Indices start in arrival order; `sort_by` is stable so
        // `(tick, char_id)` ties keep queue order.
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by(|&a, &b| paint_order(&self.entries[a].paint, &self.entries[b].paint));
        let take = self.paints_per_frame.min(order.len());
        let mut out: Vec<Paint> = Vec::with_capacity(take);
        let mut taken: Vec<usize> = Vec::with_capacity(take);
        for &idx in order.iter().take(take) {
            out.push(self.entries[idx].paint);
            taken.push(idx);
        }
        // Remove highest index first so the survivors keep arrival order
        // and later ties still break by queue order.
        taken.sort_unstable();
        for idx in taken.into_iter().rev() {
            self.entries.remove(idx);
        }
        self.char_order
            .retain(|c| self.entries.iter().any(|s| s.paint.char_id == *c));
        out
    }

    /// Resident bytes: queued paints times [`PAINT_BYTES`].
    pub fn resident_bytes(&self) -> usize {
        self.entries.len() * PAINT_BYTES
    }

    /// Most recent whole-map eviction, if any.
    pub fn last_evict(&self) -> Option<EvictReport> {
        self.last_evict
    }

    /// Push a clamped paint and enforce the byte cap.
    fn push(&mut self, char_id: u32, rect: UvRect, kind: WoundKind, tick: u32) {
        if !self.char_order.contains(&char_id) {
            self.char_order.push(char_id);
        }
        self.entries.push(StoredPaint {
            paint: Paint {
                tick,
                char_id,
                rect,
                kind,
            },
        });
        self.enforce_cap();
    }

    /// Drop the whole oldest character map until under `cap_bytes`.
    fn enforce_cap(&mut self) {
        while self.resident_bytes() > self.cap_bytes && !self.entries.is_empty() {
            let Some(&victim) = self.char_order.first() else {
                break;
            };
            let mut dropped: usize = 0;
            self.entries.retain(|s| {
                if s.paint.char_id == victim {
                    dropped += 1;
                    false
                } else {
                    true
                }
            });
            self.char_order.retain(|c| *c != victim);
            self.last_evict = Some(EvictReport {
                char_id: victim,
                dropped,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rect helper: distinct non-empty UV rect per tick.
    fn rect(tick: u32) -> UvRect {
        let u0 = (tick % 16) as f32 / 16.0;
        UvRect {
            u0,
            v0: 0.0,
            u1: u0 + 1.0 / 16.0,
            v1: 0.25,
        }
    }

    #[test]
    fn wound_drain_caps_per_frame_in_order() {
        let mut maps = WoundMaps::new(PAINT_BYTES * 16, 4);
        // Queue order is scrambled on purpose: (char_id, tick).
        let queued: [(u32, u32); 10] = [
            (9, 4),
            (9, 1),
            (3, 1),
            (3, 0),
            (9, 3),
            (3, 2),
            (9, 7),
            (3, 5),
            (9, 6),
            (3, 8),
        ];
        for (i, &(char_id, tick)) in queued.iter().enumerate() {
            let kind = if i % 2 == 0 {
                WoundKind::Cut
            } else {
                WoundKind::Bruise
            };
            assert!(maps.queue(char_id, rect(tick), kind, tick));
        }
        let first = maps.drain(100);
        let second = maps.drain(101);
        let third = maps.drain(102);
        assert_eq!(first.len(), 4);
        assert_eq!(second.len(), 4);
        assert_eq!(third.len(), 2);
        let mut keys: Vec<(u32, u32)> = Vec::new();
        for p in first.iter().chain(second.iter()).chain(third.iter()) {
            keys.push((p.tick, p.char_id));
        }
        assert_eq!(
            keys,
            vec![
                (0, 3),
                (1, 3),
                (1, 9),
                (2, 3),
                (3, 9),
                (4, 9),
                (5, 3),
                (6, 9),
                (7, 9),
                (8, 3),
            ]
        );
    }

    #[test]
    fn wound_evicts_oldest_map_over_cap() {
        // Cap holds 8 paints; queuing 10 across two bodies must drop the
        // whole oldest map (char 7, five paints).
        let mut maps = WoundMaps::new(PAINT_BYTES * 8, 4);
        for tick in 0..5 {
            assert!(maps.queue(7, rect(tick), WoundKind::Cut, tick));
        }
        for tick in 5..10 {
            assert!(maps.queue(9, rect(tick), WoundKind::Burn, tick));
        }
        assert!(maps.resident_bytes() <= PAINT_BYTES * 8);
        assert_eq!(
            maps.last_evict(),
            Some(EvictReport {
                char_id: 7,
                dropped: 5,
            })
        );
        let mut all: Vec<Paint> = Vec::new();
        all.extend(maps.drain(200));
        all.extend(maps.drain(201));
        all.extend(maps.drain(202));
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|p| p.char_id == 9));
        let mut ticks: Vec<u32> = all.iter().map(|p| p.tick).collect();
        ticks.sort_unstable();
        assert_eq!(ticks, vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn wound_queue_reports_false_when_cap_evicts_own_map() {
        // Cap holds one paint; a second paint for the same body evicts the
        // whole map including the just-pushed paint, so the call reports
        // `false` and the eviction names the body.
        let mut maps = WoundMaps::new(PAINT_BYTES, 4);
        assert!(maps.queue(1, rect(0), WoundKind::Cut, 0));
        assert!(!maps.queue(1, rect(1), WoundKind::Bruise, 1));
        assert_eq!(maps.resident_bytes(), 0);
        assert_eq!(
            maps.last_evict(),
            Some(EvictReport {
                char_id: 1,
                dropped: 2,
            })
        );
        // A zero cap stores nothing and reports `false` at once.
        let mut flat = WoundMaps::new(0, 4);
        assert!(!flat.queue(2, rect(2), WoundKind::Cut, 2));
        assert_eq!(flat.resident_bytes(), 0);
        // A fresh body queues fine after the eviction.
        assert!(maps.queue(2, rect(2), WoundKind::Cut, 2));
        assert_eq!(maps.resident_bytes(), PAINT_BYTES);
    }

    #[test]
    fn wound_stable_x2() {
        fn seed(maps: &mut WoundMaps) {
            let rows: [(u32, u32, WoundKind); 6] = [
                (5, 2, WoundKind::Cut),
                (6, 2, WoundKind::Bruise),
                (5, 0, WoundKind::Burn),
                (6, 1, WoundKind::Cut),
                (5, 1, WoundKind::Bruise),
                (6, 0, WoundKind::Burn),
            ];
            for &(char_id, tick, kind) in rows.iter() {
                assert!(maps.queue(char_id, rect(tick), kind, tick));
            }
        }
        let mut first = WoundMaps::new(PAINT_BYTES * 16, 3);
        let mut second = WoundMaps::new(PAINT_BYTES * 16, 3);
        seed(&mut first);
        seed(&mut second);
        let drain_a = first.drain(300);
        let drain_b = second.drain(300);
        assert_eq!(drain_a, drain_b);
        assert_eq!(first.resident_bytes(), second.resident_bytes());
        let drain_a2 = first.drain(301);
        let drain_b2 = second.drain(301);
        assert_eq!(drain_a2, drain_b2);
        assert_eq!(first.resident_bytes(), second.resident_bytes());
    }

    #[test]
    fn wound_drops_empty_rect_and_backface() {
        let mut maps = WoundMaps::new(PAINT_BYTES * 16, 4);
        let outside = UvRect {
            u0: 1.5,
            v0: 0.2,
            u1: 2.0,
            v1: 0.4,
        };
        assert!(!maps.queue(1, outside, WoundKind::Cut, 0));
        assert!(!maps.queue_facing(1, rect(0), WoundKind::Cut, 0, -0.5));
        assert!(!maps.queue_facing(1, rect(0), WoundKind::Cut, 0, 0.0));
        assert_eq!(maps.resident_bytes(), 0);
        assert!(maps.queue_facing(1, rect(0), WoundKind::Cut, 0, 0.5));
        assert_eq!(maps.resident_bytes(), PAINT_BYTES);
    }
}
