//! phase_warp: marker-driven attack phase warp with a telegraph meter (GP44).
//!
//! Owner: `engine-anim`; consumer: the look's attack clock (the sim's armed
//! startup says the blow lands `n` ticks after the press, the clip says its
//! impact frame sits `impact` clip ticks in). A clip authored at one speed
//! meets the hit only for one weapon; this maps real ticks onto clip ticks so
//! the clip's impact frame lands on the sim's impact tick for every lead.
//!
//! Markers come from the clip's event track ([`WINDUP_MARKER_CODE`],
//! [`IMPACT_MARKER_CODE`], [`RECOVER_MARKER_CODE`]): the windup is the span
//! from the windup-start marker to the impact marker, the recovery plays after
//! it. The windup stretches to fill the lead at up to [`MAX_STRETCH`] (2x, half
//! clip rate); past that the clip holds its last windup frame, so the impact
//! pose is never drawn before the sim's impact tick and the impact frame is
//! never late. A lead shorter than the windup squeezes it (faster than 1:1,
//! uncapped: the hit is the truth). After the impact the clip plays 1:1.
//!
//! Presentation only: pure maps of integer ticks, no clock, no state, no
//! allocation, and the sim hash never reads it. Units: ticks at 60 Hz in,
//! clip ticks and a 0..1 meter out. Every function is total; a lead of zero
//! means the blow lands now (the clip starts on its impact frame).

use super::events::{IMPACT_MARKER_CODE, RECOVER_MARKER_CODE, WINDUP_MARKER_CODE};
use super::graph::TimedEvent;

/// Largest windup stretch, as a multiplier: half the clip rate at most, so a
/// long lead holds the peak instead of crawling through the windup.
pub const MAX_STRETCH: f32 = 2.0;

/// The three attack markers of one clip, in clip ticks at 60 Hz.
///
/// Ordering `windup_start <= impact <= recover` is an authoring invariant;
/// [`ClipMarkers::new`] refuses anything else rather than warping a broken
/// track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClipMarkers {
    /// First tick of the windup; the warp starts the stretch here.
    pub windup_start: u32,
    /// The impact frame; it meets the sim's impact tick exactly.
    pub impact: u32,
    /// First tick after the impact; the clip plays 1:1 from here.
    pub recover: u32,
}

impl ClipMarkers {
    /// Markers in clip ticks; the ordering is checked, not repaired.
    pub fn new(windup_start: u32, impact: u32, recover: u32) -> Result<Self, PhaseWarpError> {
        if windup_start > impact || impact > recover {
            return Err(PhaseWarpError::Order {
                windup_start,
                impact,
                recover,
            });
        }
        Ok(Self {
            windup_start,
            impact,
            recover,
        })
    }

    /// Read the markers off a clip event track; each marker code appears once.
    pub fn from_cues(cues: &[TimedEvent]) -> Result<Self, PhaseWarpError> {
        let windup_start = marker_tick(cues, WINDUP_MARKER_CODE)?;
        let impact = marker_tick(cues, IMPACT_MARKER_CODE)?;
        let recover = marker_tick(cues, RECOVER_MARKER_CODE)?;
        Self::new(windup_start, impact, recover)
    }

    /// Windup length in clip ticks: zero when the clip has no windup.
    #[must_use]
    pub fn windup_ticks(&self) -> u32 {
        self.impact - self.windup_start
    }
}

/// The one cue carrying `code`; a missing or repeated marker is an error.
fn marker_tick(cues: &[TimedEvent], code: u32) -> Result<u32, PhaseWarpError> {
    let mut found = None;
    for cue in cues {
        if cue.code != code {
            continue;
        }
        if found.is_some() {
            return Err(PhaseWarpError::DuplicateMarker { code });
        }
        found = Some(cue.tick);
    }
    found.ok_or(PhaseWarpError::MissingMarker { code })
}

