//! temporal: named history textures that live across frames, with a
//! validity flag (RL62).
//!
//! A history texture (the TAA colour, a GTAO or fog accumulation) is known
//! by name and survives frames. Its life has the kajiya shape: between
//! frames it is inert; a pass takes it (reads last frame, writes this
//! frame) and exports it back when this frame's result is ready; the frame
//! end makes every exported texture inert again. Taking a texture twice in
//! a frame is refused, so two passes can never fight over one history.
//!
//! The flag `history_valid` says whether the pixels in a texture belong to
//! the scene the reader is drawing now. A first use, a resize, a dyn-res
//! step (all three change [`HistoryDesc`]) and a camera cut
//! ([`TemporalRegistry::camera_cut`]) make it false; the next export makes
//! it true. A reader that sees false must not blend the old pixels: the TAA
//! resolve does this through [`crate::post::TaaParams::with_history`].
//!
//! Units: pixels and bytes. Determinism: pure state machine, no clock, no
//! GPU, never part of a simulation hash. Errors: typed [`TemporalError`].
//! Caller: the TAA chain reads the flag through `TaaParams`; the GPU owner
//! of a history texture reallocates when [`Take::recreated`] is true.

use std::collections::BTreeMap;
use std::fmt;

/// Size identity of one history texture. A different size is a different
/// texture: the old pixels do not map onto the new ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryDesc {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per texel (the format).
    pub bytes_per_texel: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Between frames: no pass holds the texture.
    Inert,
    /// A pass took it this frame and has not exported yet.
    Taken,
    /// The pass exported it; it turns inert at the frame end.
    Exported,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    desc: HistoryDesc,
    phase: Phase,
    valid: bool,
}

/// What a pass gets from [`TemporalRegistry::take`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Take {
    /// The history pixels belong to the scene being drawn; false means read
    /// nothing from them.
    pub history_valid: bool,
    /// The texture is new or changed size: the owner must (re)allocate it.
    pub recreated: bool,
}

/// Why a take, export or frame end was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalError {
    /// The texture was already taken this frame.
    AlreadyTaken(String),
    /// Export of a texture no pass took this frame (or exported twice).
    NotTaken(String),
    /// A pass took the texture and never exported it by the frame end.
    NeverExported(String),
}

impl fmt::Display for TemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyTaken(name) => write!(f, "history {name} is already taken this frame"),
            Self::NotTaken(name) => write!(f, "history {name} is not taken, nothing to export"),
            Self::NeverExported(name) => write!(f, "history {name} was taken and never exported"),
        }
    }
}

impl std::error::Error for TemporalError {}

/// The registry of named history textures.
#[derive(Debug, Default)]
pub struct TemporalRegistry {
    entries: BTreeMap<String, Entry>,
}

