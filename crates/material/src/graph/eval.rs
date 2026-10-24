//! eval: the sample grid, the field operations and the record fold.
//!
//! Owner: `engine-material`. Split out of `graph` so no one file carries the
//! types and the sampling loop at once. Everything here is deterministic:
//! integer-hash noise, integer exponents, no RNG and no platform `powf`.

use super::{Axis, BlendMode, Node, NodeKind, Params};
use crate::json::Value;
use crate::record::{Channel, Layer};

/// One procedural field: `size * size` samples in `0..=1`, row-major.
#[derive(Clone, PartialEq, Debug)]
pub struct Field {
    /// Sample grid side, samples.
    pub size: u32,
    /// Samples, row-major.
    pub values: Vec<f32>,
}

impl Field {
    /// A field whose samples are all `value`.
    #[must_use]
    pub fn flat(size: u32, value: f32) -> Self {
        let count = usize::try_from(size)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(size).unwrap_or(0));
        Self {
            size,
            values: vec![value; count],
        }
    }

    /// Sample at a row-major index, clamped into the grid.
    #[must_use]
    pub fn at(&self, index: usize) -> f32 {
        self.values
            .get(index)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
    }

    /// Sample at a column and row, clamped into the grid.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> f32 {
        let side = self.size.max(1);
        let x = x.min(side - 1);
        let y = y.min(side - 1);
        let index = usize::try_from(y)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(side).unwrap_or(0))
            .saturating_add(usize::try_from(x).unwrap_or(0));
        self.at(index)
    }

    /// Mean of every sample.
    #[must_use]
    pub fn mean(&self) -> f32 {
        if self.values.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.values.iter().map(|v| v.clamp(0.0, 1.0)).sum();
        (sum / count_f32(self.values.len())).clamp(0.0, 1.0)
    }

    /// First differences at a row-major index, in samples per map side.
    pub(super) fn slope(&self, index: usize, size: u32) -> (f32, f32) {
        let side = i64::from(size.max(2));
        let column = i64::try_from(index).unwrap_or(0) % side;
        let row = i64::try_from(index).unwrap_or(0) / side;
        let x0 = u32::try_from((column + 1) % side).unwrap_or(0);
        let x1 = u32::try_from((column - 1 + side) % side).unwrap_or(0);
        let y0 = u32::try_from((row + 1) % side).unwrap_or(0);
        let y1 = u32::try_from((row - 1 + side) % side).unwrap_or(0);
        let dx = self.get(x0, u32::try_from(row).unwrap_or(0))
            - self.get(x1, u32::try_from(row).unwrap_or(0));
        let dy = self.get(u32::try_from(column).unwrap_or(0), y0)
            - self.get(u32::try_from(column).unwrap_or(0), y1);
        (dx, dy)
    }
}

/// The folded numbers of one graph: means plus the full height field.
#[derive(Clone, PartialEq, Debug)]
pub struct Surface {
    /// Mean base color, linear `0..=1`.
    pub base_color: [f32; 3],
    /// Mean roughness, `0..=1`.
    pub roughness: f32,
    /// Mean metalness, `0..=1`.
    pub metalness: f32,
    /// Mean coverage per [`Channel::all`] slot, `0..=1`.
    pub coverage: [f32; 5],
    /// Per-pixel coverage, [`Channel::all`] order; an unmapped channel is flat
    /// zero. The source of a coverage bake.
    pub coverage_fields: [Field; 5],
    /// The height field.
    pub height: Field,
    /// The emissive field (all zero when the graph names none).
    pub emissive: Field,
    /// Sample grid side.
    pub size: u32,
}

impl Surface {
    /// Mean coverage a channel reads; an unmapped channel is zero.
    #[must_use]
    pub fn coverage_of(&self, channel: Channel) -> f32 {
        Channel::all()
            .iter()
            .position(|c| *c == channel)
            .and_then(|slot| self.coverage.get(slot).copied())
            .unwrap_or(0.0)
    }
}

