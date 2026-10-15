//! Capture manifest reader for the look judge.
//!
//! Consumer: `super::judge`, which binds each frame to its manifest row and
//! grounds regions on the entities the row names. Data owner: the capture
//! path that wrote `manifest.json`; this module only reads.
//! Shapes read: an array of frame rows (the staged-fight writer) or an object
//! with a `frames` array (the capture-sink writer). Unknown keys are skipped.
//! Units: positions in millimetres (the writers print metres to three
//! decimals, parsed here without a float), ticks in fixed ticks, sizes in
//! pixels. Determinism: pure over the input text; the same text twice gives
//! the same rows or the same error. Errors: [`ManifestError`] carries the
//! byte offset of a syntax fault or the shape that was expected.
//! Budget: one pass; at most [`MAX_ROWS`] frame rows and [`MAX_FIGHTERS`]
//! fighters per row, nesting at most eight deep.

use std::fmt;

use crate::parse::{Cursor, ParseError};

/// Most frame rows read from one manifest.
pub const MAX_ROWS: usize = 4096;
/// Most fighters read from one frame row.
pub const MAX_FIGHTERS: usize = 64;
/// Deepest JSON nesting accepted.
const MAX_DEPTH: usize = 8;

/// Why a manifest did not read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ManifestError {
    /// The text is not JSON this reader accepts (offset inside).
    Syntax(ParseError),
    /// The JSON parsed but is not one of the two manifest shapes.
    Shape(&'static str),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "manifest is not readable JSON: {error:?}"),
            Self::Shape(want) => write!(f, "manifest shape refused: {want}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// One fighter the capture path says it drew.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManifestFigure {
    /// Fighter slot, the entity id regions are grounded on.
    pub slot: u32,
    /// Arena x in millimetres.
    pub x_mm: i64,
    /// Arena z in millimetres.
    pub z_mm: i64,
    /// Health in permille when the row carries it.
    pub hp: Option<i64>,
    /// True when the fighter flashed a hit on this frame.
    pub hit: bool,
}

/// One frame row.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ManifestFrame {
    /// Fixed tick the frame was taken at.
    pub tick: Option<u64>,
    /// File name the row binds to, when the writer names it.
    pub file: Option<String>,
    /// FNV-1a hex the writer filed over the PNG bytes.
    pub png_fnv: Option<String>,
    /// `(width, height)` the writer filed.
    pub size: Option<(u32, u32)>,
    /// Fighters the writer says it drew (producer-reported, never scored).
    pub fighters: Vec<ManifestFigure>,
}

/// A parsed manifest: its frame rows in file order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Manifest {
    /// Frame rows in file order.
    pub frames: Vec<ManifestFrame>,
}

