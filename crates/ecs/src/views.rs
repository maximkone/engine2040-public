//! views: optional iteration plus dynamic spawn bundles over spine columns.
//!
//! Owner: engine-ecs. Units: spine slots in index order. Determinism: slot
//! order only. Errors: [`BundleError`]. Budget: bundles build on setup;
//! [`for_each_optional`] borrows in place heap-free. No new storage.
//! Caller: `crates/ecs/src/spine.rs` (`Spine::spawn_bundle`,
//! `Spine::spawn_bundles`, `Spine::for_each_optional` over RC4 prefabs).

use std::any::Any;

use engine_core::fixed::Fixed;

use crate::spine::{AiState, AnimState, Body, Spine, Transform, Vitals};

/// What went wrong building or spawning a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// The spine held no free slot.
    Full,
    /// A runtime component matched no spine column.
    Unknown,
}

impl core::fmt::Display for BundleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Full => write!(f, "spine bundle: store is full"),
            Self::Unknown => write!(f, "spine bundle: unknown component"),
        }
    }
}

impl std::error::Error for BundleError {}

/// One prefab spawn bundle: the RC4 components present on one prefab node.
/// Absent columns spawn from defaults. Pattern: hecs bundle/entity_builder.
#[derive(Clone, Debug, Default)]
pub struct SpawnBundle {
    /// Pose column when the prefab carries one.
    pub transform: Option<Transform>,
    /// Body column when the prefab carries one.
    pub body: Option<Body>,
    /// Vitals column when the prefab carries one.
    pub vitals: Option<Vitals>,
    /// AI column when the prefab carries one.
    pub ai: Option<AiState>,
    /// Anim column when the prefab carries one.
    pub anim: Option<AnimState>,
}

impl SpawnBundle {
    /// Insert one component by runtime type (the dynamic-spawn path).
    pub fn push_any(&mut self, mut value: Box<dyn Any>) -> Result<(), BundleError> {
        match value.downcast::<Transform>() {
            Ok(hit) => {
                self.transform = Some(*hit);
                return Ok(());
            }
            Err(back) => value = back,
        }
        match value.downcast::<Body>() {
            Ok(hit) => {
                self.body = Some(*hit);
                return Ok(());
            }
            Err(back) => value = back,
        }
        match value.downcast::<Vitals>() {
            Ok(hit) => {
                self.vitals = Some(*hit);
                return Ok(());
            }
            Err(back) => value = back,
        }
        match value.downcast::<AiState>() {
            Ok(hit) => {
                self.ai = Some(*hit);
                return Ok(());
            }
            Err(back) => value = back,
        }
        match value.downcast::<AnimState>() {
            Ok(hit) => {
                self.anim = Some(*hit);
                return Ok(());
            }
            Err(back) => value = back,
        }
        let _ = value;
        Err(BundleError::Unknown)
    }

    /// Spawn one slot; missing columns keep spine defaults. Setup path only.
    pub fn spawn_into(&self, spine: &mut Spine) -> Result<usize, BundleError> {
        let slot = spine
            .spawn(
                self.transform.unwrap_or_default(),
                self.body.unwrap_or_default(),
                self.vitals.unwrap_or_default(),
            )
            .map_err(|_| BundleError::Full)?;
        let Some(cell) = spine.slot_mut(slot) else {
            return Err(BundleError::Full);
        };
        *cell.ai = self.ai.unwrap_or_default();
        *cell.anim = self.anim.unwrap_or_default();
        Ok(slot)
    }
}

/// One live slot: required columns plus prefab-absent optionals. `body` is
/// absent for a degenerate capsule, `ai` for an undecided source, `anim`
/// for an unassigned pose. Pattern: shipyard optional.
pub struct OptionalSlot<'a> {
    /// Pose column, always present on a live slot.
    pub transform: &'a mut Transform,
    /// Vitals column, always present on a live slot.
    pub vitals: &'a mut Vitals,
    /// Body column unless the capsule is degenerate.
    pub body: Option<&'a mut Body>,
    /// AI column unless no intent was decided yet.
    pub ai: Option<&'a mut AiState>,
    /// Anim column unless no pose was assigned yet.
    pub anim: Option<&'a mut AnimState>,
}

