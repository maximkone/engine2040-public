//! body: the body description as data - a body file read into plain fields.
//!
//! Owner: this crate. A body file says what one body is made of: the bones
//! with their capsules, how hard the muscles brace, how much blood it sheds
//! and what a cut at each joint costs. It is data, not code: the file is read
//! once at load and the fields are read from then on.
//!
//! The file is a strict subset of TOML: comments start with `#`, a value is a
//! quoted string or a number, a table header is `[a.b]`, and the keys are the
//! ones below. Nothing else is read; an unknown key or table is refused.
//!
//! Units: meters for the capsules, permille for the muscle and the gore,
//! whole newton-seconds for a cut.
//!
//! The gore of a match is not in this file: engine-combat owns that policy and
//! it caps what a body shows. The permille here is how much this body sheds,
//! and the muscle is how hard its joints brace.
//!
//! Determinism: the same text gives the same description, field for field.
//!
//! Errors: typed, one per bad field, and the line it was found on.
//!
//! Budget: read once at load; the fields are plain data afterwards.

use crate::error::{Error, Field};
use crate::rig::Rig;
use std::io;
use std::path::{Path, PathBuf};

/// The most gore a body may shed, permille of the whole.
pub const GORE_FULL: u16 = 1000;

/// The most a body may brace, permille of the whole.
pub const MUSCLE_FULL: u16 = 1000;

/// One bone of a body: its name, its parent and the capsule around it.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneRow {
    /// The bone's name, the same name the animation uses.
    pub name: String,
    /// The parent's name, empty for the root bone.
    pub parent: String,
    /// Capsule radius, meters.
    pub radius: f32,
    /// Capsule half height, meters.
    pub half_height: f32,
}

/// One joint of a body a blade may take, and what the cut costs.
#[derive(Clone, Debug, PartialEq)]
pub struct CutRow {
    /// The joint's name, the same name the animation uses.
    pub joint: String,
    /// The energy the cut costs, newton-seconds.
    pub energy: u32,
}

/// A body, read from a body file.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyDesc {
    /// The body's name, for the page and the receipts.
    pub name: String,
    /// The bones, parents before children.
    pub bones: Vec<BoneRow>,
    /// How hard the muscles brace, permille of the whole; 0 lets every joint
    /// give way at once.
    pub muscle_permille: u16,
    /// How much blood the body sheds, permille of the whole.
    pub gore_permille: u16,
    /// The joints a blade may take.
    pub cuts: Vec<CutRow>,
}

/// What a body file got wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyError {
    /// A line that is not a key, a value or a table header.
    Syntax {
        /// The line number, counting from one.
        line: usize,
    },
    /// A key the file does not have.
    UnknownKey {
        /// The key that was not known.
        key: String,
        /// The line number, counting from one.
        line: usize,
    },
    /// A table the file does not have.
    UnknownTable {
        /// The table that was not known.
        table: String,
        /// The line number, counting from one.
        line: usize,
    },
    /// The same key twice.
    Duplicate {
        /// The key that came twice.
        key: String,
        /// The line number, counting from one.
        line: usize,
    },
    /// A key the body needs but the file does not have.
    Missing {
        /// The key that was missing.
        key: String,
    },
    /// A value that is not what the key asks for.
    BadValue {
        /// The key whose value was wrong.
        key: String,
        /// The value that was wrong.
        value: String,
        /// The line number, counting from one.
        line: usize,
    },
    /// A bone whose parent comes after it, or names no bone.
    Order {
        /// The bone that was out of order.
        bone: String,
        /// The line number, counting from one.
        line: usize,
    },
    /// A name the body does not have.
    UnknownBone {
        /// The name that is not a bone of this body.
        name: String,
        /// The line number, counting from one.
        line: usize,
    },
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyError::Syntax { line } => {
                write!(f, "line {line} is not a key, a value or a table header")
            }
            BodyError::UnknownKey { key, line } => {
                write!(f, "line {line} has the unknown key `{key}`")
            }
            BodyError::UnknownTable { table, line } => {
                write!(f, "line {line} opens the unknown table `{table}`")
            }
            BodyError::Duplicate { key, line } => {
                write!(f, "line {line} has the key `{key}` twice")
            }
            BodyError::Missing { key } => write!(f, "the body file has no `{key}`"),
            BodyError::BadValue { key, value, line } => {
                write!(f, "line {line} has the bad value `{value}` for `{key}`")
            }
            BodyError::Order { bone, line } => {
                write!(f, "line {line} has the bone `{bone}` before its parent")
            }
            BodyError::UnknownBone { name, line } => {
                write!(
                    f,
                    "line {line} names the bone `{name}`, which this body does not have"
                )
            }
        }
    }
}

impl std::error::Error for BodyError {}

