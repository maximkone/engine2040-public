//! preset: named mixer settings as data.
//!
//! [`MixPreset`] is one named set of bus gains, written as permille of each
//! bus's canon level (1000 = the crate's own gain, 0 = silent), so a shipped
//! game can carry "quiet menu", "loud fight" and "headphones" tables in a text
//! file instead of four constants in code. [`MixPreset::apply`] writes the
//! gains onto a [`BusMixer`], which is the same mixer [`BusMixer::render_frame`]
//! uses, so a preset changes real output.
//!
//! Owner: `engine-audio`. Consumer: whoever builds the mixer for a mode; until
//! a door names it the tests below apply a preset and measure the rendered
//! frame. Units: permille of canon gain, integers only. Determinism: a pure
//! parse and a pure multiply per bus, no clock and no IO; the same table and
//! the same canon gains give the same f32 gains. Failure: a refused table is a
//! typed [`PresetError`] naming the line, the preset or the bus. Budget: O(rows)
//! to parse, four multiplies to apply.
//!
//! Mixing is presentation only and never hashed, so these f32 gains stay out of
//! the golden.

use std::fmt;

use crate::dsp::{db_to_linear, Bus, BusMixer};

/// Most presets one table may hold.
pub const MAX_PRESETS: usize = 32;
/// Permille of the canon gain that equals the crate's own level.
pub const CANON_Q: u16 = 1000;
/// Highest permille a preset may ask for: +6 dB over canon.
pub const MAX_GAIN_Q: u16 = 2000;
/// Bus names in one preset, in the order the mixer sums them.
pub const BUS_NAMES: [&str; 4] = ["sfx", "music", "dialogue", "master"];

/// One named mixer setting: gains in permille, in [`BUS_NAMES`] order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MixPreset {
    name: String,
    gains: [u16; BUS_NAMES.len()],
}

/// A whole preset file, in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MixPresets {
    presets: Vec<MixPreset>,
}

/// Why a preset table was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// A row is not `preset <name>` or `<bus> <permille>`.
    BadRow {
        /// 1-based line of the row.
        line: usize,
        /// What the row should have been.
        reason: &'static str,
    },
    /// A bus row appears before any `preset` row.
    BeforePreset {
        /// 1-based line of the row.
        line: usize,
    },
    /// The same preset name is opened twice.
    DuplicatePreset {
        /// 1-based line of the second row.
        line: usize,
        /// The name listed twice.
        name: String,
    },
    /// A row names a bus the mixer does not sum.
    UnknownBus {
        /// 1-based line of the row.
        line: usize,
        /// The bus as written.
        bus: String,
    },
    /// One preset names the same bus twice.
    BusTwice {
        /// 1-based line of the second row.
        line: usize,
        /// The bus listed twice.
        bus: &'static str,
    },
    /// A preset ends (or the file ends) without one of the four buses.
    MissingBus {
        /// The preset that is short.
        name: String,
        /// The bus it never named.
        bus: &'static str,
    },
    /// A gain is not a whole number.
    BadGain {
        /// 1-based line of the row.
        line: usize,
        /// The text that did not parse.
        value: String,
    },
    /// A gain past [`MAX_GAIN_Q`].
    GainTooHigh {
        /// 1-based line of the row.
        line: usize,
        /// The permille asked for.
        permille: u16,
    },
    /// A file with no preset in it.
    NoPresets,
    /// More presets than [`MAX_PRESETS`].
    TooMany {
        /// How many presets the file holds.
        count: usize,
    },
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRow { line, reason } => write!(f, "line {line}: {reason}"),
            Self::BeforePreset { line } => write!(
                f,
                "line {line}: a bus row needs a preset above it; write `preset <name>` first"
            ),
            Self::DuplicatePreset { line, name } => {
                write!(f, "line {line}: preset `{name}` is opened twice")
            }
            Self::UnknownBus { line, bus } => write!(
                f,
                "line {line}: `{bus}` is not a bus; name one of {}",
                BUS_NAMES.join(", ")
            ),
            Self::BusTwice { line, bus } => {
                write!(f, "line {line}: bus `{bus}` is set twice in one preset")
            }
            Self::MissingBus { name, bus } => write!(
                f,
                "preset `{name}` never sets `{bus}`; every preset sets all four buses"
            ),
            Self::BadGain { line, value } => {
                write!(f, "line {line}: `{value}` is not a whole permille")
            }
            Self::GainTooHigh { line, permille } => {
                write!(f, "line {line}: {permille} permille; at most {MAX_GAIN_Q}")
            }
            Self::NoPresets => write!(f, "no preset row: a table needs at least one"),
            Self::TooMany { count } => write!(f, "{count} presets; at most {MAX_PRESETS}"),
        }
    }
}

impl std::error::Error for PresetError {}