/// Visit every live slot in index order through [`OptionalSlot`].
/// Tick path: borrows in place, no heap use.
pub fn for_each_optional(spine: &mut Spine, mut f: impl FnMut(usize, OptionalSlot<'_>)) {
    for slot in 0..spine.len() {
        if !spine.is_alive(slot) {
            continue;
        }
        if let Some(cell) = spine.slot_mut(slot) {
            let show_body = cell.body.radius != Fixed::ZERO || cell.body.height != Fixed::ZERO;
            let show_ai = cell.ai.source != AiState::FROM_NONE;
            let show_anim = cell.anim.pose_ref != 0;
            f(
                slot,
                OptionalSlot {
                    transform: cell.transform,
                    vitals: cell.vitals,
                    body: if show_body { Some(cell.body) } else { None },
                    ai: if show_ai { Some(cell.ai) } else { None },
                    anim: if show_anim { Some(cell.anim) } else { None },
                },
            );
        }
    }
}

/// Spawn every bundle in order, then count live slots on one optional pass.
/// Returns the live-slot total. Production caller for [`SpawnBundle`] plus
/// [`for_each_optional`].
pub fn spawn_prefab_batch(
    spine: &mut Spine,
    bundles: &[SpawnBundle],
) -> Result<usize, BundleError> {
    for bundle in bundles {
        bundle.spawn_into(spine)?;
    }
    let mut live = 0usize;
    for_each_optional(spine, |_, _| live += 1);
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::Intent;
    use engine_core::alloc_probe::measure;

    fn catalog() -> Vec<SpawnBundle> {
        let mut out = Vec::with_capacity(20);
        for i in 0..20u32 {
            let w = u16::try_from(i).expect("index fits u16");
            let f = f32::from(w);
            out.push(SpawnBundle {
                transform: Some(Transform {
                    pos: [f * 1.5, 0.0, -f],
                    yaw: f * 0.25,
                }),
                body: Some(Body {
                    vel: [Fixed::ZERO, Fixed::ZERO],
                    radius: Fixed::from_bits(26_214),
                    height: Fixed::from_bits(117_965),
                }),
                vitals: Some(Vitals {
                    hp_permille: 1000u16.saturating_sub(w * 10),
                    stamina: 100,
                    poise: 0,
                    state: 0,
                }),
                ai: (i % 2 == 0).then(|| AiState {
                    intent: Intent {
                        mx: 10,
                        mz: -5,
                        facing: 900,
                        buttons: 1,
                    },
                    clock: i,
                    source: AiState::FROM_BRAIN,
                }),
                anim: (i % 3 == 0).then(|| AnimState {
                    loco: 1,
                    phase: 0.1,
                    facing: [0.0, 1.0],
                    airborne: false,
                    interrupt: false,
                    pose_ref: i + 1,
                }),
            });
        }
        out
    }

    #[test]
    fn optional_views_prefab_spawns_match_bytes_twice() {
        let catalog = catalog();
        let run_once = || {
            let mut first = Spine::with_capacity(10);
            let mut second = Spine::with_capacity(10);
            let live_first =
                spawn_prefab_batch(&mut first, &catalog[..10]).expect("ten spawns fit");
            let live_second =
                spawn_prefab_batch(&mut second, &catalog[10..]).expect("ten spawns fit");
            assert_eq!((live_first, live_second), (10, 10));
            (first.snapshot(), second.snapshot())
        };
        let (a_first, a_second) = run_once();
        let (b_first, b_second) = run_once();
        assert_eq!(a_first, b_first, "first ten spawns repeat byte-identical");
        assert_eq!(
            a_second, b_second,
            "second ten spawns repeat byte-identical"
        );
        let mut moved = Spine::with_capacity(10);
        spawn_prefab_batch(&mut moved, &catalog[..10]).expect("fits");
        for_each_optional(&mut moved, |slot, view| {
            if slot == 0 {
                view.transform.pos[0] += 0.125;
            }
        });
        assert_ne!(
            moved.snapshot().bytes,
            a_first.bytes,
            "oracle must move bytes"
        );
    }

    #[test]
    fn optional_views_tick_path_allocates_nothing() {
        let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
        let catalog = catalog();
        let mut spine = Spine::with_capacity(10);
        spawn_prefab_batch(&mut spine, &catalog[..10]).expect("fits");
        for_each_optional(&mut spine, |_, _| {});
        let ((), heap) = measure(|| {
            for tick in 0..64u32 {
                for_each_optional(&mut spine, |_, view| {
                    view.transform.yaw += 0.001;
                    if let Some(ai) = view.ai {
                        ai.clock = tick;
                    }
                });
                std::hint::black_box(spine.column_digests());
            }
        });
        assert_eq!(heap.calls, 0, "optional tick pass allocated");
    }

    #[test]
    fn optional_views_refusals_are_loud() {
        let catalog = catalog();
        let mut full = Spine::with_capacity(1);
        assert_eq!(
            spawn_prefab_batch(&mut full, &catalog[..2]),
            Err(BundleError::Full)
        );
        let mut bundle = SpawnBundle::default();
        assert_eq!(bundle.push_any(Box::new(7u16)), Err(BundleError::Unknown));
        bundle
            .push_any(Box::new(Transform {
                pos: [1.0, 0.0, 0.0],
                yaw: 0.0,
            }))
            .expect("transform is a spine column");
        assert!(bundle.transform.is_some());
        let mut spine = Spine::with_capacity(2);
        let slot = bundle.spawn_into(&mut spine).expect("fits");
        assert!(spine.kill(slot));
        let (mut live, mut ai_seen) = (0usize, 0usize);
        for_each_optional(&mut spine, |_, view| {
            live += 1;
            ai_seen += usize::from(view.ai.is_some());
        });
        assert_eq!((live, ai_seen), (0, 0), "dead slots never visit");
    }
}
