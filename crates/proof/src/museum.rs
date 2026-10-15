//! Format museum: stored v1 fixtures still load, migrate and replay.
//!
//! Consumer: the LT1 gate. Data owner: `fixtures/` (a tape v1 byte fixture
//! and a scene v1 JSON fixture). Determinism scope: loading, migration and
//! replay are pure; the same fixture bytes always give the same result.
//! Errors: [`Error`] for tapes, [`MuseumError`] for scenes. Corrupt bytes
//! are errors with bounded static diagnostics, never panics.

use std::fmt;

#[cfg(feature = "scene-museum")]
use engine_scene::codec::{CodecError, Scene, SceneNode, CURRENT_VERSION, MIN_SUPPORTED_VERSION};
#[cfg(feature = "scene-museum")]
use engine_scene::ids::ObjectId;
#[cfg(feature = "scene-museum")]
use engine_scene::transform::{Hierarchy, Transform};

use crate::tape::FORMAT_VERSION;
use crate::{replay, Checkpoint, Error, Tape};

/// Tape envelope version this museum loads; v1 is also current.
pub const MUSEUM_TAPE_FORMAT: u16 = FORMAT_VERSION;
/// Scene save version this museum loads; v1 is also current.
#[cfg(feature = "scene-museum")]
pub const MUSEUM_SCENE_VERSION: u32 = CURRENT_VERSION;
/// Upper bound on any diagnostic this module prints, so a corrupt fixture
/// can never flood a log with echoed input.
pub const MAX_DIAGNOSTIC_CHARS: usize = 256;
/// Largest accepted scene JSON text; checked before parsing to bound work.
#[cfg(feature = "scene-museum")]
pub const MAX_SCENE_TEXT: usize = 1 << 20;
/// Largest accepted scene node count; checked while extracting nodes.
#[cfg(feature = "scene-museum")]
pub const MAX_SCENE_NODES: usize = 10_000;
/// Deepest accepted JSON nesting; the scene shape needs only a few levels.
#[cfg(feature = "scene-museum")]
const MAX_JSON_DEPTH: usize = 32;
/// Most JSON values parsed from one document; bounds adversarial allocation.
#[cfg(feature = "scene-museum")]
const MAX_JSON_VALUES: usize = 65_536;

/// What went wrong loading a museum scene fixture.
///
/// Tape fixtures reuse [`Error`]; this covers the JSON text layer plus the
/// existing scene codec. No variant ever echoes unbounded input.
#[derive(Debug, PartialEq, Eq)]
pub enum MuseumError {
    /// The tape side failed (version gate or replay input invalid).
    Tape(Error),
    /// The existing scene codec refused the document (version, duplicate,
    /// missing parent, cycle, nil id or non-finite pose), culprit named.
    #[cfg(feature = "scene-museum")]
    Scene(CodecError),
    /// The JSON text itself is malformed or out of bounds; the static
    /// reason keeps diagnostics bounded.
    Encoding(&'static str),
}

impl fmt::Display for MuseumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tape(error) => write!(f, "museum tape: {error}"),
            #[cfg(feature = "scene-museum")]
            Self::Scene(error) => write!(f, "museum scene: {error}"),
            Self::Encoding(reason) => write!(f, "invalid scene text: {reason}"),
        }
    }
}

impl std::error::Error for MuseumError {}

impl From<Error> for MuseumError {
    fn from(error: Error) -> Self {
        Self::Tape(error)
    }
}

#[cfg(feature = "scene-museum")]
impl From<CodecError> for MuseumError {
    fn from(error: CodecError) -> Self {
        Self::Scene(error)
    }
}

/// Load a stored tape v1 fixture.
///
/// [`Tape::decode`] gates both the envelope and the fixture version, so any
/// non-v1 tape fails with [`Error::Version`] and any corrupt byte fails with
/// a bounded [`Error::Invalid`]; nothing here panics on input bytes.
pub fn load_tape_v1(bytes: &[u8]) -> Result<Tape, Error> {
    Tape::decode(bytes)
}

/// Load a v1 tape, run the migration gate and replay it, returning the
/// recorded checkpoints.
///
/// v1 is the current tape version, so migration is a checked no-op: the
/// decode gate above already rejected anything that is not v1. Later tape
/// versions add their field fixups here ahead of the replay.
pub fn migrate_and_replay(bytes: &[u8]) -> Result<Vec<Checkpoint>, Error> {
    let tape = load_tape_v1(bytes)?;
    replay(&tape)?;
    Ok(tape.checkpoints)
}