/// What a body file on disk got wrong.
#[derive(Debug)]
pub enum BodyFileError {
    /// The file could not be read.
    Read {
        /// The path that was read.
        path: PathBuf,
        /// The reason the read failed.
        error: io::Error,
    },
    /// The file's text is not a body.
    Parse {
        /// The path that was read.
        path: PathBuf,
        /// What the text got wrong, with the key and the line.
        error: BodyError,
    },
}

impl std::fmt::Display for BodyFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyFileError::Read { path, error } => {
                write!(
                    f,
                    "the body file `{}` could not be read: {error}",
                    path.display()
                )
            }
            BodyFileError::Parse { path, error } => {
                write!(
                    f,
                    "the body file `{}` is not a body: {error}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for BodyFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BodyFileError::Read { error, .. } => Some(error),
            BodyFileError::Parse { error, .. } => Some(error),
        }
    }
}

/// One value of a body file: a quoted string or a number.
enum Value {
    /// A quoted string.
    Text(String),
    /// A number, with the text it was written as.
    Number(String),
}

/// One key of a body file, with the table it sits in.
struct Entry {
    /// The table path, empty at the top of the file.
    table: Vec<String>,
    /// The key.
    key: String,
    /// The value.
    value: Value,
    /// The line number, counting from one.
    line: usize,
}

/// Reads a body file into a [`BodyDesc`].
///
/// The file is a strict subset of TOML:
///
/// ```text
/// name = "gladiator"
/// gore_permille = 1000
/// muscle_permille = 900
///
/// [bone.pelvis]
/// parent = ""
/// radius = 0.09
/// half_height = 0.0
///
/// [cut.arm_upper_l]
/// energy = 40
/// ```
pub fn parse(text: &str) -> Result<BodyDesc, BodyError> {
    let entries = entries(text)?;
    let mut file = BodyFile::default();
    for entry in &entries {
        match entry.table.as_slice() {
            [] => file.top_key(entry)?,
            [kind, item] => file.table_key(entry, kind, item)?,
            other => {
                return Err(BodyError::UnknownTable {
                    table: other.join("."),
                    line: entry.line,
                })
            }
        }
    }
    file.finish()
}

/// Reads a body file from disk into a [`BodyDesc`].
///
/// Read once at load: keep the body and pass `&body` to the four fx calls, so
/// no tick reads the disk. A file that cannot be read, or whose text is not a
/// body, is refused with its path; a bad text keeps the key and the line it was
/// found on.
///
/// Native only: the browser has no filesystem and parses fetched bytes with
/// [`parse`] instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Result<BodyDesc, BodyFileError> {
    let text = std::fs::read_to_string(path).map_err(|error| BodyFileError::Read {
        path: path.to_path_buf(),
        error,
    })?;
    parse(&text).map_err(|error| BodyFileError::Parse {
        path: path.to_path_buf(),
        error,
    })
}

/// The browser cannot read a path; a fetched body is parsed with [`parse`].
#[cfg(target_arch = "wasm32")]
pub fn load(path: &Path) -> Result<BodyDesc, BodyFileError> {
    Err(BodyFileError::Read {
        path: path.to_path_buf(),
        error: std::io::Error::new(std::io::ErrorKind::NotFound, "no filesystem in the browser"),
    })
}

/// The file as it is read: the top keys and the tables, before they are checked
/// against each other.
#[derive(Default)]
struct BodyFile {
    /// The body's name, once it is read.
    name: Option<String>,
    /// The gore in permille, once it is read.
    gore: Option<u16>,
    /// The muscle in permille, once it is read.
    muscle: Option<u16>,
    /// The bones, parents before children.
    bones: Vec<BoneRow>,
    /// The joints a blade may take.
    cuts: Vec<CutRow>,
}

impl BodyFile {
    /// A key at the top of the file: the name, the gore or the muscle.
    fn top_key(&mut self, entry: &Entry) -> Result<(), BodyError> {
        match entry.key.as_str() {
            "name" => {
                if self.name.is_some() {
                    return Err(BodyError::Duplicate {
                        key: entry.key.clone(),
                        line: entry.line,
                    });
                }
                self.name = Some(text_of(entry)?);
            }
            "gore_permille" => {
                if self.gore.is_some() {
                    return Err(BodyError::Duplicate {
                        key: entry.key.clone(),
                        line: entry.line,
                    });
                }
                self.gore = Some(permille_of(entry, GORE_FULL)?);
            }
            "muscle_permille" => {
                if self.muscle.is_some() {
                    return Err(BodyError::Duplicate {
                        key: entry.key.clone(),
                        line: entry.line,
                    });
                }
                self.muscle = Some(permille_of(entry, MUSCLE_FULL)?);
            }
            other => {
                return Err(BodyError::UnknownKey {
                    key: other.to_string(),
                    line: entry.line,
                })
            }
        }
        Ok(())
    }

