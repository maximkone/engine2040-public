//! preset: look presets (gap A7) as data, JSON in and chain settings out.
//!
//! A look preset is the art-direction half of color: a name, exposure in
//! hundredths of a stop, the key-fill and fog shares in permille, a
//! per-channel grade lift in thousandths, an optional six-byte palette lock
//! and an optional `.cube` path. [`parse_json`] reads one object and
//! [`LookPreset::apply`] writes it onto a [`ChainConfig`]; the built-ins
//! [`DUSTY_NOON`], [`ARENA_DUSK`] and [`NIGHT_SIEGE`] are consts of the same
//! shape. Units are the integer ticks above, so a preset file is exact and
//! two parses compare equal. Determinism: integers in, plain arithmetic out,
//! no clock and no IO; the LUT path is only named here, never opened (the
//! loader reads it and calls `ChainConfig::set_lut`). Errors are typed
//! [`PresetError`], each naming the key, the legal range and one fix.
//!
//! Shape: the exposure and grade fields land on the chain today (exposure as
//! stops, grade as per-channel display lift). `key_fill_permille`,
//! `fog_permille` and `palette_lock` are look metadata: the key-fill, fog
//! and palette stages are not chain fields yet, so `apply` carries them in
//! the preset (and in its digest) and the stages read them when they land.
//! `apply` always clears the previous table and records the new path: a look
//! owns its LUT, so the loader must fill `cfg.lut` again for every preset.

use std::borrow::Cow;
use std::fmt;

use engine_core::Fnv64;

use super::{ChainConfig, ExposureConfig};

/// Lowest `exposure_ev_x100` a preset may carry (hundredths of a stop).
pub const PRESET_EV_X100_MIN: i16 = -800;
/// Highest `exposure_ev_x100` a preset may carry (hundredths of a stop).
pub const PRESET_EV_X100_MAX: i16 = 800;
/// Highest `key_fill_permille` and `fog_permille` a preset may carry.
pub const PRESET_PERMILLE_MAX: u16 = 1000;
/// Lowest per-channel `grade` lift in thousandths of display units.
pub const PRESET_GRADE_MIN: i16 = -1000;
/// Highest per-channel `grade` lift in thousandths of display units.
pub const PRESET_GRADE_MAX: i16 = 1000;
/// Longest accepted preset name in bytes.
pub const PRESET_NAME_MAX: usize = 64;
/// Every key a look object may carry; anything else is refused.
pub const PRESET_KEYS: [&str; 7] = [
    "name",
    "exposure_ev_x100",
    "key_fill_permille",
    "fog_permille",
    "grade",
    "palette_lock",
    "lut",
];

/// Why a look preset was refused. Every variant names the key, and
/// [`fmt::Display`] adds the legal range and one fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresetError {
    /// The text is not the accepted JSON shape (`detail` says which part).
    Syntax {
        /// 1-based line the reader stopped on.
        line: usize,
        /// What was wrong, in a few words.
        detail: &'static str,
    },
    /// A key that is not one of [`PRESET_KEYS`].
    UnknownKey {
        /// 1-based line of the key.
        line: usize,
        /// Key as written.
        key: String,
    },
    /// A required key the object did not carry.
    Missing {
        /// 1-based line where the object closed.
        line: usize,
        /// Key that was missing.
        key: &'static str,
    },
    /// A value of the wrong JSON shape for its key.
    Type {
        /// 1-based line of the value.
        line: usize,
        /// Key being read.
        key: &'static str,
        /// Shape the key needs.
        expected: &'static str,
    },
    /// A value outside its legal band. `line` is `None` for a built-in
    /// preset, which has no file line.
    Range {
        /// 1-based line of the value, or `None` for a built-in.
        line: Option<usize>,
        /// Key being read.
        key: &'static str,
        /// Lowest legal value.
        min: i64,
        /// Highest legal value.
        max: i64,
    },
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, detail } => write!(
                f,
                "preset line {line}: {detail}; write one JSON object with the look keys"
            ),
            Self::UnknownKey { line, key } => write!(
                f,
                "preset line {line}: key \"{key}\" is not a look field; use one of {}",
                PRESET_KEYS.join(", ")
            ),
            Self::Missing { line, key } => {
                write!(f, "preset line {line}: no \"{key}\"; write the key")
            }
            Self::Type {
                line,
                key,
                expected,
            } => write!(f, "preset line {line}: \"{key}\" must be {expected}"),
            Self::Range {
                line,
                key,
                min,
                max,
            } => match line {
                Some(line) => write!(
                    f,
                    "preset line {line}: \"{key}\" is outside {min}..={max}; write a value in that band"
                ),
                None => write!(
                    f,
                    "preset \"{key}\" is outside {min}..={max}; write a value in that band"
                ),
            },
        }
    }
}