/// Load a stored scene v1 fixture from its JSON text.
///
/// The text is parsed with the std-only reader below (no new dependencies),
/// then the version is gated against the existing codec: anything outside
/// the supported range fails with [`CodecError::UnsupportedVersion`].
/// Corrupt text fails with a bounded [`MuseumError::Encoding`].
#[cfg(feature = "scene-museum")]
pub fn load_scene_v1(text: &str) -> Result<Scene, MuseumError> {
    if text.len() > MAX_SCENE_TEXT {
        return Err(MuseumError::Encoding("scene text exceeds the museum bound"));
    }
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        values: 0,
    };
    let root = parser.parse_document()?;
    scene_from_json(&root)
}

/// Load a v1 scene fixture, migrate it forward and decode the hierarchy.
///
/// v1 is the current scene version, so [`Scene::migrate`] only stamps the
/// version today; later versions run their fixups there. A dangling parent
/// link fails with [`CodecError::MissingParent`] naming both ends, and no
/// partial hierarchy escapes on any failure.
#[cfg(feature = "scene-museum")]
pub fn migrate_scene(text: &str) -> Result<Hierarchy, MuseumError> {
    let mut scene = load_scene_v1(text)?;
    scene.migrate()?;
    Ok(scene.decode()?)
}

/// Emit a scene document in the canonical museum text form: compact JSON,
/// struct field order, nodes in the given order.
///
/// The parser above accepts this output, so fixtures round-trip
/// structurally through `load_scene_v1(emit_scene(scene))`.
#[cfg(feature = "scene-museum")]
pub fn emit_scene(scene: &Scene) -> String {
    let mut text = String::from("{\"version\":");
    text.push_str(&scene.version.to_string());
    text.push_str(",\"nodes\":[");
    for (index, node) in scene.nodes.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str("{\"id\":\"");
        text.push_str(&node.id.to_string());
        text.push_str("\",\"parent\":\"");
        text.push_str(&node.parent.to_string());
        text.push_str("\",\"transform\":");
        push_transform(&mut text, &node.transform);
        text.push('}');
    }
    text.push_str("]}");
    text
}

#[cfg(feature = "scene-museum")]
fn push_transform(text: &mut String, transform: &Transform) {
    // One object with three arrays, matching the codec's field order.
    text.push_str("{\"position\":");
    push_float_array(text, &transform.position);
    text.push_str(",\"rotation\":");
    push_float_array(text, &transform.rotation);
    text.push_str(",\"scale\":");
    push_float_array(text, &transform.scale);
    text.push('}');
}

#[cfg(feature = "scene-museum")]
fn push_float_array(text: &mut String, values: &[f32]) {
    text.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&value.to_string());
    }
    text.push(']');
}

/// Minimal JSON value model, just enough for the scene document shape.
#[cfg(feature = "scene-museum")]
#[derive(Debug)]
enum Json {
    Null,
    True,
    False,
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Bounded recursive-descent JSON reader over the scene fixture text.
///
/// Every failure is a static reason: diagnostics never echo input, and
/// depth, value count and string escapes are all capped so hostile text
/// stays cheap to reject.
#[cfg(feature = "scene-museum")]
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    values: usize,
}