impl TemporalRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Textures known by name.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True before the first take.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The flag of one texture; false for a name never taken.
    pub fn history_valid(&self, name: &str) -> bool {
        self.entries.get(name).is_some_and(|entry| entry.valid)
    }

    /// A pass takes the texture `name` of size `desc` for this frame. A new
    /// name, or a changed size (resize, dyn-res step), recreates it with
    /// `history_valid` false. A second take before the frame end is refused.
    pub fn take(&mut self, name: &str, desc: HistoryDesc) -> Result<Take, TemporalError> {
        match self.entries.get_mut(name) {
            Some(entry) if entry.phase != Phase::Inert => {
                Err(TemporalError::AlreadyTaken(name.to_string()))
            }
            Some(entry) => {
                let recreated = entry.desc != desc;
                if recreated {
                    entry.desc = desc;
                    entry.valid = false;
                }
                entry.phase = Phase::Taken;
                Ok(Take {
                    history_valid: entry.valid,
                    recreated,
                })
            }
            None => {
                let entry = Entry {
                    desc,
                    phase: Phase::Taken,
                    valid: false,
                };
                self.entries.insert(name.to_string(), entry);
                Ok(Take {
                    history_valid: false,
                    recreated: true,
                })
            }
        }
    }

    /// The pass hands the texture back with this frame's result in it: the
    /// next frame may read it.
    pub fn export(&mut self, name: &str) -> Result<(), TemporalError> {
        match self.entries.get_mut(name) {
            Some(entry) if entry.phase == Phase::Taken => {
                entry.phase = Phase::Exported;
                entry.valid = true;
                Ok(())
            }
            _ => Err(TemporalError::NotTaken(name.to_string())),
        }
    }

    /// A camera cut: every texture keeps its memory but its pixels belong to
    /// another shot, so each flag turns false. Call between frames.
    pub fn camera_cut(&mut self) {
        for entry in self.entries.values_mut() {
            entry.valid = false;
        }
    }

    /// End the frame: every exported texture turns inert. A texture taken
    /// and never exported turns inert and invalid, and the first such name
    /// is reported.
    pub fn end_frame(&mut self) -> Result<(), TemporalError> {
        let mut lost = None;
        for (name, entry) in &mut self.entries {
            if entry.phase == Phase::Taken {
                entry.valid = false;
                lost.get_or_insert_with(|| name.clone());
            }
            entry.phase = Phase::Inert;
        }
        lost.map_or(Ok(()), |name| Err(TemporalError::NeverExported(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aa_scale::{ScaleController, ScalePolicy};
    use crate::post::{taa_resolve, MotionField, TaaParams};

    const DESC: HistoryDesc = HistoryDesc {
        width: 1920,
        height: 1080,
        bytes_per_texel: 8,
    };

    fn frame(reg: &mut TemporalRegistry, desc: HistoryDesc) -> Take {
        let take = reg.take("taa", desc).expect("take");
        reg.export("taa").expect("export");
        reg.end_frame().expect("end frame");
        take
    }

    #[test]
    fn temporal_first_take_is_new_and_invalid_then_the_export_makes_it_valid() {
        let mut reg = TemporalRegistry::new();
        assert!(!reg.history_valid("taa"));
        assert_eq!(
            frame(&mut reg, DESC),
            Take {
                history_valid: false,
                recreated: true
            }
        );
        assert!(reg.history_valid("taa"));
        assert_eq!(
            frame(&mut reg, DESC),
            Take {
                history_valid: true,
                recreated: false
            }
        );
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn temporal_resize_flips_the_flag() {
        let mut reg = TemporalRegistry::new();
        frame(&mut reg, DESC);
        assert!(frame(&mut reg, DESC).history_valid);
        let smaller = HistoryDesc {
            width: 1280,
            height: 720,
            ..DESC
        };
        let resized = frame(&mut reg, smaller);
        assert_eq!(
            resized,
            Take {
                history_valid: false,
                recreated: true
            }
        );
        assert!(
            reg.history_valid("taa"),
            "the resized texture is valid after its export"
        );
        assert!(frame(&mut reg, smaller).history_valid);
    }

    #[test]
    fn temporal_dyn_res_step_recreates_the_history() {
        let mut ctl = ScaleController::new(ScalePolicy::default()).expect("policy");
        let size = |ctl: &ScaleController| {
            let (width, height) = ctl.render_size(1920, 1080).expect("size");
            HistoryDesc {
                width,
                height,
                bytes_per_texel: 8,
            }
        };
        let mut reg = TemporalRegistry::new();
        frame(&mut reg, size(&ctl));
        assert!(frame(&mut reg, size(&ctl)).history_valid);
        // A 3x spike steps the scale down; the history must not survive it.
        for _ in 0..4 {
            ctl.update(50.0).expect("spike frame");
        }
        assert!(ctl.scale() < 1.0);
        let stepped = frame(&mut reg, size(&ctl));
        assert!(!stepped.history_valid && stepped.recreated);
    }

    #[test]
    fn temporal_double_take_is_refused() {
        let mut reg = TemporalRegistry::new();
        reg.take("taa", DESC).expect("first take");
        assert_eq!(
            reg.take("taa", DESC),
            Err(TemporalError::AlreadyTaken("taa".to_string()))
        );
        // Still refused after the export, until the frame ends.
        reg.export("taa").expect("export");
        assert_eq!(
            reg.take("taa", DESC),
            Err(TemporalError::AlreadyTaken("taa".to_string()))
        );
        reg.end_frame().expect("end frame");
        assert!(reg.take("taa", DESC).is_ok());
    }

    #[test]
    fn temporal_export_needs_a_take_and_a_taken_texture_must_be_exported() {
        let mut reg = TemporalRegistry::new();
        assert_eq!(
            reg.export("taa"),
            Err(TemporalError::NotTaken("taa".to_string()))
        );
        frame(&mut reg, DESC);
        assert!(reg.history_valid("taa"));
        reg.take("taa", DESC).expect("take");
        assert_eq!(
            reg.end_frame(),
            Err(TemporalError::NeverExported("taa".to_string()))
        );
        assert!(!reg.history_valid("taa"), "a lost frame leaves no history");
        assert!(reg.take("taa", DESC).is_ok(), "the registry recovered");
    }

    #[test]
    fn temporal_camera_cut_invalidates_every_texture() {
        let mut reg = TemporalRegistry::new();
        for name in ["taa", "gtao", "fog"] {
            reg.take(name, DESC).expect("take");
            reg.export(name).expect("export");
        }
        reg.end_frame().expect("end frame");
        assert!(["taa", "gtao", "fog"].iter().all(|n| reg.history_valid(n)));
        reg.camera_cut();
        assert!(["taa", "gtao", "fog"].iter().all(|n| !reg.history_valid(n)));
        let take = reg.take("taa", DESC).expect("take after the cut");
        assert_eq!(
            take,
            Take {
                history_valid: false,
                recreated: false
            }
        );
    }

    /// A 16x16 stripe frame: `vertical` stripes alternate along x, else along y.
    fn stripes(vertical: bool, bright: f32, dark: f32) -> Vec<[f32; 3]> {
        (0..16usize * 16)
            .map(|i| {
                let (x, y) = (i % 16, i / 16);
                let on = if vertical { x % 2 == 0 } else { y % 2 == 0 };
                let v = if on { bright } else { dark };
                [v, v, v]
            })
            .collect()
    }

    /// Pixels whose resolved value differs from the current frame.
    fn ghost_pixels(resolved: &[[f32; 3]], current: &[[f32; 3]]) -> usize {
        resolved
            .iter()
            .zip(current)
            .filter(|(r, c)| r.iter().zip(c.iter()).any(|(a, b)| (a - b).abs() > 1e-6))
            .count()
    }

    #[test]
    fn temporal_camera_cut_fixture_has_zero_ghost_pixels() {
        let history = stripes(true, 4.0, 0.0);
        let current = stripes(false, 1.0, 0.1);
        let motion = MotionField::zero(16, 16).expect("motion");
        let resolve = |params: &TaaParams| {
            taa_resolve(&current, &history, &motion, 16, 16, params).expect("resolve")
        };

        // Without the flag the old shot leaks into the new one.
        let stale = resolve(&TaaParams::default());
        assert!(ghost_pixels(&stale, &current) > 0, "the fixture must ghost");

        // The registry knows the cut; the resolve follows the current frame.
        let desc = HistoryDesc {
            width: 16,
            height: 16,
            bytes_per_texel: 12,
        };
        let mut reg = TemporalRegistry::new();
        frame(&mut reg, desc);
        assert!(frame(&mut reg, desc).history_valid);
        reg.camera_cut();
        let take = reg.take("taa", desc).expect("take after the cut");
        let cut = resolve(&TaaParams::default().with_history(take.history_valid));
        assert_eq!(ghost_pixels(&cut, &current), 0);
        reg.export("taa").expect("export");
        reg.end_frame().expect("end frame");

        // The frame after the cut blends history again.
        let next = reg.take("taa", desc).expect("next frame");
        assert!(next.history_valid);
    }
}
