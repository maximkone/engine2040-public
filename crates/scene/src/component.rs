//! Typed, versioned scene components beyond `label`: mesh, collider,
//! material, light, spawner, destructible, anatomy, rig.
//!
//! Consumer: the editor command service stores one [`Component`] per node and
//! kind through `SetComponent` / `RemoveComponent` ops, the save, WAL and
//! durable codecs write [`Component::to_bytes`], and the door turns authored
//! JSON into the same values. Units: meters, radians-free (colors are linear
//! 0..=1, lights in lumens-free scene units), time in integer ticks, ids as
//! [`AssetName`]. Determinism: every payload has one canonical byte form
//! (kind tag, then fields in declaration order, little-endian, f32 as raw
//! bits) and one canonical JSON form (fields in declaration order), so the
//! same value always saves, hashes and prints the same bytes. Errors:
//! [`ComponentError`] names the component, the field and the legal range;
//! nothing here panics on authored input. Versioning: [`SCHEMA_VERSION`] is
//! 1 for all eight kinds; [`register_builtin`] adds them to a
//! [`SchemaRegistry`] so older payloads migrate forward through
//! [`crate::schema`] before they parse. Layer: this module has no editor
//! knowledge; the editor and the door depend on it, never the reverse.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::schema::{Schema, SchemaError, SchemaRegistry};
#[cfg(feature = "json")]
use crate::schema::VersionedComponent;

/// Version every shipped component kind starts at.
pub const SCHEMA_VERSION: u32 = 1;
/// Longest accepted asset or archetype name in bytes.
pub const MAX_ASSET_NAME_LEN: usize = 64;
/// Most chunks a destructible may name (matches the fracture caps).
pub const MAX_CHUNKS: u16 = 4096;
/// Most limbs an anatomy may name.
pub const MAX_LIMBS: u8 = 32;
/// Largest per-axis extent, radius or range in meters.
pub const MAX_EXTENT_M: f32 = 100_000.0;

/// A validated asset name: short ASCII (`0-9 A-Z a-z - _ . /`), copy-sized.
///
/// Fixed inline storage keeps the editor op enum `Copy` and the bytes stable;
/// empty is unrepresentable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetName {
    len: u8,
    bytes: [u8; MAX_ASSET_NAME_LEN],
}

impl AssetName {
    /// Name text; construction only admits ASCII, so the fallback is dead.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

impl fmt::Debug for AssetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetName({:?})", self.as_str())
    }
}

impl fmt::Display for AssetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AssetName {
    type Error = ComponentError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        if name.is_empty() {
            return Err(ComponentError::BadName {
                reason: "asset name is empty; name the asset",
            });
        }
        if name.len() > MAX_ASSET_NAME_LEN {
            return Err(ComponentError::BadName {
                reason: "asset name is longer than 64 bytes; shorten it",
            });
        }
        let ok =
            |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/');
        if !name.bytes().all(ok) {
            return Err(ComponentError::BadName {
                reason: "asset name allows only 0-9 A-Z a-z - _ . /; rename it",
            });
        }
        let mut bytes = [0u8; MAX_ASSET_NAME_LEN];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        // The length was bounded by MAX_ASSET_NAME_LEN (64) above, so it fits.
        let len = u8::try_from(name.len()).map_err(|_| ComponentError::BadName {
            reason: "asset name is longer than 64 bytes; shorten it",
        })?;
        Ok(Self { len, bytes })
    }
}

impl Serialize for AssetName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AssetName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::try_from(text.as_str()).map_err(serde::de::Error::custom)
    }
}

/// Which of the eight component kinds a value is. Orders by tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComponentKind {
    /// Mesh asset to draw.
    Mesh,
    /// Collision shape.
    Collider,
    /// Material reference plus tint.
    Material,
    /// Light source.
    Light,
    /// Entity spawner.
    Spawner,
    /// Destructible body (fracture budget).
    Destructible,
    /// Body plan: limb template.
    Anatomy,
    /// Skeleton plus clip set.
    Rig,
}

/// Every kind, in tag order.
pub const ALL_KINDS: [ComponentKind; 8] = [
    ComponentKind::Mesh,
    ComponentKind::Collider,
    ComponentKind::Material,
    ComponentKind::Light,
    ComponentKind::Spawner,
    ComponentKind::Destructible,
    ComponentKind::Anatomy,
    ComponentKind::Rig,
];