#[cfg(feature = "scene-museum")]
impl Parser<'_> {
    fn parse_document(&mut self) -> Result<Json, MuseumError> {
        self.skip_ws();
        let value = self.parse_value(0)?;
        self.skip_ws();
        if self.pos != self.bytes.len() {
            return Err(MuseumError::Encoding("scene text has trailing data"));
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Json, MuseumError> {
        if depth > MAX_JSON_DEPTH {
            return Err(MuseumError::Encoding("scene text nests too deeply"));
        }
        self.values += 1;
        if self.values > MAX_JSON_VALUES {
            return Err(MuseumError::Encoding("scene text holds too many values"));
        }
        match self.bytes.get(self.pos) {
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(b't') => self.parse_literal("true", Json::True),
            Some(b'f') => self.parse_literal("false", Json::False),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b'[') => self.parse_array(depth),
            Some(b'{') => self.parse_object(depth),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(MuseumError::Encoding("scene text holds invalid JSON")),
        }
    }

    fn parse_literal(&mut self, word: &str, value: Json) -> Result<Json, MuseumError> {
        if self.bytes.get(self.pos..self.pos + word.len()) == Some(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(MuseumError::Encoding("scene text holds invalid JSON"))
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Json, MuseumError> {
        self.pos += 1; // Consume `[`.
        let mut items = Vec::new();
        self.skip_ws();
        if self.eat(b']') {
            return Ok(Json::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value(depth + 1)?);
            self.skip_ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Json::Arr(items));
            }
            return Err(MuseumError::Encoding("scene array needs a comma"));
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Json, MuseumError> {
        self.pos += 1; // Consume `{`.
        let mut members = Vec::new();
        self.skip_ws();
        if self.eat(b'}') {
            return Ok(Json::Obj(members));
        }
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(MuseumError::Encoding("scene object keys must be strings"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if !self.eat(b':') {
                return Err(MuseumError::Encoding("scene object needs a colon"));
            }
            self.skip_ws();
            let value = self.parse_value(depth + 1)?;
            members.push((key, value));
            self.skip_ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Json::Obj(members));
            }
            return Err(MuseumError::Encoding("scene object needs a comma"));
        }
    }

    fn parse_string(&mut self) -> Result<String, MuseumError> {
        self.pos += 1; // Consume the opening quote.
                       // Raw bytes first: the input is `&str`, so plain bytes stay valid
                       // UTF-8 by construction, and escapes contribute encoded chars.
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or(MuseumError::Encoding("scene string is unterminated"))?;
            self.pos += 1;
            match byte {
                b'"' => {
                    return String::from_utf8(out)
                        .map_err(|_| MuseumError::Encoding("scene string is not utf8"));
                }
                b'\\' => {
                    let escaped = *self
                        .bytes
                        .get(self.pos)
                        .ok_or(MuseumError::Encoding("scene string is unterminated"))?;
                    self.pos += 1;
                    let next = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000C}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.parse_hex_char()?,
                        _ => return Err(MuseumError::Encoding("scene string escape is invalid")),
                    };
                    let mut encoded = [0; 4];
                    out.extend(next.encode_utf8(&mut encoded).as_bytes());
                }
                0x00..=0x1F => return Err(MuseumError::Encoding("scene string holds a control")),
                _ => out.push(byte),
            }
        }
    }

    fn parse_hex_char(&mut self) -> Result<char, MuseumError> {
        let high = self.escape_point()?;
        if (0xD800..0xDC00).contains(&high) {
            if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u".as_slice()) {
                return Err(MuseumError::Encoding("scene string surrogate lacks a pair"));
            }
            self.pos += 2;
            let low = self.escape_point()?;
            if !(0xDC00..0xE000).contains(&low) {
                return Err(MuseumError::Encoding("scene string surrogate lacks a pair"));
            }
            let point = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
            char::from_u32(point).ok_or(MuseumError::Encoding("scene string holds bad unicode"))
        } else if (0xDC00..0xE000).contains(&high) {
            Err(MuseumError::Encoding("scene string surrogate lacks a pair"))
        } else {
            char::from_u32(high).ok_or(MuseumError::Encoding("scene string holds bad unicode"))
        }
    }

    fn escape_point(&mut self) -> Result<u32, MuseumError> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or(MuseumError::Encoding("scene string is unterminated"))?;
        let value = engine_core::parse_hex4(digits)
            .ok_or(MuseumError::Encoding("scene string holds bad unicode"))?;
        self.pos += 4;
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Json, MuseumError> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.bytes.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(c) if c.is_ascii_digit() => {
                while self.bytes.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(MuseumError::Encoding("scene number is malformed")),
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !self.bytes.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                return Err(MuseumError::Encoding("scene number is malformed"));
            }
            while self.bytes.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.bytes.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                return Err(MuseumError::Encoding("scene number is malformed"));
            }
            while self.bytes.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let raw = self.bytes.get(start..self.pos).unwrap_or_default();
        let digits =
            str::from_utf8(raw).map_err(|_| MuseumError::Encoding("scene number is malformed"))?;
        let value: f64 = digits
            .parse()
            .map_err(|_| MuseumError::Encoding("scene number is malformed"))?;
        if !value.is_finite() {
            return Err(MuseumError::Encoding("scene number is not finite"));
        }
        Ok(Json::Num(value))
    }

    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(feature = "scene-museum")]