/// The slot of a bus row, or `None` when the text names no such bus.
fn bus_slot(bus: &str) -> Option<usize> {
    BUS_NAMES.iter().position(|known| *known == bus)
}

/// A canon gain scaled by permille, so 1000 keeps the canon level exactly.
fn scaled(canon: f32, permille: u16) -> f32 {
    canon * f32::from(permille) / f32::from(CANON_Q)
}

/// A preset being read: the name plus the buses it has set so far.
struct Open {
    name: String,
    gains: [Option<u16>; BUS_NAMES.len()],
}

impl Open {
    /// Close the preset, refusing a missing bus by name.
    fn close(self) -> Result<MixPreset, PresetError> {
        let mut gains = [CANON_Q; BUS_NAMES.len()];
        for (slot, value) in self.gains.iter().enumerate() {
            let Some(permille) = value else {
                return Err(PresetError::MissingBus {
                    name: self.name,
                    bus: BUS_NAMES[slot],
                });
            };
            gains[slot] = *permille;
        }
        Ok(MixPreset {
            name: self.name,
            gains,
        })
    }
}

impl MixPreset {
    /// Preset name, as its `preset` row wrote it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gains in permille, in [`BUS_NAMES`] order.
    #[must_use]
    pub fn gains(&self) -> [u16; BUS_NAMES.len()] {
        self.gains
    }

    /// Write this preset's gains onto a mixer: each bus becomes its canon gain
    /// scaled by the preset's permille. The sample rate is left alone.
    pub fn apply(&self, mixer: &mut BusMixer) {
        mixer.g_sfx = scaled(Bus::Sfx.gain_linear(), self.gains[0]);
        mixer.g_music = scaled(Bus::Music.gain_linear(), self.gains[1]);
        mixer.g_dlg = scaled(Bus::Dialogue.gain_linear(), self.gains[2]);
        mixer.g_master = scaled(db_to_linear(crate::GAIN_MASTER_DB), self.gains[3]);
    }
}

impl MixPresets {
    /// Parse a preset table: a `preset <name>` row opens a block, and the four
    /// `<bus> <permille>` rows fill it. `#` starts a comment.
    pub fn parse(text: &str) -> Result<Self, PresetError> {
        let mut presets: Vec<MixPreset> = Vec::new();
        let mut open: Option<Open> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.split('#').next().unwrap_or(raw).trim();
            if body.is_empty() {
                continue;
            }
            let mut words = body.split_whitespace();
            let Some(key) = words.next() else {
                continue;
            };
            if key == "preset" {
                if let Some(previous) = open.take() {
                    presets.push(previous.close()?);
                }
                let Some(name) = words.next() else {
                    return Err(PresetError::BadRow {
                        line,
                        reason: "preset needs a name",
                    });
                };
                if words.next().is_some() {
                    return Err(PresetError::BadRow {
                        line,
                        reason: "preset takes one name and nothing else",
                    });
                }
                if presets.iter().any(|preset| preset.name == name) {
                    return Err(PresetError::DuplicatePreset {
                        line,
                        name: name.to_owned(),
                    });
                }
                open = Some(Open {
                    name: name.to_owned(),
                    gains: [None; BUS_NAMES.len()],
                });
                continue;
            }
            let Some(current) = open.as_mut() else {
                return Err(PresetError::BeforePreset { line });
            };
            let Some(slot) = bus_slot(key) else {
                return Err(PresetError::UnknownBus {
                    line,
                    bus: key.to_owned(),
                });
            };
            let Some(value) = words.next() else {
                return Err(PresetError::BadRow {
                    line,
                    reason: "a bus row needs a permille",
                });
            };
            if words.next().is_some() {
                return Err(PresetError::BadRow {
                    line,
                    reason: "a bus row takes one permille and nothing else",
                });
            }
            let permille: u16 = value.parse().map_err(|_| PresetError::BadGain {
                line,
                value: value.to_owned(),
            })?;
            if permille > MAX_GAIN_Q {
                return Err(PresetError::GainTooHigh { line, permille });
            }
            if current.gains[slot].is_some() {
                return Err(PresetError::BusTwice {
                    line,
                    bus: BUS_NAMES[slot],
                });
            }
            current.gains[slot] = Some(permille);
        }
        if let Some(last) = open {
            presets.push(last.close()?);
        }
        if presets.is_empty() {
            return Err(PresetError::NoPresets);
        }
        if presets.len() > MAX_PRESETS {
            return Err(PresetError::TooMany {
                count: presets.len(),
            });
        }
        Ok(Self { presets })
    }

    /// The preset with this name, or `None`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&MixPreset> {
        self.presets.iter().find(|preset| preset.name == name)
    }

    /// Preset names in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.presets
            .iter()
            .map(|preset| preset.name.as_str())
            .collect()
    }

    /// Presets in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.presets.len()
    }

    /// True when the table holds no preset.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