impl ComponentKind {
    /// Stable one-byte tag used by every binary codec.
    pub fn tag(self) -> u8 {
        match self {
            Self::Mesh => 1,
            Self::Collider => 2,
            Self::Material => 3,
            Self::Light => 4,
            Self::Spawner => 5,
            Self::Destructible => 6,
            Self::Anatomy => 7,
            Self::Rig => 8,
        }
    }

    /// Kind for a tag, if the tag is known.
    pub fn from_tag(tag: u8) -> Option<Self> {
        ALL_KINDS.into_iter().find(|kind| kind.tag() == tag)
    }

    /// Schema name (`"mesh"`, `"collider"`, ...), as authored in JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Collider => "collider",
            Self::Material => "material",
            Self::Light => "light",
            Self::Spawner => "spawner",
            Self::Destructible => "destructible",
            Self::Anatomy => "anatomy",
            Self::Rig => "rig",
        }
    }

    /// Kind for a schema name, if known.
    pub fn from_name(name: &str) -> Option<Self> {
        ALL_KINDS.into_iter().find(|kind| kind.name() == name)
    }

    /// Comma list of every legal name, for refusal messages.
    pub fn name_list() -> String {
        ALL_KINDS
            .iter()
            .map(|kind| kind.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What rejected a component value. Every variant names the legal range or
/// shape and one recovery, so an agent can fix its input and retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentError {
    /// An asset name failed its shape check.
    BadName {
        /// Static reason with the recovery.
        reason: &'static str,
    },
    /// A float field was NaN or infinite.
    NonFinite {
        /// Component kind.
        component: &'static str,
        /// Field name.
        field: &'static str,
    },
    /// A numeric field was outside its legal range.
    OutOfRange {
        /// Component kind.
        component: &'static str,
        /// Field name.
        field: &'static str,
        /// Legal range, as text (`"1..=4096"`).
        range: &'static str,
    },
    /// A component name no schema serves.
    UnknownComponent {
        /// The name given.
        name: String,
    },
    /// The binary form was cut short, had a bad tag or trailing bytes.
    BadBytes {
        /// Static reason.
        reason: &'static str,
    },
    /// The JSON payload did not fit the component's schema.
    BadPayload {
        /// Component kind.
        component: &'static str,
        /// Parser cause, bounded.
        cause: String,
    },
    /// The version stamp could not be served or migrated.
    Schema(SchemaError),
}

impl fmt::Display for ComponentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName { reason } => write!(f, "bad asset name: {reason}"),
            Self::NonFinite { component, field } => write!(
                f,
                "{component}.{field} is not a finite number; send a finite value"
            ),
            Self::OutOfRange {
                component,
                field,
                range,
            } => write!(
                f,
                "{component}.{field} is outside {range}; send a value inside that range"
            ),
            Self::UnknownComponent { name } => write!(
                f,
                "unknown component `{name}`; the schemas are [{}]; re-run with one of those",
                ComponentKind::name_list()
            ),
            Self::BadBytes { reason } => write!(f, "bad component bytes: {reason}"),
            Self::BadPayload { component, cause } => write!(
                f,
                "{component} payload does not fit its v{SCHEMA_VERSION} schema ({cause}); \
                 describe the component for its fields and re-run"
            ),
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ComponentError {}

impl From<SchemaError> for ComponentError {
    fn from(error: SchemaError) -> Self {
        Self::Schema(error)
    }
}

/// Mesh asset to draw.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    /// Mesh asset name.
    pub asset: AssetName,
    /// True when the mesh casts shadows.
    #[serde(default = "yes")]
    pub cast_shadows: bool,
}

/// Collision shape family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColliderShape {
    /// Box: `half` holds the three half extents.
    Box,
    /// Sphere: `half[0]` is the radius.
    Sphere,
    /// Capsule: `half[0]` is the radius, `half[1]` the half height.
    Capsule,
}

/// Collision shape.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collider {
    /// Shape family.
    pub shape: ColliderShape,
    /// Half extents in meters (see [`ColliderShape`] for the meaning).
    pub half: [f32; 3],
    /// True for a trigger volume that reports overlaps without pushing.
    #[serde(default)]
    pub sensor: bool,
}