fn scene_from_json(root: &Json) -> Result<Scene, MuseumError> {
    let Json::Obj(members) = root else {
        return Err(MuseumError::Encoding("scene document must be an object"));
    };
    let mut version: Option<u32> = None;
    let mut nodes: Option<Vec<SceneNode>> = None;
    for (key, value) in members {
        match key.as_str() {
            "version" => {
                let Json::Num(number) = value else {
                    return Err(MuseumError::Encoding("scene version must be a number"));
                };
                if number.trunc() != *number || *number < 0.0 || *number > u32::MAX as f64 {
                    return Err(MuseumError::Encoding("scene version must be a u32"));
                }
                version = Some(*number as u32);
            }
            "nodes" => {
                let Json::Arr(items) = value else {
                    return Err(MuseumError::Encoding("scene nodes must be an array"));
                };
                if items.len() > MAX_SCENE_NODES {
                    return Err(MuseumError::Encoding("scene holds too many nodes"));
                }
                nodes = Some(
                    items
                        .iter()
                        .map(node_from_json)
                        .collect::<Result<Vec<SceneNode>, MuseumError>>()?,
                );
            }
            // Unknown fields are ignored so newer saves still parse their
            // v1 subset; the codec below owns semantic validation.
            _ => {}
        }
    }
    let version = version.ok_or(MuseumError::Encoding("scene document needs a version"))?;
    if version != CURRENT_VERSION {
        return Err(MuseumError::Scene(CodecError::UnsupportedVersion {
            got: version,
            min: MIN_SUPPORTED_VERSION,
            current: CURRENT_VERSION,
        }));
    }
    Ok(Scene {
        version,
        nodes: nodes.ok_or(MuseumError::Encoding("scene document needs its nodes"))?,
    })
}

#[cfg(feature = "scene-museum")]
fn node_from_json(value: &Json) -> Result<SceneNode, MuseumError> {
    let Json::Obj(members) = value else {
        return Err(MuseumError::Encoding("scene node must be an object"));
    };
    let mut id: Option<ObjectId> = None;
    let mut parent: Option<ObjectId> = None;
    let mut transform: Option<Transform> = None;
    for (key, value) in members {
        match key.as_str() {
            "id" => id = Some(link_from_json(value)?),
            "parent" => parent = Some(link_from_json(value)?),
            "transform" => transform = Some(transform_from_json(value)?),
            _ => {}
        }
    }
    Ok(SceneNode {
        id: id.ok_or(MuseumError::Encoding("scene node needs its id"))?,
        parent: parent.ok_or(MuseumError::Encoding("scene node needs its parent"))?,
        transform: transform.ok_or(MuseumError::Encoding("scene node needs its transform"))?,
    })
}

#[cfg(feature = "scene-museum")]
fn link_from_json(value: &Json) -> Result<ObjectId, MuseumError> {
    let Json::Str(text) = value else {
        return Err(MuseumError::Encoding("scene link must be a string"));
    };
    // Bound the parse input; uuid text is 36 chars, anything far larger is
    // hostile rather than a reference.
    if text.len() > 64 {
        return Err(MuseumError::Encoding("scene link is not a uuid"));
    }
    ObjectId::parse(text).map_err(|_| MuseumError::Encoding("scene link is not a uuid"))
}

#[cfg(feature = "scene-museum")]
fn transform_from_json(value: &Json) -> Result<Transform, MuseumError> {
    let Json::Obj(members) = value else {
        return Err(MuseumError::Encoding("scene transform must be an object"));
    };
    let mut position: Option<[f32; 3]> = None;
    let mut rotation: Option<[f32; 4]> = None;
    let mut scale: Option<[f32; 3]> = None;
    for (key, value) in members {
        match key.as_str() {
            "position" => position = Some(floats3(value)?),
            "rotation" => rotation = Some(floats4(value)?),
            "scale" => scale = Some(floats3(value)?),
            _ => {}
        }
    }
    Ok(Transform::new(
        position.ok_or(MuseumError::Encoding("scene transform needs position"))?,
        rotation.ok_or(MuseumError::Encoding("scene transform needs rotation"))?,
        scale.ok_or(MuseumError::Encoding("scene transform needs scale"))?,
    ))
}

