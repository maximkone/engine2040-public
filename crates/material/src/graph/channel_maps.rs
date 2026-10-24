//! channel_maps: the five coverage channel rasters of a bake.
//!
//! The bake's four maps carry the surface look; the coverage set carries what
//! the gameplay layers read (`wetness`, `dirt`, `blood`, `strain`, `heat`).
//! Five channels do not fit one RGBA sheet, so the set is written three ways:
//!
//! - one map per channel ([`COVERAGE_MAPS`]), the channel in red, green and
//!   blue zero, alpha opaque;
//! - `coverage`, a 4+1 split's first sheet: red wetness, green dirt, blue
//!   blood, alpha strain;
//! - `coverage_extra`, the second sheet: red heat, green and blue zero, alpha
//!   opaque.
//!
//! A channel whose every sample is the same value is *not* refused: a graph
//! may drive a channel flat on purpose. The map is written and the channel's
//! name is listed in [`ChannelBake::flat`] so the caller can report a warning
//! row. Only the look maps keep the hard flat refusal.

use super::eval::{range, to_u8, BakeMap, Surface};
use crate::record::Channel;
use engine_core::hash_bytes;

/// The five per-channel map names, [`Channel::all`] order.
pub const COVERAGE_MAPS: [&str; 5] = [
    "coverage_wetness",
    "coverage_dirt",
    "coverage_blood",
    "coverage_strain",
    "coverage_heat",
];

/// The two packed sheets: the 4+1 split of the five channels.
pub const COVERAGE_SHEET_MAPS: [&str; 2] = ["coverage", "coverage_extra"];

/// Which channel each sheet slot carries; `None` leaves the slot zero.
const SHEETS: [([Option<Channel>; 4], &str); 2] = [
    (
        [
            Some(Channel::Wetness),
            Some(Channel::Dirt),
            Some(Channel::Blood),
            Some(Channel::Strain),
        ],
        "coverage",
    ),
    ([Some(Channel::Heat), None, None, None], "coverage_extra"),
];

/// The coverage half of a bake: five channel maps, the two packed sheets and
/// the names of the channels that came out flat.
#[derive(Clone, PartialEq, Debug)]
pub struct ChannelBake {
    /// One map per channel, [`COVERAGE_MAPS`] order.
    pub maps: Vec<BakeMap>,
    /// The two packed sheets, [`COVERAGE_SHEET_MAPS`] order.
    pub sheets: Vec<BakeMap>,
    /// Channels flat on every sample, in [`COVERAGE_MAPS`] order: a warning,
    /// not a refusal.
    pub flat: Vec<&'static str>,
}

/// Sample count of a square map.
fn pixels(resolution: u32) -> usize {
    usize::try_from(resolution)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(resolution).unwrap_or(0))
}

/// A raw run as a [`BakeMap`], with its flatness reported instead of refused.
fn map(name: &'static str, resolution: u32, rgba: Vec<u8>) -> BakeMap {
    let (min, max) = range(&rgba);
    BakeMap {
        name,
        width: resolution,
        height: resolution,
        png: crate::png::encode_rgba(resolution, resolution, &rgba),
        digest: hash_bytes(&rgba),
        min,
        max,
        rgba,
    }
}

/// One channel as a grayscale run: the sample in red, alpha opaque.
fn channel_run(surface: &Surface, slot: usize, count: usize) -> (Vec<u8>, bool) {
    let field = surface.coverage_fields.get(slot);
    let mut rgba = vec![0u8; count.saturating_mul(4)];
    let mut low = 1.0f32;
    let mut high = 0.0f32;
    for index in 0..count {
        let value = field.map(|field| field.at(index)).unwrap_or(0.0);
        low = low.min(value);
        high = high.max(value);
        if let Some(pixel) = rgba.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4) {
            pixel[0] = to_u8(value);
            pixel[3] = 255;
        }
    }
    (rgba, low == high)
}

/// One packed sheet: the named channel per slot, alpha opaque when the slot
/// carries a channel.
fn sheet_run(surface: &Surface, slots: &[Option<Channel>; 4], count: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; count.saturating_mul(4)];
    for index in 0..count {
        for (slot, channel) in slots.iter().enumerate() {
            let Some(channel) = channel else { continue };
            let field = Channel::all()
                .iter()
                .position(|candidate| candidate == channel)
                .and_then(|slot| surface.coverage_fields.get(slot));
            let value = field.map(|field| field.at(index)).unwrap_or(0.0);
            if let Some(pixel) = rgba.get_mut(index.saturating_mul(4)..index.saturating_mul(4) + 4)
            {
                if let Some(out) = pixel.get_mut(slot) {
                    *out = to_u8(value);
                }
                if let Some(alpha) = pixel.get_mut(3) {
                    *alpha = 255;
                }
            }
        }
    }
    rgba
}