/// Material reference plus tint.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialRef {
    /// Material asset name.
    pub asset: AssetName,
    /// Linear RGB tint, each 0..=1 (white is no tint).
    #[serde(default = "white")]
    pub tint: [f32; 3],
}

/// Light family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightKind {
    /// Omnidirectional point light.
    Point,
    /// Cone light.
    Spot,
    /// Parallel sun-like light.
    Directional,
}

/// Light source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    /// Light family.
    pub kind: LightKind,
    /// Linear RGB color, each 0..=1.
    pub color: [f32; 3],
    /// Intensity in scene units, 0 or more.
    pub intensity: f32,
    /// Reach in meters (ignored by directional lights).
    pub range: f32,
}

/// Entity spawner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawner {
    /// Archetype name to spawn.
    pub archetype: AssetName,
    /// Entities per burst, at least 1.
    pub count: u16,
    /// Ticks between bursts, at least 1 (integer ticks, never seconds).
    pub period_ticks: u32,
}

/// Destructible body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destructible {
    /// Physical material name (toughness and sound come from it).
    pub material: AssetName,
    /// Integer health, at least 1.
    pub health: u32,
    /// Chunk budget for fracture, 1..=[`MAX_CHUNKS`].
    pub chunks: u16,
}

/// Body plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anatomy {
    /// Anatomy template name.
    pub template: AssetName,
    /// Limb count, 1..=[`MAX_LIMBS`].
    pub limbs: u8,
}

/// Skeleton plus clip set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rig {
    /// Skeleton asset name.
    pub skeleton: AssetName,
    /// Clip set asset name.
    pub clips: AssetName,
}

fn yes() -> bool {
    true
}

fn white() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

/// One typed component value. `Copy` and inline so the editor op stays `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Component {
    /// See [`Mesh`].
    Mesh(Mesh),
    /// See [`Collider`].
    Collider(Collider),
    /// See [`MaterialRef`].
    Material(MaterialRef),
    /// See [`Light`].
    Light(Light),
    /// See [`Spawner`].
    Spawner(Spawner),
    /// See [`Destructible`].
    Destructible(Destructible),
    /// See [`Anatomy`].
    Anatomy(Anatomy),
    /// See [`Rig`].
    Rig(Rig),
}

fn finite(component: &'static str, field: &'static str, value: f32) -> Result<(), ComponentError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ComponentError::NonFinite { component, field })
    }
}

fn within(
    component: &'static str,
    field: &'static str,
    value: f32,
    low: f32,
    high: f32,
    range: &'static str,
) -> Result<(), ComponentError> {
    finite(component, field, value)?;
    if value < low || value > high {
        return Err(ComponentError::OutOfRange {
            component,
            field,
            range,
        });
    }
    Ok(())
}

impl Component {
    /// Kind of this value.
    pub fn kind(&self) -> ComponentKind {
        match self {
            Self::Mesh(_) => ComponentKind::Mesh,
            Self::Collider(_) => ComponentKind::Collider,
            Self::Material(_) => ComponentKind::Material,
            Self::Light(_) => ComponentKind::Light,
            Self::Spawner(_) => ComponentKind::Spawner,
            Self::Destructible(_) => ComponentKind::Destructible,
            Self::Anatomy(_) => ComponentKind::Anatomy,
            Self::Rig(_) => ComponentKind::Rig,
        }
    }