impl std::error::Error for PresetError {}

/// One look preset: the art-direction numbers plus the optional LUT path.
/// Field order and units are fixed, so [`LookPreset::digest`] is stable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LookPreset {
    /// Look name (`Cow` so the built-ins stay consts and a parsed name is owned).
    pub name: Cow<'static, str>,
    /// Exposure in hundredths of a stop, [`PRESET_EV_X100_MIN`]`..=`[`PRESET_EV_X100_MAX`].
    pub exposure_ev_x100: i16,
    /// Key-to-fill share in permille, `0..=`[`PRESET_PERMILLE_MAX`].
    pub key_fill_permille: u16,
    /// Fog share in permille, `0..=`[`PRESET_PERMILLE_MAX`].
    pub fog_permille: u16,
    /// Per-channel display lift in thousandths, each inside
    /// [`PRESET_GRADE_MIN`]`..=`[`PRESET_GRADE_MAX`].
    pub grade: [i16; 3],
    /// Six palette-lock bytes (three stops, each a low and a high sample).
    pub palette_lock: Option<[u8; 6]>,
    /// `.cube` file the look names; the loader reads it, `apply` never does.
    pub lut: Option<String>,
}

/// Dusty noon: neutral exposure, warm highlights, light fog.
pub const DUSTY_NOON: LookPreset = LookPreset {
    name: Cow::Borrowed("dusty noon"),
    exposure_ev_x100: 0,
    key_fill_permille: 420,
    fog_permille: 120,
    grade: [40, 10, -30],
    palette_lock: Some([214, 190, 150, 132, 104, 92]),
    lut: None,
};

/// Arena dusk: a third of a stop down, cool shadows, heavier fog.
pub const ARENA_DUSK: LookPreset = LookPreset {
    name: Cow::Borrowed("arena dusk"),
    exposure_ev_x100: -35,
    key_fill_permille: 300,
    fog_permille: 260,
    grade: [-25, 0, 35],
    palette_lock: Some([168, 120, 96, 92, 104, 128]),
    lut: None,
};

/// Night siege: a fifth of a stop up, blue night key, thick fog.
pub const NIGHT_SIEGE: LookPreset = LookPreset {
    name: Cow::Borrowed("night siege"),
    exposure_ev_x100: 20,
    key_fill_permille: 520,
    fog_permille: 420,
    grade: [10, 0, 60],
    palette_lock: Some([120, 132, 160, 64, 72, 96]),
    lut: None,
};

impl LookPreset {
    /// Exposure in stops (`exposure_ev_x100 / 100`), the chain unit.
    pub fn exposure_ev(&self) -> f32 {
        f32_from_i16(self.exposure_ev_x100) / 100.0
    }

    /// Write this look onto `cfg`: exposure stops and per-channel grade lift
    /// from the preset, and the LUT slot reset to the preset's path with no
    /// table loaded (the loader fills it afterwards). A preset without a LUT
    /// path therefore drops the previous table instead of leaking it.
    ///
    /// # Errors
    ///
    /// [`PresetError::Range`] when a field left its band (only a hand-built
    /// preset can reach this; `parse_json` and the built-ins check first).
    pub fn apply(&self, cfg: &mut ChainConfig) -> Result<(), PresetError> {
        let mut chain = cfg.chain;
        let exposure = ExposureConfig {
            exposure_ev: self.exposure_ev(),
        };
        exposure.mult().map_err(|_| PresetError::Range {
            line: None,
            key: "exposure_ev_x100",
            min: i64::from(PRESET_EV_X100_MIN),
            max: i64::from(PRESET_EV_X100_MAX),
        })?;
        let mut grade = chain.grade;
        for (lift, thousandths) in grade.lift.iter_mut().zip(self.grade.iter()) {
            *lift = f32_from_i16(*thousandths) / 1000.0;
        }
        chain.exposure = exposure;
        chain.grade = grade;
        cfg.chain = chain;
        cfg.lut = None;
        cfg.lut_path.clone_from(&self.lut);
        Ok(())
    }