/// Typed marker failures; every variant names the value at fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhaseWarpError {
    /// The markers do not run windup start, impact, recover.
    Order {
        /// Windup-start tick as given.
        windup_start: u32,
        /// Impact tick as given.
        impact: u32,
        /// Recover tick as given.
        recover: u32,
    },
    /// No cue carries the marker `code`.
    MissingMarker {
        /// Marker code the track lacks.
        code: u32,
    },
    /// More than one cue carries the marker `code`.
    DuplicateMarker {
        /// Marker code the track repeats.
        code: u32,
    },
}

impl std::fmt::Display for PhaseWarpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PhaseWarpError::Order {
                windup_start,
                impact,
                recover,
            } => write!(
                f,
                "phase_warp: markers {windup_start} <= {impact} <= {recover} is false"
            ),
            PhaseWarpError::MissingMarker { code } => {
                write!(f, "phase_warp: no marker with code {code}")
            }
            PhaseWarpError::DuplicateMarker { code } => {
                write!(f, "phase_warp: marker code {code} appears twice")
            }
        }
    }
}

impl std::error::Error for PhaseWarpError {}

/// One attack's warp: markers plus the sim's ticks to the impact.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PhaseWarp {
    markers: ClipMarkers,
    ticks_to_impact: u32,
}

impl PhaseWarp {
    /// Warp `markers` so the impact frame meets `ticks_to_impact` from now.
    pub fn new(markers: ClipMarkers, ticks_to_impact: u32) -> Self {
        Self {
            markers,
            ticks_to_impact,
        }
    }

    /// Markers this warp was built from.
    #[must_use]
    pub fn markers(&self) -> ClipMarkers {
        self.markers
    }

    /// Sim ticks from the press to the impact, as given.
    #[must_use]
    pub fn ticks_to_impact(&self) -> u32 {
        self.ticks_to_impact
    }

    /// Windup stretch this lead asks for: `1.0` at the authored rate, above
    /// `1.0` stretched (capped at [`MAX_STRETCH`]), below `1.0` squeezed. A
    /// clip with no windup reports `1.0`.
    #[must_use]
    pub fn stretch(&self) -> f32 {
        let windup = to_f32(self.markers.windup_ticks());
        if windup <= 0.0 {
            return 1.0;
        }
        (to_f32(self.ticks_to_impact) / windup).min(MAX_STRETCH)
    }

    /// Clip time `since` real ticks after the press.
    ///
    /// At `since == ticks_to_impact` this is exactly the impact marker. The
    /// map never runs backwards and never leaves the windup faster than
    /// [`MAX_STRETCH`]; while the cap binds it holds the last windup frame
    /// (`impact - 1`), never the impact pose. A non-finite or negative `since`
    /// reads as the press tick.
    #[must_use]
    pub fn clip_time(&self, since: f32) -> f32 {
        let since = clamp_since(since);
        let lead = to_f32(self.ticks_to_impact);
        let start = to_f32(self.markers.windup_start);
        let impact = to_f32(self.markers.impact);
        if since >= lead {
            // The hit has landed: the recovery is the clip's own rate.
            return impact + (since - lead);
        }
        let windup = to_f32(self.markers.windup_ticks());
        if windup <= 0.0 {
            // No windup to warp; the clip already sits on its impact frame.
            return start;
        }
        let natural = since * windup / lead;
        let capped = since / MAX_STRETCH;
        if capped > natural {
            start + capped.min(windup - 1.0)
        } else {
            start + natural
        }
    }

    /// Telegraph meter: `0.0` on the press, `1.0` on the impact tick and after.
    /// A lead of zero reads `1.0`: there is no windup to show.
    #[must_use]
    pub fn telegraph(&self, since: f32) -> f32 {
        let lead = to_f32(self.ticks_to_impact);
        if lead <= 0.0 {
            return 1.0;
        }
        (clamp_since(since) / lead).min(1.0)
    }
}

/// Non-finite and negative reads as the press tick, so the maps stay total.
fn clamp_since(since: f32) -> f32 {
    if since.is_finite() && since > 0.0 {
        since
    } else {
        0.0
    }
}

/// Tick counts are small; the saturating conversion keeps the cast lint out.
fn to_f32(ticks: u32) -> f32 {
    f32::from(u16::try_from(ticks).unwrap_or(u16::MAX))
}

#[cfg(test)]
#[path = "phase_warp_test.rs"]
mod tests;