    /// Check every field against its legal range. Names are validated at
    /// construction; this covers floats and counts.
    pub fn validate(&self) -> Result<(), ComponentError> {
        match self {
            Self::Mesh(_) | Self::Rig(_) => Ok(()),
            Self::Collider(c) => {
                for axis in c.half {
                    within(
                        "collider",
                        "half",
                        axis,
                        1e-4,
                        MAX_EXTENT_M,
                        "0.0001..=100000",
                    )?;
                }
                Ok(())
            }
            Self::Material(m) => {
                for channel in m.tint {
                    within("material", "tint", channel, 0.0, 1.0, "0..=1")?;
                }
                Ok(())
            }
            Self::Light(l) => {
                for channel in l.color {
                    within("light", "color", channel, 0.0, 1.0, "0..=1")?;
                }
                within("light", "intensity", l.intensity, 0.0, 1.0e6, "0..=1000000")?;
                within("light", "range", l.range, 0.0, MAX_EXTENT_M, "0..=100000")
            }
            Self::Spawner(s) => {
                if s.count == 0 {
                    return Err(ComponentError::OutOfRange {
                        component: "spawner",
                        field: "count",
                        range: "1..=65535",
                    });
                }
                if s.period_ticks == 0 {
                    return Err(ComponentError::OutOfRange {
                        component: "spawner",
                        field: "period_ticks",
                        range: "1..=4294967295",
                    });
                }
                Ok(())
            }
            Self::Destructible(d) => {
                if d.health == 0 {
                    return Err(ComponentError::OutOfRange {
                        component: "destructible",
                        field: "health",
                        range: "1..=4294967295",
                    });
                }
                if d.chunks == 0 || d.chunks > MAX_CHUNKS {
                    return Err(ComponentError::OutOfRange {
                        component: "destructible",
                        field: "chunks",
                        range: "1..=4096",
                    });
                }
                Ok(())
            }
            Self::Anatomy(a) => {
                if a.limbs == 0 || a.limbs > MAX_LIMBS {
                    return Err(ComponentError::OutOfRange {
                        component: "anatomy",
                        field: "limbs",
                        range: "1..=32",
                    });
                }
                Ok(())
            }
        }
    }

    /// Canonical binary form: kind tag, then fields in declaration order.
    /// Same value, same bytes, on every machine.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![self.kind().tag()];
        match self {
            Self::Mesh(m) => {
                put_name(&mut out, &m.asset);
                out.push(u8::from(m.cast_shadows));
            }
            Self::Collider(c) => {
                out.push(match c.shape {
                    ColliderShape::Box => 0,
                    ColliderShape::Sphere => 1,
                    ColliderShape::Capsule => 2,
                });
                put_floats(&mut out, &c.half);
                out.push(u8::from(c.sensor));
            }
            Self::Material(m) => {
                put_name(&mut out, &m.asset);
                put_floats(&mut out, &m.tint);
            }
            Self::Light(l) => {
                out.push(match l.kind {
                    LightKind::Point => 0,
                    LightKind::Spot => 1,
                    LightKind::Directional => 2,
                });
                put_floats(&mut out, &l.color);
                put_floats(&mut out, &[l.intensity, l.range]);
            }
            Self::Spawner(s) => {
                put_name(&mut out, &s.archetype);
                out.extend_from_slice(&s.count.to_le_bytes());
                out.extend_from_slice(&s.period_ticks.to_le_bytes());
            }
            Self::Destructible(d) => {
                put_name(&mut out, &d.material);
                out.extend_from_slice(&d.health.to_le_bytes());
                out.extend_from_slice(&d.chunks.to_le_bytes());
            }
            Self::Anatomy(a) => {
                put_name(&mut out, &a.template);
                out.push(a.limbs);
            }
            Self::Rig(r) => {
                put_name(&mut out, &r.skeleton);
                put_name(&mut out, &r.clips);
            }
        }
        out
    }

    /// Parse the canonical binary form, strictly: a bad tag, a cut-short
    /// body, trailing bytes or an out-of-range field refuses.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ComponentError> {
        let mut cursor = Cursor { bytes, pos: 0 };
        let tag = cursor.u8()?;
        let kind = ComponentKind::from_tag(tag).ok_or(ComponentError::BadBytes {
            reason: "unknown component tag",
        })?;
        let value = match kind {
            ComponentKind::Mesh => Self::Mesh(Mesh {
                asset: cursor.name()?,
                cast_shadows: cursor.flag()?,
            }),
            ComponentKind::Collider => {
                let shape = match cursor.u8()? {
                    0 => ColliderShape::Box,
                    1 => ColliderShape::Sphere,
                    2 => ColliderShape::Capsule,
                    _ => {
                        return Err(ComponentError::BadBytes {
                            reason: "unknown collider shape",
                        })
                    }
                };
                Self::Collider(Collider {
                    shape,
                    half: cursor.floats()?,
                    sensor: cursor.flag()?,
                })
            }
            ComponentKind::Material => Self::Material(MaterialRef {
                asset: cursor.name()?,
                tint: cursor.floats()?,
            }),
            ComponentKind::Light => {
                let kind = match cursor.u8()? {
                    0 => LightKind::Point,
                    1 => LightKind::Spot,
                    2 => LightKind::Directional,
                    _ => {
                        return Err(ComponentError::BadBytes {
                            reason: "unknown light kind",
                        })
                    }
                };
                let color = cursor.floats()?;
                let intensity = cursor.f32()?;
                let range = cursor.f32()?;
                Self::Light(Light {
                    kind,
                    color,
                    intensity,
                    range,
                })
            }
            ComponentKind::Spawner => Self::Spawner(Spawner {
                archetype: cursor.name()?,
                count: u16::from_le_bytes(cursor.num()?),
                period_ticks: u32::from_le_bytes(cursor.num()?),
            }),
            ComponentKind::Destructible => Self::Destructible(Destructible {
                material: cursor.name()?,
                health: u32::from_le_bytes(cursor.num()?),
                chunks: u16::from_le_bytes(cursor.num()?),
            }),
            ComponentKind::Anatomy => Self::Anatomy(Anatomy {
                template: cursor.name()?,
                limbs: cursor.u8()?,
            }),
            ComponentKind::Rig => Self::Rig(Rig {
                skeleton: cursor.name()?,
                clips: cursor.name()?,
            }),
        };
        if cursor.pos != bytes.len() {
            return Err(ComponentError::BadBytes {
                reason: "trailing bytes after the component",
            });
        }
        value.validate()?;
        Ok(value)
    }

    /// One-line preview text, for diffs and audit effects.
    pub fn describe(&self) -> String {
        match self {
            Self::Mesh(m) => format!("mesh {}", m.asset),
            Self::Collider(c) => format!(
                "collider {:?} half {:?}{}",
                c.shape,
                c.half,
                if c.sensor { " sensor" } else { "" }
            ),
            Self::Material(m) => format!("material {} tint {:?}", m.asset, m.tint),
            Self::Light(l) => format!(
                "light {:?} intensity {} range {}",
                l.kind, l.intensity, l.range
            ),
            Self::Spawner(s) => format!(
                "spawner {} x{} every {} ticks",
                s.archetype, s.count, s.period_ticks
            ),
            Self::Destructible(d) => format!(
                "destructible {} health {} chunks {}",
                d.material, d.health, d.chunks
            ),
            Self::Anatomy(a) => format!("anatomy {} limbs {}", a.template, a.limbs),
            Self::Rig(r) => format!("rig {} clips {}", r.skeleton, r.clips),
        }
    }
}