/// One baked map.
#[derive(Clone, PartialEq, Debug)]
pub struct BakeMap {
    /// Map name, one of [`BAKE_MAPS`].
    pub name: &'static str,
    /// Width, pixels.
    pub width: u32,
    /// Height, pixels.
    pub height: u32,
    /// PNG file bytes (linear `0..=1` samples quantized to 8 bits).
    pub png: Vec<u8>,
    /// Raw RGBA8 samples, row-major.
    pub rgba: Vec<u8>,
    /// Digest of the raw samples: equal graphs give equal digests.
    pub digest: u64,
    /// Smallest value per channel, `0..=1`.
    pub min: [f32; 4],
    /// Largest value per channel, `0..=1`.
    pub max: [f32; 4],
}

/// A bake of one graph: four maps and the digest of the graph text.
#[derive(Clone, PartialEq, Debug)]
pub struct Baked {
    /// Maps in [`BAKE_MAPS`] order.
    pub maps: Vec<BakeMap>,
    /// Digest of the canonical graph text.
    pub graph: u64,
}

/// Smallest and largest value per RGBA slot of a raw sample run, `0..=1`.
pub(super) fn range(rgba: &[u8]) -> ([f32; 4], [f32; 4]) {
    let mut min = [1.0f32; 4];
    let mut max = [0.0f32; 4];
    for pixel in rgba.chunks_exact(4) {
        for (slot, byte) in pixel.iter().enumerate() {
            let value = f32::from(*byte) / 255.0;
            if let Some(low) = min.get_mut(slot) {
                *low = low.min(value);
            }
            if let Some(high) = max.get_mut(slot) {
                *high = high.max(value);
            }
        }
    }
    (min, max)
}

/// Linear `0..=1` to one 8-bit sample, rounding half up.
pub(super) fn to_u8(value: f32) -> u8 {
    let unit = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let scaled = engine_core::checked::saturating_f64_to_u64(f64::from(unit) * 255.0 + 0.5);
    u8::try_from(scaled.min(255)).unwrap_or(255)
}

/// A sample count or a grid side as `f32`, without a truncating cast.
pub(super) fn count_f32(count: usize) -> f32 {
    engine_core::checked::f32_from_u64(u64::try_from(count).unwrap_or(u64::MAX)).max(1.0)
}

/// A number as canonical JSON text.
pub(super) fn decimal(value: f32) -> Value {
    Value::Number(format!("{value:.4}"))
}

pub(super) fn params_json(node: &Node) -> Value {
    let p = node.params;
    let mut members: Vec<(String, Value)> = Vec::new();
    match node.kind {
        NodeKind::Noise => {
            members.push(("scale".to_owned(), Value::Number(p.scale.to_string())));
            members.push(("seed".to_owned(), Value::Number(p.seed.to_string())));
            members.push(("octaves".to_owned(), Value::Number(p.octaves.to_string())));
        }
        NodeKind::Tile => {
            members.push(("rows".to_owned(), Value::Number(p.rows.to_string())));
            members.push(("cols".to_owned(), Value::Number(p.cols.to_string())));
            members.push(("mortar".to_owned(), decimal(p.mortar)));
        }
        NodeKind::Gradient => {
            members.push(("axis".to_owned(), Value::Str(p.axis.name().to_owned())));
            members.push(("from".to_owned(), decimal(p.from)));
            members.push(("to".to_owned(), decimal(p.to)));
        }
        NodeKind::Curve => {
            members.push(("exponent".to_owned(), Value::Number(p.exponent.to_string())));
            members.push(("invert".to_owned(), Value::Bool(p.invert)));
        }
        NodeKind::Warp => {
            members.push(("strength".to_owned(), decimal(p.strength)));
        }
        NodeKind::Blend => {
            members.push(("mode".to_owned(), Value::Str(p.mode.name().to_owned())));
        }
    }
    Value::Object(members)
}

