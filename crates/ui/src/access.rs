//! access: accessibility pass for the HUD. A contrast gate over every HUD
//! palette, and caption lines from fight cues for players who cannot hear the
//! hits or the crowd.
//!
//! Consumer: the HUD renderer (caption strip) and the settings screen. Data
//! owner: `engine-ui`. Units: contrast is the `contrast_ratio` number checked
//! against [`MIN_CONTRAST`]; caption ticks are fixed steps; fighters are
//! [`ParticipantId`] slots. Determinism: pure functions over owned values; the
//! caption strip is oldest-first and bounded. Failure mode: a bad strip
//! capacity returns [`HudError`]; nothing panics. The gate is numbers only;
//! taste goes to the AI review panel.
//!
//! Fight cues: this crate has no edge to the combat or audio crates. The
//! caller maps its fight event into [`FightNote`] field for field, the same
//! pattern the audio crate uses for its own cue type.

use engine_input::ParticipantId;

use super::{contrast_ratio, HudError, Palette, MIN_CONTRAST};

/// The HUD palette set in fixed order: default, then the three colorblind-safe
/// sets. The index is the palette id a [`DuelOverlay`](super::DuelOverlay) takes.
#[must_use]
pub fn hud_palettes() -> [Palette; 4] {
    [
        Palette::default_palette(),
        Palette::deuteranopia(),
        Palette::protanopia(),
        Palette::tritanopia(),
    ]
}

/// Contrast numbers for one palette: the two pairs [`Palette::validate`] checks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PaletteContrast {
    /// Palette name.
    pub name: &'static str,
    /// Foreground over background.
    pub text_ratio: f32,
    /// Accent over background.
    pub accent_ratio: f32,
}

impl PaletteContrast {
    /// Measure one palette with [`contrast_ratio`]. Pure.
    #[must_use]
    pub fn measure(palette: &Palette) -> Self {
        Self {
            name: palette.name,
            text_ratio: contrast_ratio(palette.fg, palette.bg),
            accent_ratio: contrast_ratio(palette.accent, palette.bg),
        }
    }

    /// The lower of the two ratios.
    #[must_use]
    pub fn weakest(&self) -> f32 {
        self.text_ratio.min(self.accent_ratio)
    }

    /// True when both pairs reach [`MIN_CONTRAST`].
    #[must_use]
    pub fn passes(&self) -> bool {
        self.weakest() >= MIN_CONTRAST
    }
}

/// Contrast numbers for every HUD palette, in [`hud_palettes`] order. Pure.
#[must_use]
pub fn contrast_table() -> [PaletteContrast; 4] {
    hud_palettes().map(|palette| PaletteContrast::measure(&palette))
}

/// One fight cue as the caller maps it. Same shape as the combat fight events,
/// without the damage amounts, which the audio layer keeps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FightNote {
    /// Full damage landed.
    Hit {
        /// Attacker slot.
        attacker: ParticipantId,
        /// Victim slot.
        victim: ParticipantId,
    },
    /// Held block soaked the hit.
    Blocked {
        /// Attacker slot.
        attacker: ParticipantId,
        /// Victim slot.
        victim: ParticipantId,
    },
    /// Parry window: the attacker is staggered.
    Parried {
        /// Attacker slot (staggered).
        attacker: ParticipantId,
        /// Victim slot (parried).
        victim: ParticipantId,
    },
    /// Dodge i-frames negated the hit.
    Dodged {
        /// Attacker slot.
        attacker: ParticipantId,
        /// Victim slot (dodged).
        victim: ParticipantId,
    },
    /// Attack or dodge pressed without stamina.
    StaminaOut {
        /// Fighter slot.
        fighter: ParticipantId,
    },
    /// The victim dropped to zero health.
    Knockdown {
        /// Attacker slot.
        attacker: ParticipantId,
        /// Victim slot (now at zero health).
        victim: ParticipantId,
    },
}

/// One caption line: the words, the tick they start on, and the fighter the
/// line is about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Subtitle {
    /// Fixed tick the cue landed on.
    pub tick: u64,
    /// Short caption text, shown as written.
    pub text: &'static str,
    /// Fighter the line is about: the one hit, the one staggered, or the one
    /// out of stamina.
    pub subject: ParticipantId,
}