/// JSON value with numbers held in thousandths so no float is ever read.
#[derive(Clone, PartialEq, Debug)]
enum Value {
    Null,
    Bool(bool),
    Milli(i64),
    Text(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    fn field(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn uint(&self) -> Option<u64> {
        match self {
            Self::Milli(m) if *m >= 0 && m % 1000 == 0 => u64::try_from(m / 1000).ok(),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Read a manifest.
///
/// # Errors
///
/// [`ManifestError::Syntax`] for text that is not JSON (floats with an
/// exponent included), [`ManifestError::Shape`] for JSON that is neither a
/// frame array nor an object with a `frames` array or that exceeds
/// [`MAX_ROWS`] or [`MAX_FIGHTERS`].
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let mut cursor = Cursor::new(text.trim_start_matches('\u{feff}'));
    cursor.ws();
    let root = read_value(&mut cursor, 0).map_err(ManifestError::Syntax)?;
    cursor.ws();
    cursor.finish().map_err(ManifestError::Syntax)?;
    let rows = match &root {
        Value::List(rows) => rows,
        Value::Map(_) => match root.field("frames") {
            Some(Value::List(rows)) => rows,
            _ => return Err(ManifestError::Shape("an object with a `frames` array")),
        },
        _ => return Err(ManifestError::Shape("an array of frame rows")),
    };
    if rows.len() > MAX_ROWS {
        return Err(ManifestError::Shape("at most 4096 frame rows"));
    }
    let frames = rows.iter().map(frame_row).collect::<Result<Vec<_>, _>>()?;
    Ok(Manifest { frames })
}

fn frame_row(row: &Value) -> Result<ManifestFrame, ManifestError> {
    if !matches!(row, Value::Map(_)) {
        return Err(ManifestError::Shape("each frame row is an object"));
    }
    let size = match row.field("size") {
        Some(Value::List(pair)) if pair.len() == 2 => pair[0].uint().zip(pair[1].uint()),
        _ => row
            .field("width")
            .and_then(Value::uint)
            .zip(row.field("height").and_then(Value::uint)),
    };
    let size = size.and_then(|(w, h)| Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?)));
    let fighters = match row.field("fighters") {
        Some(Value::List(list)) if list.len() > MAX_FIGHTERS => {
            return Err(ManifestError::Shape("at most 64 fighters per frame row"))
        }
        Some(Value::List(list)) => list.iter().filter_map(fighter).collect(),
        _ => Vec::new(),
    };
    Ok(ManifestFrame {
        tick: row.field("tick").and_then(Value::uint),
        file: row.field("file").and_then(Value::text).map(str::to_owned),
        png_fnv: row
            .field("png_fnv")
            .and_then(Value::text)
            .map(str::to_owned),
        size,
        fighters,
    })
}

fn fighter(entry: &Value) -> Option<ManifestFigure> {
    let milli = |key: &str| match entry.field(key) {
        Some(Value::Milli(m)) => Some(*m),
        _ => None,
    };
    Some(ManifestFigure {
        slot: u32::try_from(entry.field("slot")?.uint()?).ok()?,
        x_mm: milli("x")?,
        z_mm: milli("z")?,
        hp: milli("hp").map(|m| m / 1000),
        hit: matches!(entry.field("hit"), Some(Value::Bool(true))),
    })
}

fn read_value(cursor: &mut Cursor<'_>, depth: usize) -> Result<Value, ParseError> {
    if depth > MAX_DEPTH {
        return Err(ParseError::NestingTooDeep { pos: cursor.pos() });
    }
    match cursor.peek() {
        None => Err(ParseError::EndOfInput { pos: cursor.pos() }),
        Some(b'{') => read_map(cursor, depth),
        Some(b'[') => read_list(cursor, depth),
        Some(b'"') => read_text(cursor).map(Value::Text),
        Some(b't') => cursor.lit("true").map(|()| Value::Bool(true)),
        Some(b'f') => cursor.lit("false").map(|()| Value::Bool(false)),
        Some(b'n') => cursor.lit("null").map(|()| Value::Null),
        Some(b'-' | b'0'..=b'9') => read_milli(cursor),
        Some(byte) => Err(ParseError::UnexpectedByte {
            pos: cursor.pos(),
            byte,
        }),
    }
}

/// `-?digits(.digits)?` into thousandths; digits past the third decimal are
/// dropped, an exponent refuses.
fn read_milli(cursor: &mut Cursor<'_>) -> Result<Value, ParseError> {
    let start = cursor.pos();
    let negative = cursor.eat(b'-');
    let whole = digits(cursor);
    if whole.is_empty() {
        return Err(ParseError::BadNumber { pos: start });
    }
    let mut fraction = String::new();
    if cursor.eat(b'.') {
        fraction = digits(cursor);
        if fraction.is_empty() {
            return Err(ParseError::BadFraction { pos: start });
        }
    }
    if matches!(cursor.peek(), Some(b'e' | b'E')) {
        return Err(ParseError::FloatRefused { pos: start });
    }
    let padded: String = fraction.chars().chain("000".chars()).take(3).collect();
    let out_of_range = ParseError::NumberOutOfRange { pos: start };
    let magnitude = whole
        .parse::<i64>()
        .ok()
        .and_then(|w| w.checked_mul(1000))
        .zip(padded.parse::<i64>().ok())
        .and_then(|(w, f)| w.checked_add(f))
        .ok_or(out_of_range)?;
    Ok(Value::Milli(if negative { -magnitude } else { magnitude }))
}

fn digits(cursor: &mut Cursor<'_>) -> String {
    let mut out = String::new();
    while let Some(byte @ b'0'..=b'9') = cursor.peek() {
        out.push(char::from(byte));
        cursor.bump();
    }
    out
}

fn read_text(cursor: &mut Cursor<'_>) -> Result<String, ParseError> {
    cursor.bump();
    let mut out = String::new();
    loop {
        let Some(byte) = cursor.peek() else {
            return Err(ParseError::UnterminatedString { pos: cursor.pos() });
        };
        match byte {
            b'"' => {
                cursor.bump();
                return Ok(out);
            }
            b'\\' => {
                cursor.bump();
                out.push(read_escape(cursor)?);
            }
            0..=0x1f => return Err(ParseError::ControlByte { pos: cursor.pos() }),
            _ => {
                let text = std::str::from_utf8(cursor.rest())
                    .map_err(|_| ParseError::BadUtf8 { pos: cursor.pos() })?;
                let ch = text
                    .chars()
                    .next()
                    .ok_or(ParseError::EndOfInput { pos: cursor.pos() })?;
                out.push(ch);
                cursor.advance(ch.len_utf8());
            }
        }
    }
}

fn read_escape(cursor: &mut Cursor<'_>) -> Result<char, ParseError> {
    let pos = cursor.pos();
    let Some(byte) = cursor.peek() else {
        return Err(ParseError::UnterminatedEscape { pos });
    };
    cursor.bump();
    match byte {
        b'"' | b'\\' | b'/' => Ok(char::from(byte)),
        b'b' => Ok('\u{8}'),
        b'f' => Ok('\u{c}'),
        b'n' => Ok('\n'),
        b'r' => Ok('\r'),
        b't' => Ok('\t'),
        b'u' => {
            let mut code = 0u32;
            for _ in 0..4 {
                let digit = cursor
                    .peek()
                    .and_then(|b| char::from(b).to_digit(16))
                    .ok_or(ParseError::BadHex { pos: cursor.pos() })?;
                code = code * 16 + digit;
                cursor.bump();
            }
            char::from_u32(code).ok_or(ParseError::BadScalar { unit: code })
        }
        other => Err(ParseError::BadEscape {
            pos,
            byte: Some(other),
        }),
    }
}

fn read_list(cursor: &mut Cursor<'_>, depth: usize) -> Result<Value, ParseError> {
    cursor.bump();
    let mut items = Vec::new();
    cursor.ws();
    if cursor.eat(b']') {
        return Ok(Value::List(items));
    }
    loop {
        cursor.ws();
        items.push(read_value(cursor, depth + 1)?);
        cursor.ws();
        if cursor.eat(b']') {
            return Ok(Value::List(items));
        }
        if !cursor.eat(b',') {
            return Err(ParseError::MissingComma { pos: cursor.pos() });
        }
    }
}

fn read_map(cursor: &mut Cursor<'_>, depth: usize) -> Result<Value, ParseError> {
    cursor.bump();
    let mut entries = Vec::new();
    cursor.ws();
    if cursor.eat(b'}') {
        return Ok(Value::Map(entries));
    }
    loop {
        cursor.ws();
        if cursor.peek() != Some(b'"') {
            return Err(ParseError::ExpectedKey { pos: cursor.pos() });
        }
        let key = read_text(cursor)?;
        cursor.ws();
        if !cursor.eat(b':') {
            return Err(ParseError::MissingColon { pos: cursor.pos() });
        }
        cursor.ws();
        entries.push((key, read_value(cursor, depth + 1)?));
        cursor.ws();
        if cursor.eat(b'}') {
            return Ok(Value::Map(entries));
        }
        if !cursor.eat(b',') {
            return Err(ParseError::MissingComma { pos: cursor.pos() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two rows copied from a real staged-fight manifest.
    const STAGED: &str = concat!(
        r#"[{"schema":1,"task":"WIN6","tick":4,"checkpoint":"c9626092c6275e3f","#,
        r#""png_fnv":"db704b799e75ce73","fighters":[{"slot":0,"x":19.218,"z":3.438,"hp":1000,"hit":false},"#,
        r#"{"slot":1,"x":-16.567,"z":-11.156,"hp":760,"hit":true}],"size":[128,96]},"#,
        r#"{"schema":1,"tick":44,"png_fnv":"0989f5cdc2444a55","fighters":[],"size":[128,96]}]"#
    );

    const SINK: &str = concat!(
        r#"{"schema":1,"renderer":"presenter-twin","seed":7,"tape":"live","tape_fnv":null,"#,
        r#""width":96,"height":64,"rows":["WIN6"],"frames":[{"tick":0,"file":"frame_000000.png","#,
        r#""width":96,"height":64,"png_fnv":"00ff","sha256":"ab","bytes":9}]}"#
    );

    #[test]
    fn look_manifest_reads_the_staged_fight_shape_in_millimetres() {
        let manifest = parse_manifest(STAGED).expect("staged manifest reads");
        assert_eq!(manifest.frames.len(), 2);
        let first = &manifest.frames[0];
        assert_eq!(first.tick, Some(4));
        assert_eq!(first.png_fnv.as_deref(), Some("db704b799e75ce73"));
        assert_eq!(first.size, Some((128, 96)));
        assert_eq!(first.fighters.len(), 2);
        assert_eq!(
            first.fighters[1],
            ManifestFigure {
                slot: 1,
                x_mm: -16_567,
                z_mm: -11_156,
                hp: Some(760),
                hit: true
            }
        );
        assert!(manifest.frames[1].fighters.is_empty());
    }

    #[test]
    fn look_manifest_reads_the_capture_sink_shape() {
        let manifest = parse_manifest(SINK).expect("sink manifest reads");
        assert_eq!(manifest.frames.len(), 1);
        let row = &manifest.frames[0];
        assert_eq!(row.file.as_deref(), Some("frame_000000.png"));
        assert_eq!(row.size, Some((96, 64)));
        assert_eq!(row.png_fnv.as_deref(), Some("00ff"));
        assert!(row.fighters.is_empty());
    }

    #[test]
    fn look_manifest_refuses_bad_text_and_shapes() {
        assert!(matches!(
            parse_manifest("[{\"tick\":1.5e2}]"),
            Err(ManifestError::Syntax(ParseError::FloatRefused { .. }))
        ));
        assert!(matches!(
            parse_manifest("[{\"tick\":1}] trailing"),
            Err(ManifestError::Syntax(ParseError::TrailingBytes { .. }))
        ));
        assert!(matches!(
            parse_manifest("?not json"),
            Err(ManifestError::Syntax(ParseError::UnexpectedByte { .. }))
        ));
        assert!(matches!(
            parse_manifest("nope"),
            Err(ManifestError::Syntax(ParseError::BadLiteral { .. }))
        ));
        assert!(matches!(
            parse_manifest("{\"rows\":[]}"),
            Err(ManifestError::Shape(_))
        ));
        assert!(matches!(parse_manifest("7"), Err(ManifestError::Shape(_))));
        assert!(matches!(
            parse_manifest("[1,2]"),
            Err(ManifestError::Shape(_))
        ));
        let deep = format!("{}1{}", "[".repeat(12), "]".repeat(12));
        assert!(matches!(
            parse_manifest(&deep),
            Err(ManifestError::Syntax(ParseError::NestingTooDeep { .. }))
        ));
        let crowd = format!(
            "[{{\"fighters\":[{}]}}]",
            vec!["{\"slot\":1,\"x\":1,\"z\":1}"; MAX_FIGHTERS + 1].join(",")
        );
        assert!(matches!(
            parse_manifest(&crowd),
            Err(ManifestError::Shape(_))
        ));
        assert!(ManifestError::Shape("x")
            .to_string()
            .contains("shape refused"));
    }

    #[test]
    fn look_manifest_drops_digits_past_three_decimals_and_skips_unknown_keys() {
        let manifest = parse_manifest(
            "[{\"extra\":{\"a\":[true,null,\"q\\u0041\\n\"]},\"tick\":9,\"fighters\":[{\"slot\":2,\"x\":0.12399,\"z\":-0.5}]}]",
        )
        .expect("reads");
        let fighter = &manifest.frames[0].fighters[0];
        assert_eq!((fighter.x_mm, fighter.z_mm, fighter.hp), (123, -500, None));
        assert_eq!(manifest.frames[0].tick, Some(9));
    }

    #[test]
    fn look_manifest_same_text_reads_identically_twice() {
        assert_eq!(parse_manifest(STAGED), parse_manifest(STAGED));
        assert_eq!(parse_manifest(SINK), parse_manifest(SINK));
    }
}