    /// Digest over every field in struct order: name, exposure, key fill,
    /// fog, the three grade ticks, the palette lock (presence byte first) and
    /// the LUT path (presence byte first). Two parses of one file digest
    /// alike; any changed field moves it.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_len32(self.name.len());
        hasher.write_str(&self.name);
        hasher.write_i32(i32::from(self.exposure_ev_x100));
        hasher.write_u16(self.key_fill_permille);
        hasher.write_u16(self.fog_permille);
        for tick in self.grade {
            hasher.write_i32(i32::from(tick));
        }
        match &self.palette_lock {
            Some(palette) => {
                hasher.write_bool(true);
                for byte in palette {
                    hasher.write_u8(*byte);
                }
            }
            None => hasher.write_bool(false),
        }
        match &self.lut {
            Some(path) => {
                hasher.write_bool(true);
                hasher.write_len32(path.len());
                hasher.write_str(path);
            }
            None => hasher.write_bool(false),
        }
        hasher.finish()
    }
}

/// One `i16` tick as `f32`, without a float cast (`unsigned_abs` fits the
/// exact integer range of `f32`).
fn f32_from_i16(tick: i16) -> f32 {
    let magnitude = f32::from(tick.unsigned_abs());
    if tick < 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// Read one look object. Accepts the seven [`PRESET_KEYS`] once each, whole
/// numbers only, and nothing else; unknown keys are refused, never ignored.
///
/// # Errors
///
/// [`PresetError`] with the line: syntax, unknown key, missing key, wrong
/// JSON shape, or a value outside its band.
pub fn parse_json(text: &str) -> Result<LookPreset, PresetError> {
    let mut reader = JsonReader::new(text);
    reader.skip();
    reader.take(b'{', "a look is one JSON object")?;
    let mut digger = Digger::default();
    loop {
        reader.skip();
        if reader.peek() == Some(b'}') {
            reader.bump();
            break;
        }
        let key_line = reader.line;
        let key = reader.string()?;
        reader.skip();
        reader.take(b':', "a key is followed by a colon")?;
        reader.skip();
        let value_line = reader.line;
        if digger.taken(&key) {
            return Err(PresetError::Syntax {
                line: key_line,
                detail: "a key written twice",
            });
        }
        digger.read(&mut reader, &key, value_line)?;
        reader.skip();
        match reader.peek() {
            Some(b',') => {
                reader.bump();
            }
            Some(b'}') => {}
            _ => {
                return Err(PresetError::Syntax {
                    line: reader.line,
                    detail: "a key/value pair is followed by a comma or a brace",
                })
            }
        }
    }
    let close_line = reader.line;
    reader.skip();
    if reader.peek().is_some() {
        return Err(PresetError::Syntax {
            line: reader.line,
            detail: "text after the look object",
        });
    }
    digger.finish(close_line)
}

/// Field slots while reading: `None` until the key appears.
#[derive(Default)]
struct Digger {
    name: Option<Cow<'static, str>>,
    exposure_ev_x100: Option<i16>,
    key_fill_permille: Option<u16>,
    fog_permille: Option<u16>,
    grade: Option<[i16; 3]>,
    palette_lock: Option<[u8; 6]>,
    palette_lock_seen: bool,
    lut: Option<String>,
    lut_seen: bool,
}

impl Digger {
    /// Whether the key already has a slot filled (a duplicate).
    fn taken(&self, key: &str) -> bool {
        match key {
            "name" => self.name.is_some(),
            "exposure_ev_x100" => self.exposure_ev_x100.is_some(),
            "key_fill_permille" => self.key_fill_permille.is_some(),
            "fog_permille" => self.fog_permille.is_some(),
            "grade" => self.grade.is_some(),
            "palette_lock" => self.palette_lock_seen,
            "lut" => self.lut_seen,
            _ => false,
        }
    }

    /// Read one value into its slot.
    fn read(
        &mut self,
        reader: &mut JsonReader<'_>,
        key: &str,
        line: usize,
    ) -> Result<(), PresetError> {
        match key {
            "name" => {
                let text = reader.string()?;
                if text.is_empty() || text.len() > PRESET_NAME_MAX {
                    return Err(PresetError::Range {
                        line: Some(line),
                        key: "name",
                        min: 1,
                        max: i64::try_from(PRESET_NAME_MAX).unwrap_or(i64::MAX),
                    });
                }
                self.name = Some(Cow::Owned(text));
            }
            "exposure_ev_x100" => {
                let tick = tick(
                    reader,
                    "exposure_ev_x100",
                    PRESET_EV_X100_MIN,
                    PRESET_EV_X100_MAX,
                )?;
                self.exposure_ev_x100 = Some(tick);
            }
            "key_fill_permille" => {
                let v = whole(
                    reader,
                    "key_fill_permille",
                    0,
                    i64::from(PRESET_PERMILLE_MAX),
                )?;
                self.key_fill_permille = Some(u16::try_from(v).map_err(|_| {
                    out_of_band("key_fill_permille", 0, i64::from(PRESET_PERMILLE_MAX), line)
                })?);
            }
            "fog_permille" => {
                let v = whole(reader, "fog_permille", 0, i64::from(PRESET_PERMILLE_MAX))?;
                self.fog_permille = Some(u16::try_from(v).map_err(|_| {
                    out_of_band("fog_permille", 0, i64::from(PRESET_PERMILLE_MAX), line)
                })?);
            }
            "grade" => {
                let ticks = reader.ints(
                    "grade",
                    3,
                    i64::from(PRESET_GRADE_MIN),
                    i64::from(PRESET_GRADE_MAX),
                )?;
                let mut grade = [0i16; 3];
                for (slot, v) in grade.iter_mut().zip(ticks.iter()) {
                    *slot = i16::try_from(*v).map_err(|_| {
                        out_of_band(
                            "grade",
                            i64::from(PRESET_GRADE_MIN),
                            i64::from(PRESET_GRADE_MAX),
                            line,
                        )
                    })?;
                }
                self.grade = Some(grade);
            }
            "palette_lock" => {
                self.palette_lock_seen = true;
                if reader.peek() == Some(b'n') {
                    reader.word("null")?;
                    self.palette_lock = None;
                } else {
                    let bytes = reader.ints("palette_lock", 6, 0, 255)?;
                    let mut palette = [0u8; 6];
                    for (slot, v) in palette.iter_mut().zip(bytes.iter()) {
                        *slot = u8::try_from(*v)
                            .map_err(|_| out_of_band("palette_lock", 0, 255, line))?;
                    }
                    self.palette_lock = Some(palette);
                }
            }
            "lut" => {
                self.lut_seen = true;
                if reader.peek() == Some(b'n') {
                    reader.word("null")?;
                    self.lut = None;
                } else {
                    self.lut = Some(reader.string()?);
                }
            }
            other => {
                return Err(PresetError::UnknownKey {
                    line,
                    key: other.to_string(),
                })
            }
        }
        Ok(())
    }

    /// Every required slot present: build the preset.
    fn finish(self, line: usize) -> Result<LookPreset, PresetError> {
        Ok(LookPreset {
            name: self
                .name
                .ok_or(PresetError::Missing { line, key: "name" })?,
            exposure_ev_x100: self.exposure_ev_x100.ok_or(PresetError::Missing {
                line,
                key: "exposure_ev_x100",
            })?,
            key_fill_permille: self.key_fill_permille.ok_or(PresetError::Missing {
                line,
                key: "key_fill_permille",
            })?,
            fog_permille: self.fog_permille.ok_or(PresetError::Missing {
                line,
                key: "fog_permille",
            })?,
            grade: self
                .grade
                .ok_or(PresetError::Missing { line, key: "grade" })?,
            palette_lock: self.palette_lock,
            lut: self.lut,
        })
    }
}

/// The range error for one key.
fn out_of_band(key: &'static str, min: i64, max: i64, line: usize) -> PresetError {
    PresetError::Range {
        line: Some(line),
        key,
        min,
        max,
    }
}

/// One whole number in `min..=max` as an `i16` tick.
fn tick(
    reader: &mut JsonReader<'_>,
    key: &'static str,
    min: i16,
    max: i16,
) -> Result<i16, PresetError> {
    let value = whole(reader, key, i64::from(min), i64::from(max))?;
    i16::try_from(value).map_err(|_| out_of_band(key, i64::from(min), i64::from(max), reader.line))
}

/// One whole number in `min..=max`.
fn whole(
    reader: &mut JsonReader<'_>,
    key: &'static str,
    min: i64,
    max: i64,
) -> Result<i64, PresetError> {
    let line = reader.line;
    if reader.peek() == Some(b'[') || reader.peek() == Some(b'"') {
        return Err(PresetError::Type {
            line,
            key,
            expected: "a whole number",
        });
    }
    let value = reader.integer()?;
    if value < min || value > max {
        return Err(out_of_band(key, min, max, line));
    }
    Ok(value)
}

/// The reader over the text: byte cursor plus the current 1-based line.
struct JsonReader<'a> {
    bytes: &'a [u8],
    at: usize,
    line: usize,
}