impl Subtitle {
    /// Caption for one cue at `tick`. Pure.
    #[must_use]
    pub fn from_note(tick: u64, note: FightNote) -> Self {
        let (text, subject) = match note {
            FightNote::Hit { victim, .. } => ("Hit", victim),
            FightNote::Blocked { victim, .. } => ("Blocked", victim),
            FightNote::Parried { attacker, .. } => ("Parried", attacker),
            FightNote::Dodged { victim, .. } => ("Dodged", victim),
            FightNote::StaminaOut { fighter } => ("No stamina", fighter),
            FightNote::Knockdown { victim, .. } => ("Knockdown", victim),
        };
        Self {
            tick,
            text,
            subject,
        }
    }
}

/// Most caption lines the strip holds at once.
pub const MAX_SUBTITLE_LINES: usize = 4;
/// Caption lines the strip holds by default.
pub const DEFAULT_SUBTITLE_LINES: usize = 3;

/// Bounded caption strip, oldest-first. A full strip drops its oldest line,
/// the same rule as the kill feed, and counts every line it drops so a hit
/// storm shows as a number, never as unbounded work or a growing queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleFeed {
    lines: Vec<Subtitle>,
    cap: usize,
    dropped: u64,
}

impl SubtitleFeed {
    /// Empty strip holding at most `cap` lines (`1..=MAX_SUBTITLE_LINES`).
    ///
    /// # Errors
    /// [`HudError::FeedCapOutOfRange`] outside that range.
    pub fn new(cap: usize) -> Result<Self, HudError> {
        if cap == 0 || cap > MAX_SUBTITLE_LINES {
            return Err(HudError::FeedCapOutOfRange(cap));
        }
        Ok(Self {
            lines: Vec::new(),
            cap,
            dropped: 0,
        })
    }

    /// Cues that never reached the screen or were pushed off it, since the
    /// strip was made. Shown as a "+N" count by the renderer. Pure.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Capacity in lines.
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> &[Subtitle] {
        &self.lines
    }