pub(super) fn layer_value(layer: &Layer) -> Value {
    let mut members = vec![
        ("kind".to_owned(), Value::Str(layer.kind.name().to_owned())),
        (
            "driver".to_owned(),
            Value::Str(layer.driver.name().to_owned()),
        ),
        ("strength".to_owned(), decimal(layer.strength)),
    ];
    if let Some(tint) = layer.tint {
        members.push((
            "tint".to_owned(),
            Value::Array(tint.iter().map(|v| decimal(*v)).collect()),
        ));
    }
    if let Some(darken) = layer.darken {
        members.push(("darken".to_owned(), decimal(darken)));
    }
    if let Some(roughness) = layer.roughness_to {
        members.push(("roughness_to".to_owned(), decimal(roughness)));
    }
    Value::Object(members)
}

/// The integer hash a noise lattice point reads: a SplitMix step of the
/// seed, the cell and the octave, to one unit sample.
fn lattice(seed: u32, octave: u32, x: i64, y: i64) -> f32 {
    let mut word = u64::from(seed) << 32 ^ u64::from(octave) << 16;
    word ^= u64::from_le_bytes(x.to_le_bytes());
    word = engine_core::splitmix64(word.wrapping_add(u64::from_le_bytes(y.to_le_bytes())));
    let bits = (word >> 40) & 0xFF_FFFF;
    f32::from(u16::try_from(bits >> 8).unwrap_or(0)) / 65_535.0
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(params: &Params, size: u32, x: u32, y: u32) -> f32 {
    let side = f32::from(u16::try_from(size.max(1)).unwrap_or(1));
    let mut sum = 0.0f32;
    let mut amplitude = 1.0f32;
    let mut total = 0.0f32;
    let mut cells = params.scale.max(1);
    for octave in 0..params.octaves.max(1) {
        let u = f32::from(u16::try_from(x).unwrap_or(0)) / side;
        let v = f32::from(u16::try_from(y).unwrap_or(0)) / side;
        let scaled_u = u * f32::from(u16::try_from(cells).unwrap_or(1));
        let scaled_v = v * f32::from(u16::try_from(cells).unwrap_or(1));
        let x0 = i64::try_from(engine_core::checked::saturating_f64_to_u64(f64::from(
            scaled_u.floor(),
        )))
        .unwrap_or(0);
        let y0 = i64::try_from(engine_core::checked::saturating_f64_to_u64(f64::from(
            scaled_v.floor(),
        )))
        .unwrap_or(0);
        let fx = smooth(scaled_u - scaled_u.floor());
        let fy = smooth(scaled_v - scaled_v.floor());
        let a = lattice(params.seed, octave, x0, y0);
        let b = lattice(params.seed, octave, x0 + 1, y0);
        let c = lattice(params.seed, octave, x0, y0 + 1);
        let d = lattice(params.seed, octave, x0 + 1, y0 + 1);
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        sum += (top + (bottom - top) * fy) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        cells = cells.saturating_mul(2).min(512);
    }
    (sum / total.max(f32::EPSILON)).clamp(0.0, 1.0)
}

fn ratio(part: u32, whole: u32) -> f32 {
    let whole = whole.max(1);
    f32::from(u16::try_from(part.min(whole)).unwrap_or(0))
        / f32::from(u16::try_from(whole).unwrap_or(1))
}

fn tile_of(params: &Params, a: &Field, size: u32, x: u32, y: u32) -> f32 {
    let side = size.max(1);
    let rows = params.rows.max(1);
    let cols = params.cols.max(1);
    let scaled_y = y.saturating_mul(rows);
    let row = scaled_y / side;
    let fv = ratio(scaled_y % side, side);
    let mut scaled_x = x.saturating_mul(cols);
    if row % 2 == 1 {
        scaled_x = scaled_x.saturating_add(side / 2);
    }
    let fu = ratio(scaled_x % side, side);
    let m = params.mortar.clamp(0.0, 0.5);
    let mortar = fu < m || fu > 1.0 - m || fv < m || fv > 1.0 - m;
    if mortar {
        0.0
    } else {
        a.get(x, y)
    }
}

fn gradient_of(params: &Params, size: u32, x: u32, y: u32) -> f32 {
    let last = f32::from(u16::try_from(size.saturating_sub(1).max(1)).unwrap_or(1));
    let u = f32::from(u16::try_from(x).unwrap_or(0)) / last;
    let v = f32::from(u16::try_from(y).unwrap_or(0)) / last;
    let t = match params.axis {
        Axis::X => u,
        Axis::Y => v,
        Axis::Radial => {
            let dx = u - 0.5;
            let dy = v - 0.5;
            ((dx * dx + dy * dy).sqrt() * 2.0).clamp(0.0, 1.0)
        }
    };
    (params.from + (params.to - params.from) * t).clamp(0.0, 1.0)
}

fn integer_power(value: f32, exponent: u32) -> f32 {
    let mut out = 1.0f32;
    for _ in 0..exponent.max(1) {
        out *= value.clamp(0.0, 1.0);
    }
    out.clamp(0.0, 1.0)
}

/// One node's field from its inputs.
pub(super) fn compute(
    node: Node,
    size: u32,
    a: Option<&Field>,
    b: Option<&Field>,
    mask: Option<&Field>,
) -> Field {
    let mut field = Field::flat(size, 0.0);
    let side = size.max(1);
    for y in 0..side {
        for x in 0..side {
            let value = match node.kind {
                NodeKind::Noise => value_noise(&node.params, side, x, y),
                NodeKind::Gradient => gradient_of(&node.params, side, x, y),
                NodeKind::Tile => match a {
                    Some(a) => tile_of(&node.params, a, side, x, y),
                    None => 0.0,
                },
                NodeKind::Curve => match a {
                    Some(a) => {
                        let curved = integer_power(a.get(x, y), node.params.exponent);
                        if node.params.invert {
                            1.0 - curved
                        } else {
                            curved
                        }
                    }
                    None => 0.0,
                },
                NodeKind::Warp => match a {
                    Some(a) => {
                        let shift = b.map_or(0.0, |b| (b.get(x, y) - 0.5) * node.params.strength);
                        let cells = f32::from(u16::try_from(side).unwrap_or(1));
                        let offset = i64::try_from(engine_core::checked::saturating_f64_to_u64(
                            f64::from((shift * cells).round().abs()),
                        ))
                        .unwrap_or(0);
                        let offset = if shift < 0.0 { -offset } else { offset };
                        let sx = clamp_coord(i64::from(x) + offset, side);
                        a.get(sx, y)
                    }
                    None => 0.0,
                },
                NodeKind::Blend => match a {
                    Some(a) => {
                        let av = a.get(x, y);
                        let bv = b.map_or(0.0, |b| b.get(x, y));
                        let mv = mask.map_or(0.5, |m| m.get(x, y));
                        match node.params.mode {
                            BlendMode::Add => (av + bv).clamp(0.0, 1.0),
                            BlendMode::Multiply => av * bv,
                            BlendMode::Mix => av * mv + bv * (1.0 - mv),
                            BlendMode::Overlay => {
                                if av < 0.5 {
                                    2.0 * av * bv
                                } else {
                                    1.0 - 2.0 * (1.0 - av) * (1.0 - bv)
                                }
                            }
                        }
                    }
                    None => 0.0,
                },
            };
            let index = usize::try_from(y)
                .unwrap_or(0)
                .saturating_mul(usize::try_from(side).unwrap_or(0))
                .saturating_add(usize::try_from(x).unwrap_or(0));
            if let Some(slot) = field.values.get_mut(index) {
                *slot = value.clamp(0.0, 1.0);
            }
        }
    }
    field
}

fn clamp_coord(value: i64, side: u32) -> u32 {
    let last = i64::from(side.saturating_sub(1));
    u32::try_from(value.clamp(0, last)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_varies() {
        let params = Params::default();
        let a = value_noise(&params, 64, 3, 7);
        let b = value_noise(&params, 64, 3, 7);
        assert_eq!(a, b, "one lattice point is one sample");
        assert!((0.0..=1.0).contains(&a));
        let far = value_noise(&params, 64, 31, 19);
        assert_ne!(a, far, "two lattice points differ");
    }
}
