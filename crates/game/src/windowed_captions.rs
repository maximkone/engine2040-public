//! windowed_captions: accessibility captions for the live windowed session.
//!
//! Consumer: [`super::run_windowed`] pushes each accepted tick's fight cues
//! through this module, so the built `engine_ui::access::SubtitleFeed` and its
//! `FightNote` mirror have a live caller outside their own tests. Data owner:
//! `engine-ui` (the strip and the note shape); this module owns the cue mapping
//! and the HUD cell/quad count only. Units: ticks at the arena rate, cells and
//! quads from the UI text path. Determinism: pure over owned values; the strip
//! is oldest-first and bounded, and nothing here enters the sim hash. Failure
//! mode: a refused strip cap (never at [`DEFAULT_SUBTITLE_LINES`]) leaves
//! captions off, never a panic. Budget: one heap `Vec` sized to the tick's cue
//! count on the presentation path only; zero in the sim tick.

use engine_audio::FightCue;
use engine_input::FighterId;
use engine_ui::{
    access::{FightNote, SubtitleFeed, DEFAULT_SUBTITLE_LINES},
    text::{emit_quads, layout, Align, Atlas},
};

/// The live caption strip for one windowed session, or `None` when the cap is
/// refused. [`DEFAULT_SUBTITLE_LINES`] sits inside the accepted range, so the
/// shipped session always builds one.
#[must_use]
pub(crate) fn caption_strip() -> Option<SubtitleFeed> {
    SubtitleFeed::new(DEFAULT_SUBTITLE_LINES).ok()
}

/// Push one tick of `cues` into the caption strip (when it built) and count
/// the live strip. A refused strip (`None`) reports `(0, 0)`.
#[must_use]
pub(crate) fn push_caption_tick(
    feed: Option<&mut SubtitleFeed>,
    tick: u64,
    cues: &[FightCue],
) -> (usize, usize) {
    match feed {
        Some(strip) => subtitle_cells_quads(strip, tick, cues),
        None => (0, 0),
    }
}

/// One fight cue as a caption [`FightNote`], field for field, or `None` when a
/// slot is outside the participant range. Pure; the caption layer keeps no
/// damage amounts (the audio layer owns those).
fn fight_note(cue: FightCue) -> Option<FightNote> {
    let slot = FighterId::new;
    Some(match cue {
        FightCue::Hit {
            attacker, victim, ..
        } => FightNote::Hit {
            attacker: slot(attacker)?,
            victim: slot(victim)?,
        },
        FightCue::Blocked {
            attacker, victim, ..
        } => FightNote::Blocked {
            attacker: slot(attacker)?,
            victim: slot(victim)?,
        },
        FightCue::Parried { attacker, victim } => FightNote::Parried {
            attacker: slot(attacker)?,
            victim: slot(victim)?,
        },
        FightCue::Dodged { attacker, victim } => FightNote::Dodged {
            attacker: slot(attacker)?,
            victim: slot(victim)?,
        },
        FightCue::StaminaOut { fighter } => FightNote::StaminaOut {
            fighter: slot(fighter)?,
        },
        FightCue::Knockdown { attacker, victim } => FightNote::Knockdown {
            attacker: slot(attacker)?,
            victim: slot(victim)?,
        },
    })
}

/// Caption cells plus quads for one tick's fight cues through
/// [`SubtitleFeed`]: map each cue to a [`FightNote`], push them into the strip,
/// then lay the live lines out through the same UI text path as
/// `windowed::hud_cells_quads` (layout cells, then one quad per cell).
/// Presentation only; the tick and the checkpoints never see it.
fn subtitle_cells_quads(feed: &mut SubtitleFeed, tick: u64, cues: &[FightCue]) -> (usize, usize) {
    let notes: Vec<FightNote> = cues.iter().filter_map(|cue| fight_note(*cue)).collect();
    feed.push_cues(tick, &notes);
    let atlas = Atlas::new();
    let mut cells = 0usize;
    let mut quads = 0usize;
    for line in feed.lines() {
        let laid = layout(line.text, 32, Align::Left);
        let (_, _, indices) = emit_quads(&laid, atlas, 1.0);
        cells += laid.len();
        quads += indices.len() / 6;
    }
    (cells, quads)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire proof (subtitle captions): the built
    /// `engine_ui::access::SubtitleFeed` gets a live caller in
    /// [`super::super::run_windowed`]; one tick of fight cues becomes caption
    /// lines and laid-out HUD cells, and a slot outside the fighter range is
    /// dropped instead of panicking.
    #[test]
    fn subtitle_captions_track_the_live_cues() {
        let mut feed = caption_strip().expect("default cap is valid");
        let cues = [
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 8,
            },
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
        ];
        let (cells, quads) = push_caption_tick(Some(&mut feed), 42, &cues);
        assert_eq!(feed.lines().len(), 3, "one caption per cue under the cap");
        assert_eq!(feed.lines()[0].text, "Hit");
        assert_eq!(feed.lines()[2].text, "Knockdown");
        assert!(cells > 0, "captions lay out HUD cells");
        assert_eq!(quads, cells, "one quad per caption cell");
        // A slot past the participant range drops the note, never a panic.
        let (after, _) = push_caption_tick(
            Some(&mut feed),
            43,
            &[FightCue::StaminaOut { fighter: 200 }],
        );
        assert_eq!(after, cells, "a bad slot adds no line");
        // The refused-strip path counts nothing instead of panicking.
        assert_eq!(push_caption_tick(None, 44, &cues), (0, 0));
    }
}