    /// Add the captions for one tick's cues, in cue order. Only the last
    /// `cap` cues of a tick can survive the strip, so the rest are counted
    /// without building a line: the cost is `O(cap)` however big the storm.
    /// Pure.
    pub fn push_cues(&mut self, tick: u64, notes: &[FightNote]) {
        let skipped = notes.len().saturating_sub(self.cap);
        self.dropped = self
            .dropped
            .saturating_add(u64::try_from(skipped).unwrap_or(u64::MAX));
        for &note in &notes[skipped..] {
            if self.lines.len() >= self.cap {
                // `cap >= 1` and `len >= cap`, so the vec is non-empty.
                self.lines.remove(0);
                self.dropped = self.dropped.saturating_add(1);
            }
            self.lines.push(Subtitle::from_note(tick, note));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DuelOverlay, MAX_PARTICIPANTS};

    fn fighter(slot: u8) -> ParticipantId {
        ParticipantId::new(slot).expect("test slot is valid")
    }

    /// Mid-grey text on mid-grey: fails the gate on both pairs.
    fn weak() -> Palette {
        Palette {
            name: "weak",
            bg: [120, 120, 120],
            fg: [125, 125, 125],
            accent: [255, 255, 255],
        }
    }

    #[test]
    fn every_hud_palette_passes_the_contrast_gate() {
        for row in contrast_table() {
            assert!(
                row.passes(),
                "{} weakest {:.2} is below {:.1}",
                row.name,
                row.weakest(),
                MIN_CONTRAST
            );
            assert!(row.text_ratio >= MIN_CONTRAST, "{} text", row.name);
            assert!(row.accent_ratio >= MIN_CONTRAST, "{} accent", row.name);
        }
        for palette in hud_palettes() {
            assert!(
                palette.validate().is_ok(),
                "palette {} fails Palette::validate",
                palette.name
            );
        }
    }

    #[test]
    fn the_gate_numbers_match_the_contrast_formula() {
        // Black against white is the top of the scale: (1 + 0.05) / 0.05.
        let top = contrast_ratio([255, 255, 255], [0, 0, 0]);
        assert!((top - 21.0).abs() < 1e-3, "black on white is 21, got {top}");
        let row = PaletteContrast::measure(&Palette::deuteranopia());
        assert!(
            row.text_ratio > 20.0,
            "white text on black: {}",
            row.text_ratio
        );
    }

    #[test]
    fn a_weak_palette_fails_the_gate_with_its_number() {
        let row = PaletteContrast::measure(&weak());
        assert!(!row.passes());
        assert!(row.weakest() < MIN_CONTRAST);
        assert!(weak().validate().is_err());
    }

    #[test]
    fn overlay_palette_ids_are_the_gated_set() {
        let mut overlay = DuelOverlay::new(0);
        for (id, palette) in hud_palettes().iter().enumerate() {
            let id = u8::try_from(id).expect("four palettes fit u8");
            overlay.set_palette(id).expect("palette id selects");
            assert_eq!(overlay.palette(), *palette, "palette id {id}");
        }
    }

    #[test]
    fn every_fight_cue_gets_a_short_caption_on_the_right_fighter() {
        let (a, v) = (fighter(0), fighter(1));
        let cases = [
            (
                FightNote::Hit {
                    attacker: a,
                    victim: v,
                },
                "Hit",
                v,
            ),
            (
                FightNote::Blocked {
                    attacker: a,
                    victim: v,
                },
                "Blocked",
                v,
            ),
            (
                FightNote::Parried {
                    attacker: a,
                    victim: v,
                },
                "Parried",
                a,
            ),
            (
                FightNote::Dodged {
                    attacker: a,
                    victim: v,
                },
                "Dodged",
                v,
            ),
            (FightNote::StaminaOut { fighter: a }, "No stamina", a),
            (
                FightNote::Knockdown {
                    attacker: a,
                    victim: v,
                },
                "Knockdown",
                v,
            ),
        ];
        for (note, text, subject) in cases {
            let line = Subtitle::from_note(42, note);
            assert_eq!(
                (line.tick, line.text, line.subject),
                (42, text, subject),
                "{note:?}"
            );
            assert!(
                !line.text.is_empty() && line.text.len() <= 12,
                "short caption: {}",
                line.text
            );
        }
    }

    #[test]
    fn the_caption_strip_is_bounded_oldest_first_and_refuses_bad_caps() {
        assert_eq!(SubtitleFeed::new(0), Err(HudError::FeedCapOutOfRange(0)));
        assert_eq!(
            SubtitleFeed::new(MAX_SUBTITLE_LINES + 1),
            Err(HudError::FeedCapOutOfRange(MAX_SUBTITLE_LINES + 1))
        );
        let mut strip = SubtitleFeed::new(2).expect("cap 2 is valid");
        let hit = FightNote::Hit {
            attacker: fighter(0),
            victim: fighter(1),
        };
        let dodge = FightNote::Dodged {
            attacker: fighter(0),
            victim: fighter(1),
        };
        let knock = FightNote::Knockdown {
            attacker: fighter(0),
            victim: fighter(1),
        };
        strip.push_cues(5, &[hit, dodge, knock]);
        let texts: Vec<&str> = strip.lines().iter().map(|line| line.text).collect();
        assert_eq!(texts, vec!["Dodged", "Knockdown"]);
        assert_eq!(strip.lines()[0].tick, 5);
        assert_eq!(strip.cap(), 2);
    }

    /// Every ordered pair of the full roster lands a hit in one tick: the cap
    /// times one fewer (992 cues today).
    fn full_roster_storm() -> Vec<FightNote> {
        let mut storm = Vec::new();
        for attacker in 0..MAX_PARTICIPANTS {
            for victim in (0..MAX_PARTICIPANTS).filter(|&victim| victim != attacker) {
                storm.push(FightNote::Hit {
                    attacker: fighter(attacker),
                    victim: fighter(victim),
                });
            }
        }
        storm
    }

    #[test]
    fn a_full_roster_hit_storm_stays_bounded_and_counts_every_drop() {
        let storm = full_roster_storm();
        let cap = usize::from(MAX_PARTICIPANTS);
        assert_eq!(storm.len(), cap * (cap - 1));
        let mut strip = SubtitleFeed::new(DEFAULT_SUBTITLE_LINES).expect("default cap is valid");
        let mut pushed = 0u64;
        for tick in 0..50u64 {
            strip.push_cues(tick, &storm);
            pushed += storm.len() as u64;
            assert_eq!(strip.lines().len(), DEFAULT_SUBTITLE_LINES);
            // Nothing is lost silently: shown plus dropped is everything pushed.
            assert_eq!(strip.lines().len() as u64 + strip.dropped(), pushed);
        }
        // The survivors are the storm's last cues, stamped with the last tick.
        let tail = &storm[storm.len() - DEFAULT_SUBTITLE_LINES..];
        for (line, note) in strip.lines().iter().zip(tail) {
            assert_eq!(*line, Subtitle::from_note(49, *note));
        }
    }

    #[test]
    fn the_storm_shortcut_matches_pushing_one_cue_at_a_time() {
        let storm = full_roster_storm();
        let mut batched = SubtitleFeed::new(2).expect("cap 2 is valid");
        let mut single = batched.clone();
        batched.push_cues(7, &storm[..40]);
        for note in &storm[..40] {
            single.push_cues(7, std::slice::from_ref(note));
        }
        assert_eq!(batched, single);
        assert_eq!(batched.dropped(), 38);
    }

    #[test]
    fn a_quiet_strip_drops_nothing() {
        let mut strip = SubtitleFeed::new(MAX_SUBTITLE_LINES).expect("max cap is valid");
        strip.push_cues(1, &full_roster_storm()[..MAX_SUBTITLE_LINES]);
        assert_eq!(strip.dropped(), 0);
        assert_eq!(strip.lines().len(), MAX_SUBTITLE_LINES);
    }

    #[test]
    fn the_same_cues_give_the_same_strip_twice() {
        let cues = [
            FightNote::Parried {
                attacker: fighter(2),
                victim: fighter(3),
            },
            FightNote::StaminaOut {
                fighter: fighter(2),
            },
        ];
        let mut first = SubtitleFeed::new(DEFAULT_SUBTITLE_LINES).expect("default cap is valid");
        let mut second = first.clone();
        first.push_cues(9, &cues);
        second.push_cues(9, &cues);
        assert_eq!(first, second);
        assert_eq!(first.cap(), DEFAULT_SUBTITLE_LINES);
    }

    #[test]
    fn contrast_table_names_are_pinned() {
        let table = contrast_table();
        assert_eq!(table.len(), 4);
        let palettes = hud_palettes();
        let expected = ["default", "deuteranopia", "protanopia", "tritanopia"];
        for (index, row) in table.iter().enumerate() {
            assert_eq!(row.name, expected[index], "contrast row {index}");
            assert_eq!(
                row.name, palettes[index].name,
                "row {index} follows hud_palettes order"
            );
            assert!(
                row.passes(),
                "{} weakest {:.2} is below {:.1}",
                row.name,
                row.weakest(),
                MIN_CONTRAST
            );
            assert!(
                row.weakest() >= MIN_CONTRAST,
                "{} weakest below gate",
                row.name
            );
        }
    }

    #[test]
    fn overlay_ids_match_contrast_table_order() {
        let table = contrast_table();
        let palettes = hud_palettes();
        let mut overlay = DuelOverlay::new(0);
        for id in 0..=3u8 {
            overlay.set_palette(id).expect("palette id selects");
            let index = usize::from(id);
            assert_eq!(overlay.palette(), palettes[index], "overlay id {id}");
            assert_eq!(
                overlay.palette().name,
                table[index].name,
                "overlay id {id} matches contrast row"
            );
            assert_eq!(
                PaletteContrast::measure(&overlay.palette()),
                table[index],
                "overlay id {id} measures the contrast row"
            );
        }
    }
}