fn put_name(out: &mut Vec<u8>, name: &AssetName) {
    out.push(name.len);
    out.extend_from_slice(name.as_str().as_bytes());
}

fn put_floats(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

/// Bounds-checked reader over the canonical bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ComponentError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len());
        let end = end.ok_or(ComponentError::BadBytes {
            reason: "component bytes end early",
        })?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ComponentError> {
        Ok(self.take(1)?[0])
    }

    fn flag(&mut self) -> Result<bool, ComponentError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ComponentError::BadBytes {
                reason: "flag byte is not 0 or 1",
            }),
        }
    }

    /// The next `N` bytes as an array (one shape for every integer width).
    fn num<const N: usize>(&mut self) -> Result<[u8; N], ComponentError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn f32(&mut self) -> Result<f32, ComponentError> {
        Ok(f32::from_bits(u32::from_le_bytes(self.num()?)))
    }

    fn floats(&mut self) -> Result<[f32; 3], ComponentError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    fn name(&mut self) -> Result<AssetName, ComponentError> {
        let len = usize::from(self.u8()?);
        let raw = self.take(len)?;
        let text = core::str::from_utf8(raw).map_err(|_| ComponentError::BadBytes {
            reason: "asset name is not UTF-8",
        })?;
        AssetName::try_from(text)
    }
}

/// Register all eight kinds at [`SCHEMA_VERSION`] in `registry`.
pub fn register_builtin(registry: &mut SchemaRegistry) -> Result<(), SchemaError> {
    for kind in ALL_KINDS {
        registry.register(Schema {
            component: kind.name(),
            version: SCHEMA_VERSION,
            migrate: None,
        })?;
    }
    Ok(())
}