/// The coverage bake of one sampled surface.
pub(super) fn channel_bake(surface: &Surface, resolution: u32) -> ChannelBake {
    let count = pixels(resolution);
    let mut maps = Vec::with_capacity(COVERAGE_MAPS.len());
    let mut flat = Vec::new();
    // `COVERAGE_MAPS` is `Channel::all` order: slot n is the nth channel.
    for (slot, name) in COVERAGE_MAPS.iter().enumerate() {
        let (rgba, is_flat) = channel_run(surface, slot, count);
        if is_flat {
            flat.push(*name);
        }
        maps.push(map(name, resolution, rgba));
    }
    let sheets = SHEETS
        .iter()
        .map(|(slots, name)| map(name, resolution, sheet_run(surface, slots, count)))
        .collect();
    ChannelBake { maps, sheets, flat }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    /// The door's brick fixture, with wetness and dirt driven and the other
    /// three channels absent.
    fn brick() -> String {
        r#"{
          "material": "brick_wall",
          "base": "stone",
          "nodes": [
            {"id": 1, "kind": "gradient", "params": {"axis": "y", "from": 0.2, "to": 1.0}},
            {"id": 2, "kind": "tile", "inputs": {"a": 1}, "params": {"rows": 8, "cols": 3, "mortar": 0.08}},
            {"id": 3, "kind": "noise", "params": {"scale": 24, "seed": 1}},
            {"id": 4, "kind": "blend", "inputs": {"a": 2, "b": 3, "mask": 3}, "params": {"mode": "multiply"}},
            {"id": 5, "kind": "curve", "inputs": {"a": 2}, "params": {"exponent": 2}}
          ],
          "output": {
            "color": 2, "color_low": [0.45, 0.33, 0.28], "color_high": [0.72, 0.6, 0.48],
            "roughness": 4, "metalness": 3, "height": 2,
            "channels": {"wetness": 3, "dirt": 2}
          }
        }"#
        .to_owned()
    }

    #[test]
    fn five_channel_maps_and_two_sheets_in_order() {
        let graph = Graph::parse(&brick()).expect("parses");
        let bake = graph.bake_coverage(512).expect("bakes");
        let names: Vec<&str> = bake.maps.iter().map(|map| map.name).collect();
        assert_eq!(names, COVERAGE_MAPS);
        let sheets: Vec<&str> = bake.sheets.iter().map(|map| map.name).collect();
        assert_eq!(sheets, COVERAGE_SHEET_MAPS);
        for map in bake.maps.iter().chain(bake.sheets.iter()) {
            assert_eq!(map.width, 512);
            assert_eq!(map.rgba.len(), 512 * 512 * 4);
            assert!(map.png.starts_with(&[0x89, b'P', b'N', b'G']));
        }
    }

    #[test]
    fn a_flat_channel_is_a_warning_not_a_refusal() {
        let graph = Graph::parse(&brick()).expect("parses");
        let bake = graph.bake_coverage(512).expect("bakes");
        // Blood, strain and heat are driven by nothing: flat, and named.
        assert_eq!(
            bake.flat,
            vec!["coverage_blood", "coverage_strain", "coverage_heat"]
        );
        let dirt = bake.maps.get(1).expect("dirt");
        assert!(dirt.max[0] > dirt.min[0], "dirt is not flat");
        let blood = bake.maps.get(2).expect("blood");
        assert_eq!(blood.min[0], blood.max[0]);
    }

    #[test]
    fn the_strain_slot_rides_the_first_sheets_alpha() {
        let graph = Graph::parse(&brick()).expect("parses");
        let bake = graph.bake_coverage(512).expect("bakes");
        let sheet = bake.sheets.first().expect("first sheet");
        // Wetness and dirt vary; blood and strain are zero, alpha opaque.
        assert!(sheet.max[0] > sheet.min[0], "wetness varies");
        assert!(sheet.max[1] > sheet.min[1], "dirt varies");
        assert_eq!(sheet.max[2], 0.0);
        assert_eq!(sheet.min[3], 1.0);
    }

    #[test]
    fn the_same_graph_twice_gives_the_same_maps() {
        let a = Graph::parse(&brick()).expect("parses");
        let b = Graph::parse(&brick()).expect("parses");
        let (a, b) = (
            a.bake_coverage(512).expect("first"),
            b.bake_coverage(512).expect("second"),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn the_extra_sheet_takes_the_fifth_channel() {
        let text = brick().replace(
            "\"channels\": {\"wetness\": 3, \"dirt\": 2}",
            "\"channels\": {\"heat\": 3}",
        );
        let graph = Graph::parse(&text).expect("parses");
        let bake = graph.bake_coverage(512).expect("bakes");
        let extra = bake.sheets.get(1).expect("extra sheet");
        assert!(extra.max[0] > extra.min[0], "heat varies in red");
        assert_eq!(extra.max[1], 0.0);
        assert_eq!(extra.max[2], 0.0);
    }

    #[test]
    fn a_resolution_outside_the_bake_sizes_is_refused() {
        let graph = Graph::parse(&brick()).expect("parses");
        assert!(graph.bake_coverage(500).is_err());
    }
}