#[cfg(feature = "scene-museum")]
fn floats3(value: &Json) -> Result<[f32; 3], MuseumError> {
    let Json::Arr(items) = value else {
        return Err(MuseumError::Encoding("scene triple must be an array"));
    };
    if items.len() != 3 {
        return Err(MuseumError::Encoding("scene triple needs 3 numbers"));
    }
    Ok([to_f32(&items[0])?, to_f32(&items[1])?, to_f32(&items[2])?])
}

#[cfg(feature = "scene-museum")]
fn floats4(value: &Json) -> Result<[f32; 4], MuseumError> {
    let Json::Arr(items) = value else {
        return Err(MuseumError::Encoding("scene quad must be an array"));
    };
    if items.len() != 4 {
        return Err(MuseumError::Encoding("scene quad needs 4 numbers"));
    }
    Ok([
        to_f32(&items[0])?,
        to_f32(&items[1])?,
        to_f32(&items[2])?,
        to_f32(&items[3])?,
    ])
}

#[cfg(feature = "scene-museum")]
fn to_f32(value: &Json) -> Result<f32, MuseumError> {
    let Json::Num(number) = value else {
        return Err(MuseumError::Encoding("scene component must be a number"));
    };
    // JSON text cannot spell NaN or infinity, but huge exponents still
    // parse to an out-of-range value; keep the cast exact-or-error.
    if number.abs() > f32::MAX as f64 {
        return Err(MuseumError::Encoding("scene component is outside f32"));
    }
    Ok(*number as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record;

    /// Stored tape v1 museum fixture: seed 7, 24 ticks, one velocity input.
    static TAPE_V1: &[u8] = include_bytes!("../../../fixtures/tape-v1.bin");
    #[cfg(feature = "scene-museum")]
    static SCENE_V1: &str = include_str!("../../../fixtures/scene-v1.json");

    #[test]
    fn tape_v1_loads_migrates_and_replays_to_same_checkpoints_twice() {
        assert_eq!(MUSEUM_TAPE_FORMAT, 1);
        let first = migrate_and_replay(TAPE_V1).unwrap();
        let second = migrate_and_replay(TAPE_V1).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());

        // The fixture documents one museum piece: seed 7, 24 ticks, 1 input.
        let tape = load_tape_v1(TAPE_V1).unwrap();
        assert_eq!(tape.config.seed, 7);
        assert_eq!(tape.config.ticks, 24);
        assert_eq!(tape.inputs.len(), 1);
        // A live record of the same config and inputs reaches the same
        // checkpoints, so the fixture still replays on this build.
        let live = record(tape.config, &tape.inputs).unwrap();
        assert_eq!(live.checkpoints, first);
    }

    #[cfg(feature = "scene-museum")]
    #[test]
    fn scene_v1_loads_migrates_and_matches_the_codec() {
        assert_eq!(MUSEUM_SCENE_VERSION, 1);
        let scene = load_scene_v1(SCENE_V1).unwrap();
        assert_eq!(scene.version, CURRENT_VERSION);
        assert_eq!(scene.nodes.len(), 3);

        let tree = migrate_scene(SCENE_V1).unwrap();
        assert_eq!(tree.validate(), Ok(()));
        assert_eq!(tree.len(), 3);

        // Chain root -> child -> grandchild with the authored local poses.
        let root = ObjectId::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let child = ObjectId::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let grand = ObjectId::parse("33333333-3333-4333-8333-333333333333").unwrap();
        assert_eq!(tree.parent(root), Some(ObjectId::NIL));
        assert_eq!(tree.parent(child), Some(root));
        assert_eq!(tree.parent(grand), Some(child));
        assert_eq!(
            tree.local(grand),
            Some(Transform::new(
                [0.0, 0.0, 3.0],
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 1.0, 1.0]
            ))
        );

        // The fixture parses to exactly what the codec itself encodes for
        // the same hierarchy, so the file is a genuine v1 save.
        let mut rebuilt = Hierarchy::new();
        for node in &scene.nodes {
            rebuilt
                .insert(node.id, node.parent, node.transform)
                .unwrap();
        }
        assert_eq!(Scene::encode(&rebuilt), scene);
        // And the museum text form round-trips structurally.
        let again = load_scene_v1(&emit_scene(&scene)).unwrap();
        assert_eq!(again, scene);
    }

    #[test]
    fn corrupt_tapes_are_rejected_with_bounded_diagnostics() {
        let mut bad_tapes: Vec<Vec<u8>> =
            vec![vec![], vec![0; 7], TAPE_V1[..TAPE_V1.len() - 1].to_vec()];
        let mut bad_magic = TAPE_V1.to_vec();
        bad_magic[0] ^= 0xff;
        bad_tapes.push(bad_magic);
        let mut bad_format = TAPE_V1.to_vec();
        bad_format[8..10].copy_from_slice(&99u16.to_le_bytes());
        bad_tapes.push(bad_format);
        let mut bad_fixture = TAPE_V1.to_vec();
        bad_fixture[10..12].copy_from_slice(&99u16.to_le_bytes());
        bad_tapes.push(bad_fixture);
        let mut trailing = TAPE_V1.to_vec();
        trailing.push(0);
        bad_tapes.push(trailing);

        for bytes in &bad_tapes {
            let error = migrate_and_replay(bytes).unwrap_err();
            assert!(
                format!("{error}").len() <= MAX_DIAGNOSTIC_CHARS,
                "tape diagnostic too long: {error}"
            );
        }
        // Version-gated tapes name the offending versions.
        assert!(matches!(
            load_tape_v1(&bad_tapes[4]),
            Err(Error::Version { .. })
        ));
    }

    #[cfg(feature = "scene-museum")]
    #[test]
    fn corrupt_scenes_are_rejected_with_bounded_diagnostics() {
        let bad_scenes = [
            "",
            "{",
            // Drop the closing brace (past any trailing newline) so the
            // document is genuinely truncated, not just un-newlined.
            &SCENE_V1.trim_end()[..SCENE_V1.trim_end().len() - 1],
            "{\"version\":2,\"nodes\":[]}",
            "{\"version\":1}",
            "{\"nodes\":[]}",
            "{\"version\":1,\"nodes\":[7]}",
            "{\"version\":1,\"nodes\":[{\"id\":\"not-an-id\",\"parent\":\"00000000-0000-0000-0000-000000000000\",\"transform\":{\"position\":[0,0,0],\"rotation\":[0,0,0,1],\"scale\":[1,1,1]}}]}",
            "{\"version\":1,\"nodes\":[{\"id\":\"11111111-1111-4111-8111-111111111111\",\"parent\":\"00000000-0000-0000-0000-000000000000\",\"transform\":{\"position\":[0,0],\"rotation\":[0,0,0,1],\"scale\":[1,1,1]}}]}",
            "{\"version\":1,\"nodes\":[{\"id\":\"11111111-1111-4111-8111-111111111111\",\"parent\":\"00000000-0000-0000-0000-000000000000\",\"transform\":{\"position\":[1e999,0,0],\"rotation\":[0,0,0,1],\"scale\":[1,1,1]}}]}",
        ];
        for text in bad_scenes {
            let error = migrate_scene(text).unwrap_err();
            assert!(
                format!("{error}").len() <= MAX_DIAGNOSTIC_CHARS,
                "scene diagnostic too long: {error}"
            );
        }
        // The unsupported scene version names the save and the reader range.
        assert_eq!(
            migrate_scene("{\"version\":2,\"nodes\":[]}"),
            Err(MuseumError::Scene(CodecError::UnsupportedVersion {
                got: 2,
                min: MIN_SUPPORTED_VERSION,
                current: CURRENT_VERSION,
            }))
        );
    }

    #[cfg(feature = "scene-museum")]
    #[test]
    fn missing_reference_names_both_ends_and_leaves_nothing() {
        let mut scene = load_scene_v1(SCENE_V1).unwrap();
        let orphan = ObjectId::parse("44444444-4444-4444-8444-444444444444").unwrap();
        let ghost = ObjectId::parse("55555555-5555-4555-8555-555555555555").unwrap();
        assert!(!scene.nodes.iter().any(|node| node.id == ghost));
        scene.nodes.push(SceneNode {
            id: orphan,
            parent: ghost,
            transform: Transform::identity(),
        });

        let error = migrate_scene(&emit_scene(&scene)).unwrap_err();
        assert_eq!(
            error,
            MuseumError::Scene(CodecError::MissingParent {
                node: orphan,
                parent: ghost,
            })
        );
        // The diagnostic quotes both ends of the dangling link, still bounded.
        let report = format!("{error}");
        assert!(report.contains(&orphan.to_string()));
        assert!(report.contains(&ghost.to_string()));
        assert!(report.len() <= MAX_DIAGNOSTIC_CHARS);
    }
}