impl<'a> JsonReader<'a> {
    /// Reader at the first byte of line 1.
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            at: 0,
            line: 1,
        }
    }

    /// Next byte without consuming it.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// Consume one byte, counting lines.
    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.at += 1;
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    /// Skip spaces, tabs and newlines.
    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.bump();
        }
    }

    /// Consume `want`, or refuse with `detail`.
    fn take(&mut self, want: u8, detail: &'static str) -> Result<(), PresetError> {
        if self.peek() == Some(want) {
            self.bump();
            Ok(())
        } else {
            Err(PresetError::Syntax {
                line: self.line,
                detail,
            })
        }
    }

    /// Consume a bare word (`null`).
    fn word(&mut self, want: &str) -> Result<(), PresetError> {
        for byte in want.as_bytes() {
            self.take(*byte, "the look value is not the accepted shape")?;
        }
        Ok(())
    }

    /// One quoted string with the common escapes.
    fn string(&mut self) -> Result<String, PresetError> {
        self.skip();
        if self.peek() != Some(b'"') {
            return Err(PresetError::Syntax {
                line: self.line,
                detail: "a string value is quoted",
            });
        }
        self.bump();
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.bump() {
                None => {
                    return Err(PresetError::Syntax {
                        line: self.line,
                        detail: "the string never closes",
                    })
                }
                Some(b'"') => {
                    return String::from_utf8(out).map_err(|_| PresetError::Syntax {
                        line: self.line,
                        detail: "the string is not UTF-8",
                    })
                }
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push(b'"'),
                    Some(b'\\') => out.push(b'\\'),
                    Some(b'/') => out.push(b'/'),
                    Some(b'n') => out.push(b'\n'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'r') => out.push(b'\r'),
                    _ => {
                        return Err(PresetError::Syntax {
                            line: self.line,
                            detail: "an escape this reader does not take",
                        })
                    }
                },
                Some(byte) => out.push(byte),
            }
        }
    }

    /// One whole number, sign included; fractions and exponents refused.
    fn integer(&mut self) -> Result<i64, PresetError> {
        self.skip();
        let line = self.line;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.bump();
        }
        let mut value = 0i64;
        let mut digits = 0usize;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            self.bump();
            let digit = i64::from(byte - b'0');
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(digit))
                .ok_or(PresetError::Syntax {
                    line,
                    detail: "the number is too large for a look tick",
                })?;
            digits += 1;
        }
        if digits == 0 {
            return Err(PresetError::Syntax {
                line,
                detail: "a tick is a whole number",
            });
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(PresetError::Syntax {
                line,
                detail: "look ticks are whole numbers",
            });
        }
        Ok(if negative { -value } else { value })
    }

    /// Exactly `count` whole numbers in `min..=max`, as a JSON array.
    fn ints(
        &mut self,
        key: &'static str,
        count: usize,
        min: i64,
        max: i64,
    ) -> Result<Vec<i64>, PresetError> {
        let line = self.line;
        if self.peek() != Some(b'[') {
            return Err(PresetError::Type {
                line,
                key,
                expected: "an array of whole numbers",
            });
        }
        self.bump();
        let mut values = Vec::with_capacity(count);
        for slot in 0..count {
            self.skip();
            if slot > 0 {
                self.take(b',', "array items are comma separated")?;
                self.skip();
            }
            let value_line = self.line;
            let value = self.integer()?;
            if value < min || value > max {
                return Err(out_of_band(key, min, max, value_line));
            }
            values.push(value);
        }
        self.skip();
        if self.peek() == Some(b',') {
            return Err(PresetError::Type {
                line: self.line,
                key,
                expected: "exactly its tick count (3 for grade, 6 for palette_lock)",
            });
        }
        self.take(b']', "close the tick array")?;
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete look object: one key per line, so line numbers are cheap
    /// to name in the failure tests.
    const DUSK_JSON: &str = r#"{
  "name": "dusk probe",
  "exposure_ev_x100": -35,
  "key_fill_permille": 300,
  "fog_permille": 260,
  "grade": [-25, 0, 35],
  "palette_lock": [168, 120, 96, 92, 104, 128],
  "lut": "content/looks/dusk.cube"
}"#;

    /// Two parses of one file are equal and digest alike.
    #[test]
    fn preset_two_parses_are_equal() {
        let first = parse_json(DUSK_JSON).expect("dusk parses");
        let second = parse_json(DUSK_JSON).expect("dusk parses again");
        assert_eq!(first, second, "two parses disagree");
        assert_eq!(first.digest(), second.digest(), "two parses digest apart");
        assert_eq!(first.name, "dusk probe", "the name reads back");
        assert_eq!(first.exposure_ev_x100, -35, "exposure ticks read back");
        assert_eq!(first.key_fill_permille, 300, "key fill reads back");
        assert_eq!(first.fog_permille, 260, "fog reads back");
        assert_eq!(first.grade, [-25, 0, 35], "grade ticks read back");
        assert_eq!(
            first.palette_lock,
            Some([168, 120, 96, 92, 104, 128]),
            "the palette lock reads back"
        );
        assert_eq!(
            first.lut.as_deref(),
            Some("content/looks/dusk.cube"),
            "the LUT path reads back"
        );
        assert!(
            (first.exposure_ev() + 0.35).abs() <= 1e-6,
            "exposure stops: {}",
            first.exposure_ev()
        );
    }

    /// An unknown key is refused by name, never ignored.
    #[test]
    fn preset_unknown_key_is_refused() {
        let text = r#"{
  "name": "probe",
  "hue_shift": 3
}"#;
        match parse_json(text) {
            Err(ref error @ PresetError::UnknownKey { line, ref key }) => {
                assert_eq!(line, 3, "the stray key sits on line 3");
                assert_eq!(key.as_str(), "hue_shift", "the message names the key");
                let shown = error.to_string();
                assert!(
                    shown.contains("hue_shift") && shown.contains("exposure_ev_x100"),
                    "the message names the key and the legal keys: {shown}"
                );
            }
            other => panic!("an unknown key gives {other:?}, not a refusal"),
        }
    }

    /// A field outside its band names the key, the band and one fix.
    #[test]
    fn preset_bad_field_names_the_key_and_range() {
        let text = r#"{
  "name": "probe",
  "exposure_ev_x100": 900,
  "key_fill_permille": 300,
  "fog_permille": 200,
  "grade": [0, 0, 0]
}"#;
        match parse_json(text) {
            Err(
                error @ PresetError::Range {
                    line,
                    key,
                    min,
                    max,
                },
            ) => {
                assert_eq!(line, Some(3), "the bad value sits on line 3");
                assert_eq!(key, "exposure_ev_x100", "the message names the key");
                assert_eq!((min, max), (-800, 800), "the message names the band");
                let shown = error.to_string();
                assert!(
                    shown.contains("exposure_ev_x100")
                        && shown.contains("-800..=800")
                        && shown.contains("write a value"),
                    "the message carries key, band and fix: {shown}"
                );
            }
            other => panic!("an out-of-band value gives {other:?}, not a range error"),
        }
    }

    /// A missing required key is refused at the closing brace.
    #[test]
    fn preset_missing_key_is_refused() {
        let text = r#"{
  "name": "probe",
  "exposure_ev_x100": 0,
  "key_fill_permille": 300,
  "fog_permille": 200
}"#;
        match parse_json(text) {
            Err(PresetError::Missing { line, key }) => {
                assert_eq!((line, key), (6, "grade"), "grade is the missing key");
            }
            other => panic!("a short object gives {other:?}, not a missing key"),
        }
    }

    /// Applying a look then undoing to the saved config restores the exact
    /// config digest, and a preset fully overwrites the one before it.
    #[test]
    fn preset_apply_then_undo_restores_the_digest() {
        let mut cfg = ChainConfig::default();
        let previous = cfg.clone();
        let prior = cfg.digest();
        ARENA_DUSK.apply(&mut cfg).expect("dusk applies");
        let dusk = cfg.digest();
        assert_ne!(dusk, prior, "dusk must move the config");
        DUSTY_NOON.apply(&mut cfg).expect("noon applies");
        let noon = cfg.digest();
        assert_ne!(noon, dusk, "noon must move the config again");
        cfg = previous;
        assert_eq!(
            cfg.digest(),
            prior,
            "restoring the saved config restores its digest"
        );
        let mut fresh = ChainConfig::default();
        DUSTY_NOON.apply(&mut fresh).expect("noon applies fresh");
        assert_eq!(
            fresh.digest(),
            noon,
            "a preset overwrites the whole look, so the order does not leak"
        );
    }

    /// A preset that names a LUT file applies without reading it; the loader
    /// is the one that fails on a missing file.
    #[test]
    fn preset_missing_lut_file_does_not_fail_apply() {
        let preset = parse_json(DUSK_JSON).expect("dusk parses");
        let mut cfg = ChainConfig::default();
        preset
            .apply(&mut cfg)
            .expect("apply never opens the look file");
        assert_eq!(
            cfg.lut_path.as_deref(),
            Some("content/looks/dusk.cube"),
            "the path is handed to the loader"
        );
        assert!(cfg.lut.is_none(), "apply itself loads no table");
        DUSTY_NOON.apply(&mut cfg).expect("noon applies");
        assert!(
            cfg.lut_path.is_none(),
            "a look without a LUT drops the previous path"
        );
    }

    /// Exposure and grade land on the chain in their documented units.
    #[test]
    fn preset_apply_writes_exposure_and_grade() {
        let mut cfg = ChainConfig::default();
        ARENA_DUSK.apply(&mut cfg).expect("dusk applies");
        assert!(
            (cfg.chain.exposure.exposure_ev - (-0.35)).abs() <= 1e-6,
            "exposure stops: {}",
            cfg.chain.exposure.exposure_ev
        );
        let expected = [-0.025f32, 0.0, 0.035];
        for channel in 0..3 {
            assert!(
                (cfg.chain.grade.lift[channel] - expected[channel]).abs() <= 1e-6,
                "grade lift {channel} is {}, not {}",
                cfg.chain.grade.lift[channel],
                expected[channel]
            );
        }
        assert!(
            cfg.chain.validate().is_ok(),
            "the graded chain still validates"
        );
    }

    /// The three built-ins stay inside every band and digest apart.
    #[test]
    fn preset_built_ins_are_legal_and_distinct() {
        let mut digests = Vec::new();
        for preset in [&DUSTY_NOON, &ARENA_DUSK, &NIGHT_SIEGE] {
            assert!(!preset.name.is_empty(), "a built-in needs a name");
            assert!(
                preset.name.len() <= PRESET_NAME_MAX,
                "{} is too long",
                preset.name
            );
            assert!(
                (PRESET_EV_X100_MIN..=PRESET_EV_X100_MAX).contains(&preset.exposure_ev_x100),
                "{} exposure left the band",
                preset.name
            );
            assert!(preset.key_fill_permille <= PRESET_PERMILLE_MAX, "key fill");
            assert!(preset.fog_permille <= PRESET_PERMILLE_MAX, "fog");
            for tick in preset.grade {
                assert!(
                    (PRESET_GRADE_MIN..=PRESET_GRADE_MAX).contains(&tick),
                    "{} grade tick {tick} left the band",
                    preset.name
                );
            }
            assert!(preset.lut.is_none(), "built-ins name no LUT file");
            digests.push(preset.digest());
        }
        digests.sort_unstable();
        digests.dedup();
        assert_eq!(digests.len(), 3, "the built-ins digest apart");
    }
}