#[cfg(feature = "json")]
impl Component {
    /// Canonical JSON payload: fields in declaration order, no spaces.
    pub fn to_json(&self) -> String {
        let text = match self {
            Self::Mesh(v) => serde_json::to_string(v),
            Self::Collider(v) => serde_json::to_string(v),
            Self::Material(v) => serde_json::to_string(v),
            Self::Light(v) => serde_json::to_string(v),
            Self::Spawner(v) => serde_json::to_string(v),
            Self::Destructible(v) => serde_json::to_string(v),
            Self::Anatomy(v) => serde_json::to_string(v),
            Self::Rig(v) => serde_json::to_string(v),
        };
        // Plain data structs with string keys always serialize.
        text.unwrap_or_default()
    }

    /// Parse a JSON payload of `kind` at the current schema version and
    /// validate its ranges.
    pub fn from_json(kind: ComponentKind, payload: &str) -> Result<Self, ComponentError> {
        let fail = |error: serde_json::Error| ComponentError::BadPayload {
            component: kind.name(),
            cause: error.to_string().chars().take(160).collect(),
        };
        let value = match kind {
            ComponentKind::Mesh => Self::Mesh(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Collider => Self::Collider(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Material => Self::Material(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Light => Self::Light(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Spawner => Self::Spawner(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Destructible => {
                Self::Destructible(serde_json::from_str(payload).map_err(fail)?)
            }
            ComponentKind::Anatomy => Self::Anatomy(serde_json::from_str(payload).map_err(fail)?),
            ComponentKind::Rig => Self::Rig(serde_json::from_str(payload).map_err(fail)?),
        };
        value.validate()?;
        Ok(value)
    }

    /// The versioned wire form (`component`, `version`, canonical payload).
    pub fn to_versioned(&self) -> VersionedComponent {
        VersionedComponent {
            component: self.kind().name().to_owned(),
            version: SCHEMA_VERSION,
            payload: self.to_json(),
        }
    }

    /// Migrate a versioned value forward through `registry`, then parse it.
    /// Unknown names and versions refuse with the legal names.
    pub fn from_versioned(
        registry: &SchemaRegistry,
        value: &VersionedComponent,
    ) -> Result<Self, ComponentError> {
        let kind = ComponentKind::from_name(&value.component).ok_or_else(|| {
            ComponentError::UnknownComponent {
                name: value.component.chars().take(64).collect(),
            }
        })?;
        let current = registry.migrate_value(value)?;
        Self::from_json(kind, &current.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> AssetName {
        AssetName::try_from(text).expect("test name is valid")
    }

    /// One valid sample of every kind, in tag order.
    fn samples() -> [Component; 8] {
        [
            Component::Mesh(Mesh {
                asset: name("meshes/wall"),
                cast_shadows: true,
            }),
            Component::Collider(Collider {
                shape: ColliderShape::Box,
                half: [1.0, 2.5, 0.125],
                sensor: false,
            }),
            Component::Material(MaterialRef {
                asset: name("marble"),
                tint: [1.0, 0.5, 0.25],
            }),
            Component::Light(Light {
                kind: LightKind::Spot,
                color: [1.0, 0.9, 0.8],
                intensity: 12.5,
                range: 30.0,
            }),
            Component::Spawner(Spawner {
                archetype: name("fighter.grunt"),
                count: 3,
                period_ticks: 600,
            }),
            Component::Destructible(Destructible {
                material: name("stone"),
                health: 400,
                chunks: 64,
            }),
            Component::Anatomy(Anatomy {
                template: name("humanoid"),
                limbs: 4,
            }),
            Component::Rig(Rig {
                skeleton: name("hero_skel"),
                clips: name("hero_clips"),
            }),
        ]
    }

    #[test]
    fn eight_kinds_roundtrip_bytes_and_stay_in_tag_order() {
        let samples = samples();
        for (value, kind) in samples.iter().zip(ALL_KINDS) {
            assert_eq!(value.kind(), kind);
            assert_eq!(ComponentKind::from_tag(kind.tag()), Some(kind));
            assert_eq!(ComponentKind::from_name(kind.name()), Some(kind));
            value.validate().expect("sample validates");
            let bytes = value.to_bytes();
            assert_eq!(bytes[0], kind.tag());
            let back = Component::from_bytes(&bytes).expect("bytes parse");
            assert_eq!(&back, value);
            assert_eq!(back.to_bytes(), bytes, "canonical bytes are stable");
        }
    }

    #[test]
    fn bad_bytes_refuse_with_a_reason() {
        let good = samples()[0].to_bytes();
        assert!(Component::from_bytes(&[]).is_err());
        assert!(Component::from_bytes(&[99]).is_err());
        assert!(Component::from_bytes(&good[..good.len() - 1]).is_err());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(Component::from_bytes(&trailing).is_err());
        let mut flag = good;
        let last = flag.len() - 1;
        flag[last] = 7;
        assert!(Component::from_bytes(&flag).is_err());
    }

    #[test]
    fn bad_values_refuse_with_the_field_and_range() {
        let nan = Component::Collider(Collider {
            shape: ColliderShape::Sphere,
            half: [f32::NAN, 1.0, 1.0],
            sensor: false,
        });
        assert!(matches!(
            nan.validate(),
            Err(ComponentError::NonFinite {
                component: "collider",
                field: "half"
            })
        ));
        let flat = Component::Collider(Collider {
            shape: ColliderShape::Box,
            half: [0.0, 1.0, 1.0],
            sensor: false,
        });
        assert!(flat.validate().is_err());
        let hot = Component::Material(MaterialRef {
            asset: name("m"),
            tint: [2.0, 0.0, 0.0],
        });
        assert!(hot.validate().is_err());
        let still = Component::Spawner(Spawner {
            archetype: name("a"),
            count: 1,
            period_ticks: 0,
        });
        let error = still.validate().expect_err("zero period refuses");
        assert!(error.to_string().contains("period_ticks"));
        let empty = Component::Destructible(Destructible {
            material: name("a"),
            health: 1,
            chunks: 0,
        });
        assert!(empty.validate().is_err());
        let limbless = Component::Anatomy(Anatomy {
            template: name("a"),
            limbs: 0,
        });
        assert!(limbless.validate().is_err());
        for bad in ["", "has space", "semi;colon", &"x".repeat(65)] {
            assert!(AssetName::try_from(bad).is_err(), "`{bad}` refuses");
        }
    }

    #[test]
    fn builtin_registry_serves_all_eight_at_v1() {
        let mut registry = SchemaRegistry::new();
        register_builtin(&mut registry).expect("builtin registers");
        for kind in ALL_KINDS {
            assert_eq!(registry.current_version(kind.name()), Some(SCHEMA_VERSION));
        }
        assert_eq!(registry.current_version("label"), None);
        // The shipped registry carries label plus all eight kinds.
        let shipped = SchemaRegistry::builtin();
        assert_eq!(shipped.current_version("label"), Some(1));
        for kind in ALL_KINDS {
            assert_eq!(shipped.current_version(kind.name()), Some(SCHEMA_VERSION));
        }
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_is_canonical_and_roundtrips_through_the_registry() {
        let mut registry = SchemaRegistry::new();
        register_builtin(&mut registry).expect("builtin registers");
        for value in samples() {
            let versioned = value.to_versioned();
            assert_eq!(versioned.version, SCHEMA_VERSION);
            let back = Component::from_versioned(&registry, &versioned).expect("parses");
            assert_eq!(back, value);
            assert_eq!(back.to_versioned(), versioned, "same value, same JSON text");
        }
        // Unknown fields, bad enums and range errors refuse typed.
        let unknown = Component::from_json(ComponentKind::Mesh, r#"{"asset":"a","extra":1}"#);
        assert!(matches!(unknown, Err(ComponentError::BadPayload { .. })));
        let range = Component::from_json(
            ComponentKind::Light,
            r#"{"kind":"point","color":[1,1,1],"intensity":-1,"range":5}"#,
        );
        assert!(matches!(range, Err(ComponentError::OutOfRange { .. })));
        let defaulted = Component::from_json(ComponentKind::Mesh, r#"{"asset":"a"}"#)
            .expect("shadow default applies");
        assert_eq!(
            defaulted,
            Component::Mesh(Mesh {
                asset: name("a"),
                cast_shadows: true
            })
        );
        let future = VersionedComponent {
            component: "mesh".to_owned(),
            version: 9,
            payload: "{}".to_owned(),
        };
        assert!(matches!(
            Component::from_versioned(&registry, &future),
            Err(ComponentError::Schema(SchemaError::UnknownVersion { .. }))
        ));
        let alien = VersionedComponent {
            component: "teleporter".to_owned(),
            version: 1,
            payload: "{}".to_owned(),
        };
        let refused = Component::from_versioned(&registry, &alien).expect_err("refuses");
        assert!(refused.to_string().contains("mesh, collider"));
    }
}