    /// A key inside a table: a bone row or a cut row.
    fn table_key(&mut self, entry: &Entry, kind: &str, item: &str) -> Result<(), BodyError> {
        match kind {
            "bone" => {
                let row = bone_row(&mut self.bones, item);
                match entry.key.as_str() {
                    "parent" => row.parent = text_of(entry)?,
                    "radius" => row.radius = extent_of(entry, true)?,
                    "half_height" => row.half_height = extent_of(entry, false)?,
                    other => {
                        return Err(BodyError::UnknownKey {
                            key: other.to_string(),
                            line: entry.line,
                        })
                    }
                }
            }
            "cut" => {
                let row = cut_row(&mut self.cuts, item);
                if entry.key != "energy" {
                    return Err(BodyError::UnknownKey {
                        key: entry.key.clone(),
                        line: entry.line,
                    });
                }
                row.energy = energy_of(entry)?;
            }
            other => {
                return Err(BodyError::UnknownTable {
                    table: format!("{other}.{item}"),
                    line: entry.line,
                })
            }
        }
        Ok(())
    }

    /// The file as a body: every key present, the parents named before their
    /// children and the cuts on bones this body has.
    fn finish(self) -> Result<BodyDesc, BodyError> {
        let name = self.name.ok_or_else(|| BodyError::Missing {
            key: "name".to_string(),
        })?;
        let gore_permille = self.gore.ok_or_else(|| BodyError::Missing {
            key: "gore_permille".to_string(),
        })?;
        let muscle = self.muscle.ok_or_else(|| BodyError::Missing {
            key: "muscle_permille".to_string(),
        })?;
        if self.bones.is_empty() {
            return Err(BodyError::Missing {
                key: "bone".to_string(),
            });
        }
        check_parents(&self.bones)?;
        check_cuts(&self.cuts, &self.bones)?;
        Ok(BodyDesc {
            name,
            bones: self.bones,
            muscle_permille: muscle,
            gore_permille,
            cuts: self.cuts,
        })
    }
}

/// Splits the file into its keys, with the table each sits in.
fn entries(text: &str) -> Result<Vec<Entry>, BodyError> {
    let mut entries = Vec::new();
    let mut table: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = strip_comment(raw).trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(header) = trimmed.strip_prefix('[') {
            let header = header.strip_suffix(']').ok_or(BodyError::Syntax { line })?;
            table = header
                .split('.')
                .map(str::trim)
                .map(str::to_string)
                .collect();
            if table.is_empty() || table.iter().any(String::is_empty) {
                return Err(BodyError::Syntax { line });
            }
            continue;
        }
        let (key, value) = trimmed.split_once('=').ok_or(BodyError::Syntax { line })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(BodyError::Syntax { line });
        }
        entries.push(Entry {
            table: table.clone(),
            key: key.to_string(),
            value: value_of(value.trim(), line)?,
            line,
        });
    }
    Ok(entries)
}

/// Drops a comment that starts outside a quoted string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

/// Reads a value: a quoted string or a number.
fn value_of(text: &str, line: usize) -> Result<Value, BodyError> {
    if let Some(inner) = text.strip_prefix('"') {
        let inner = inner.strip_suffix('"').ok_or(BodyError::Syntax { line })?;
        return Ok(Value::Text(inner.to_string()));
    }
    if text.is_empty() {
        return Err(BodyError::Syntax { line });
    }
    if text.parse::<f64>().is_err() {
        return Err(BodyError::Syntax { line });
    }
    Ok(Value::Number(text.to_string()))
}

/// The string value of an entry.
fn text_of(entry: &Entry) -> Result<String, BodyError> {
    match &entry.value {
        Value::Text(text) => Ok(text.clone()),
        Value::Number(raw) => Err(BodyError::BadValue {
            key: entry.key.clone(),
            value: raw.clone(),
            line: entry.line,
        }),
    }
}

/// The number value of an entry, with the text it was written as.
fn number_of(entry: &Entry) -> Result<&str, BodyError> {
    match &entry.value {
        Value::Number(raw) => Ok(raw),
        Value::Text(text) => Err(BodyError::BadValue {
            key: entry.key.clone(),
            value: text.clone(),
            line: entry.line,
        }),
    }
}

/// The number value of an entry, as a whole number of permille. A value past
/// the cap is refused.
fn permille_of(entry: &Entry, cap: u16) -> Result<u16, BodyError> {
    let raw = number_of(entry)?;
    let value = raw.parse::<u16>().map_err(|_| BodyError::BadValue {
        key: entry.key.clone(),
        value: raw.to_string(),
        line: entry.line,
    })?;
    if value > cap {
        return Err(BodyError::BadValue {
            key: entry.key.clone(),
            value: raw.to_string(),
            line: entry.line,
        });
    }
    Ok(value)
}