# two mixes a game can switch between
preset quiet
sfx 500
music 0     # silence the music bed
dialogue 1000
master 1000

preset fight
sfx 1500
music 1200
dialogue 1200
master 900
";

    /// F2P: no preset table existed before this module.
    #[test]
    fn parse_reads_presets_in_declaration_order() {
        let table = MixPresets::parse(TABLE).expect("the table parses");
        assert_eq!(table.names(), ["quiet", "fight"]);
        assert_eq!(table.len(), 2);
        let quiet = table.get("quiet").expect("quiet is named");
        assert_eq!(quiet.gains(), [500, 0, 1000, 1000]);
        assert_eq!(
            table.get("fight").expect("fight").gains(),
            [1500, 1200, 1200, 900]
        );
        assert!(table.get("loud").is_none(), "an unlisted name is no preset");
        // A table is data: the same text builds the same table twice.
        assert_eq!(MixPresets::parse(TABLE), Ok(table));
    }

    #[test]
    fn apply_writes_the_mixer_and_a_quiet_preset_renders_quieter() {
        let table = MixPresets::parse(TABLE).expect("parses");
        let canon = BusMixer::new(48_000);
        // Canon: SFX at -3 dB lands near 0.708 of the input.
        let loud = canon.render_frame(1.0, 1.0, 1.0);
        let mut quiet = BusMixer::new(48_000);
        table.get("quiet").expect("quiet").apply(&mut quiet);
        let softer = quiet.render_frame(1.0, 1.0, 1.0);
        assert!(softer < loud, "quiet {softer} is below canon {loud}");
        assert!(
            (quiet.g_sfx - Bus::Sfx.gain_linear() * 0.5).abs() < 1e-6,
            "half of canon"
        );
        assert_eq!(quiet.g_music, 0.0, "music is muted");
        assert_eq!(quiet.g_dlg, Bus::Dialogue.gain_linear(), "canon dialogue");
        assert_eq!(quiet.sample_rate, 48_000, "the rate is not a preset");
        // Applying is idempotent and independent of the order of lookups.
        let mut again = BusMixer::new(48_000);
        table.get("fight").expect("fight").apply(&mut again);
        let mut twice = BusMixer::new(48_000);
        table.get("fight").expect("fight").apply(&mut twice);
        table.get("fight").expect("fight").apply(&mut twice);
        assert_eq!(again.g_sfx, twice.g_sfx);
        assert_eq!(again.g_master, twice.g_master);
        assert!(again.g_sfx > canon.g_sfx, "+50% over canon");
    }

    #[test]
    fn refusals_name_the_line_the_preset_or_the_bus() {
        let cases: [(&str, PresetError); 8] = [
            ("sfx 1000\n", PresetError::BeforePreset { line: 1 }),
            (
                "preset quiet\nsfx 1000\nmusic 1000\ndialogue 1000\nmaster 1000\npreset quiet\n",
                PresetError::DuplicatePreset {
                    line: 6,
                    name: "quiet".to_owned(),
                },
            ),
            (
                "preset quiet\nsfx 1000\nmusic 1000\ndialogue 1000\nmaster 1000\nbass 1000\n",
                PresetError::UnknownBus {
                    line: 6,
                    bus: "bass".to_owned(),
                },
            ),
            (
                "preset quiet\nsfx 1000\nsfx 900\nmusic 1000\ndialogue 1000\nmaster 1000\n",
                PresetError::BusTwice {
                    line: 3,
                    bus: "sfx",
                },
            ),
            (
                "preset quiet\nsfx 1000\nmusic 1000\ndialogue 1000\n",
                PresetError::MissingBus {
                    name: "quiet".to_owned(),
                    bus: "master",
                },
            ),
            (
                "preset quiet\nsfx loud\nmusic 1000\ndialogue 1000\nmaster 1000\n",
                PresetError::BadGain {
                    line: 2,
                    value: "loud".to_owned(),
                },
            ),
            (
                "preset quiet\nsfx 2500\nmusic 1000\ndialogue 1000\nmaster 1000\n",
                PresetError::GainTooHigh {
                    line: 2,
                    permille: 2500,
                },
            ),
            ("# only a comment\n", PresetError::NoPresets),
        ];
        for (text, expected) in cases {
            assert_eq!(MixPresets::parse(text), Err(expected), "{text}");
        }
        // A row with no permille is refused by the row's shape, not as a gain.
        assert_eq!(
            MixPresets::parse("preset quiet\nsfx\n"),
            Err(PresetError::BadRow {
                line: 2,
                reason: "a bus row needs a permille",
            })
        );
        assert_eq!(
            MixPresets::parse("preset\nsfx 1000\n"),
            Err(PresetError::BadRow {
                line: 1,
                reason: "preset needs a name",
            })
        );
    }
}