/// The number value of an entry, as a whole number of newton-seconds.
fn energy_of(entry: &Entry) -> Result<u32, BodyError> {
    let raw = number_of(entry)?;
    raw.parse::<u32>().map_err(|_| BodyError::BadValue {
        key: entry.key.clone(),
        value: raw.to_string(),
        line: entry.line,
    })
}

/// The number value of an entry, as a capsule size in meters.
fn extent_of(entry: &Entry, above_zero: bool) -> Result<f32, BodyError> {
    let raw = number_of(entry)?;
    let value = raw.parse::<f32>().map_err(|_| BodyError::BadValue {
        key: entry.key.clone(),
        value: raw.to_string(),
        line: entry.line,
    })?;
    let ok = value.is_finite()
        && if above_zero {
            value > 0.0
        } else {
            value >= 0.0
        };
    if !ok {
        return Err(BodyError::BadValue {
            key: entry.key.clone(),
            value: raw.to_string(),
            line: entry.line,
        });
    }
    Ok(value)
}

/// The bone row of a table, found or made.
fn bone_row<'a>(bones: &'a mut Vec<BoneRow>, name: &str) -> &'a mut BoneRow {
    if let Some(at) = bones.iter().position(|row| row.name == name) {
        return &mut bones[at];
    }
    bones.push(BoneRow {
        name: name.to_string(),
        parent: String::new(),
        radius: 0.0,
        half_height: 0.0,
    });
    let last = bones.len() - 1;
    &mut bones[last]
}

/// The cut row of a table, found or made.
fn cut_row<'a>(cuts: &'a mut Vec<CutRow>, name: &str) -> &'a mut CutRow {
    if let Some(at) = cuts.iter().position(|row| row.joint == name) {
        return &mut cuts[at];
    }
    cuts.push(CutRow {
        joint: name.to_string(),
        energy: 0,
    });
    let last = cuts.len() - 1;
    &mut cuts[last]
}

/// Checks the bones: a parent is a bone of this body, declared before it.
fn check_parents(bones: &[BoneRow]) -> Result<(), BodyError> {
    for (index, bone) in bones.iter().enumerate() {
        if bone.parent.is_empty() {
            continue;
        }
        let found = bones
            .iter()
            .position(|row| row.name == bone.parent)
            .is_some_and(|at| at < index);
        if !found {
            return Err(BodyError::Order {
                bone: bone.name.clone(),
                line: index + 1,
            });
        }
    }
    Ok(())
}

/// Checks the cut table: every joint is a bone of this body.
fn check_cuts(cuts: &[CutRow], bones: &[BoneRow]) -> Result<(), BodyError> {
    for (index, cut) in cuts.iter().enumerate() {
        if !bones.iter().any(|row| row.name == cut.joint) {
            return Err(BodyError::UnknownBone {
                name: cut.joint.clone(),
                line: index + 1,
            });
        }
    }
    Ok(())
}

impl BodyDesc {
    /// The rig this body is made of: the bones as capsules, parents first, and
    /// the muscles of the body as the brace strength.
    ///
    /// The names and the parents are the ones the animation uses, so the pose
    /// that lands on this rig is the pose the animation drew. A bone whose
    /// name the animation would refuse is refused here too.
    pub fn rig(&self) -> Result<Rig, Error> {
        let mut skeleton = engine_anim::contract::Skeleton::default();
        let mut shapes = Vec::with_capacity(self.bones.len());
        for bone in &self.bones {
            let parent = if bone.parent.is_empty() {
                None
            } else {
                let at = self
                    .bones
                    .iter()
                    .position(|other| other.name == bone.parent)
                    .ok_or(Error::OutOfRange(Field::Bone))?;
                Some(at)
            };
            skeleton.joints.push(engine_anim::contract::Joint {
                name: bone.name.clone(),
                parent,
            });
            shapes.push(engine_physics::ragdoll::BoneShape {
                radius: bone.radius,
                half_height: bone.half_height,
            });
        }
        Rig::new(skeleton, shapes)?.with_muscle_permille(self.muscle_permille)
    }

    /// The bone row of this body, by name.
    pub fn bone(&self, name: &str) -> Option<&BoneRow> {
        self.bones.iter().find(|bone| bone.name == name)
    }

    /// The energy a cut at this joint costs, if the joint may be cut at all.
    pub fn cut_energy(&self, joint: &str) -> Option<u32> {
        self.cuts
            .iter()
            .find(|row| row.joint == joint)
            .map(|row| row.energy)
    }
}
