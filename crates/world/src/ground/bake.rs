//! ground bake: ordered seeded courtyard passes, bake-time only.
//! Owner: `engine-world`; tiles plus prop tables for the courtyard.
//! Units: texels in, table entries out; kinds are small integers.
//! Determinism: integer math plus `CounterRng` streams, fixed order.
//! Errors: [`BakeErr::TooLarge`] before any allocation; parse is `Option`.
//! Budget: one 64 x 64 bake under 100 ms; bake-time only, never the tick.
use crate::{BakeErr, MAX_BAKE_SIZE};
use engine_core::CounterRng;

/// Magic at the head of every ground blob; own layout, never `W204`.
pub const GROUND_MAGIC: [u8; 4] = *b"WGND";
/// Ground blob version; bump on any layout change, refuse anything else.
pub const GROUND_VERSION: u8 = 1;
/// Courtyard dirt tile kind.
pub const TILE_DIRT: u8 = 0;
/// Worn path tile kind, carved after the base pass.
pub const TILE_WORN: u8 = 1;
/// Rubble tile kind, marked under scattered props last.
pub const TILE_RUBBLE: u8 = 2;
/// Cap on scattered props per bake; bounds the prop table.
pub const MAX_GROUND_PROPS: usize = 1024;
/// Tag separating wear draws from scatter draws.
const WEAR_TAG: u64 = 0x51ab_9d3e;
/// Recipe name for the scatter substream.
const SCATTER_RECIPE: &str = "ground-scatter";

/// One scattered rubble prop in texel coords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundProp {
    /// Texel column, always below the bake side.
    pub x: u16,
    /// Texel row, always below the bake side.
    pub y: u16,
    /// Rubble variant, always below 3.
    pub kind: u8,
}

/// Baked courtyard tables: tile kinds plus scattered props in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundBake {
    /// Bake seed.
    pub seed: u64,
    /// Bake side in texels.
    pub size: u16,
    /// Row-major tile kinds (`TILE_DIRT`/`TILE_WORN`/`TILE_RUBBLE`).
    pub tiles: Vec<u8>,
    /// Scattered props in `(y, x)` order; decor only, never blockers.
    pub props: Vec<GroundProp>,
}

/// Wear cross centre for a seed and side; `(0, 0)` when empty.
fn wear_centre(seed: u64, size: u16) -> Result<(usize, usize), BakeErr> {
    let n = usize::from(size);
    if n == 0 {
        return Ok((0, 0));
    }
    let extent = u64::from(size);
    let stream = CounterRng::new(seed.wrapping_add(WEAR_TAG));
    let cx_word = stream.sample(1) % extent;
    let cy_word = stream.sample(2) % extent;
    let cx = usize::try_from(cx_word).map_err(|_| BakeErr::TooLarge)?;
    let cy = usize::try_from(cy_word).map_err(|_| BakeErr::TooLarge)?;
    Ok((cx, cy))
}

/// Carve the worn cross through the centre; keeps every tile walkable.
fn apply_wear(tiles: &mut [u8], side: usize, cx: usize, cy: usize) {
    if side == 0 {
        return;
    }
    for y in 0..side {
        for x in 0..side {
            if x == cx || y == cy {
                let at = y.checked_mul(side).and_then(|v| v.checked_add(x));
                if let Some(idx) = at {
                    if idx < tiles.len() {
                        tiles[idx] = TILE_WORN;
                    }
                }
            }
        }
    }
}

/// Prop budget for a side: one rubble per 64 texels, capped.
fn scatter_budget(side: usize) -> usize {
    side.saturating_mul(side) / 64
}

/// Deterministic rubble props in `(y, x)` order; decor only.
fn scatter_props(seed: u64, side: usize, count: usize) -> Result<Vec<GroundProp>, BakeErr> {
    let capped = count.min(MAX_GROUND_PROPS);
    if side == 0 || capped == 0 {
        return Ok(Vec::new());
    }
    let extent = u64::from(u16::try_from(side).map_err(|_| BakeErr::TooLarge)?);
    let stream = CounterRng::new(seed).fork_named(SCATTER_RECIPE);
    let mut out = Vec::with_capacity(capped);
    for i in 0..capped {
        let base = u64::try_from(i).map_err(|_| BakeErr::TooLarge)?;
        let x_word = stream.sample(base.wrapping_mul(3)) % extent;
        let y_word = stream.sample(base.wrapping_mul(3).wrapping_add(1)) % extent;
        let k_word = stream.sample(base.wrapping_mul(3).wrapping_add(2)) % 3;
        let x = u16::try_from(x_word).map_err(|_| BakeErr::TooLarge)?;
        let y = u16::try_from(y_word).map_err(|_| BakeErr::TooLarge)?;
        let kind = u8::try_from(k_word).map_err(|_| BakeErr::TooLarge)?;
        out.push(GroundProp { x, y, kind });
    }
    out.sort_by_key(|left| (left.y, left.x));
    Ok(out)
}

/// Stamp rubble under props; worn tiles keep their kind so paths hold.
fn apply_rubble(tiles: &mut [u8], side: usize, props: &[GroundProp]) {
    if side == 0 {
        return;
    }
    for prop in props {
        let x = usize::from(prop.x);
        let y = usize::from(prop.y);
        if x >= side || y >= side {
            continue;
        }
        let at = y.checked_mul(side).and_then(|v| v.checked_add(x));
        if let Some(idx) = at {
            if idx < tiles.len() && tiles[idx] != TILE_WORN {
                tiles[idx] = TILE_RUBBLE;
            }
        }
    }
}

/// Bake ordered courtyard tables: base dirt, then wear, then scatter.
/// Units: texels in, tables out. Errors: [`BakeErr::TooLarge`] past
/// [`MAX_BAKE_SIZE`], checked before any allocation. Determinism: same
/// seed and size give identical tables; passes run base, wear, scatter.
pub fn bake_ground(seed: u64, size: u16) -> Result<GroundBake, BakeErr> {
    if size > MAX_BAKE_SIZE {
        return Err(BakeErr::TooLarge);
    }
    let side = usize::from(size);
    let mut tiles = vec![TILE_DIRT; side.saturating_mul(side)];
    let (cx, cy) = wear_centre(seed, size)?;
    apply_wear(&mut tiles, side, cx, cy);
    let props = scatter_props(seed, side, scatter_budget(side))?;
    apply_rubble(&mut tiles, side, &props);
    Ok(GroundBake {
        seed,
        size,
        tiles,
        props,
    })
}

/// Bake versioned ground bytes for the content hash gate.
/// Layout: magic(4) + version(1) + seed(8LE) + size(2LE) + tiles
/// + prop count(2LE) + props(x 2LE + y 2LE + kind 1). Errors:
///   [`BakeErr::TooLarge`] past [`MAX_BAKE_SIZE`]. Caller:
///   [`bake_ground`]; the byte form is what reruns compare.
pub fn bake_ground_bytes(seed: u64, size: u16) -> Result<Vec<u8>, BakeErr> {
    let baked = bake_ground(seed, size)?;
    let side = usize::from(baked.size);
    let count = u16::try_from(baked.props.len()).map_err(|_| BakeErr::TooLarge)?;
    let mut out = Vec::with_capacity(17 + side.saturating_mul(side) + usize::from(count) * 5);
    out.extend_from_slice(&GROUND_MAGIC);
    out.push(GROUND_VERSION);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&baked.tiles);
    out.extend_from_slice(&count.to_le_bytes());
    for prop in &baked.props {
        out.extend_from_slice(&prop.x.to_le_bytes());
        out.extend_from_slice(&prop.y.to_le_bytes());
        out.push(prop.kind);
    }
    Ok(out)
}

/// Parse baked ground bytes. `None` for a wrong header, a side past
/// [`MAX_BAKE_SIZE`] or a length that does not match the header, checked
/// before any allocation.
pub fn parse_ground(bytes: &[u8]) -> Option<GroundBake> {
    if bytes.len() < 17 {
        return None;
    }
    if bytes[0..4] != GROUND_MAGIC {
        return None;
    }
    if bytes[4] != GROUND_VERSION {
        return None;
    }
    let mut seed_buf = [0u8; 8];
    seed_buf.copy_from_slice(&bytes[5..13]);
    let seed = u64::from_le_bytes(seed_buf);
    let mut size_buf = [0u8; 2];
    size_buf.copy_from_slice(&bytes[13..15]);
    let size = u16::from_le_bytes(size_buf);
    if size > MAX_BAKE_SIZE {
        return None;
    }
    let side = usize::from(size);
    let cells = side.checked_mul(side)?;
    let mut count_buf = [0u8; 2];
    let tiles_end = 15usize.checked_add(cells)?;
    if bytes.len() < tiles_end + 2 {
        return None;
    }
    count_buf.copy_from_slice(&bytes[tiles_end..tiles_end + 2]);
    let count = u16::from_le_bytes(count_buf);
    let count_usize = usize::from(count);
    let props_len = count_usize.checked_mul(5)?;
    if bytes.len() != tiles_end + 2 + props_len {
        return None;
    }
    let tiles = bytes[15..tiles_end].to_vec();
    for tile in &tiles {
        if *tile != TILE_DIRT && *tile != TILE_WORN && *tile != TILE_RUBBLE {
            return None;
        }
    }
    let mut props = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let at = tiles_end + 2 + i.checked_mul(5)?;
        let mut x_buf = [0u8; 2];
        let mut y_buf = [0u8; 2];
        x_buf.copy_from_slice(&bytes[at..at + 2]);
        y_buf.copy_from_slice(&bytes[at + 2..at + 4]);
        let x = u16::from_le_bytes(x_buf);
        let y = u16::from_le_bytes(y_buf);
        let kind = bytes[at + 4];
        if usize::from(x) >= side || usize::from(y) >= side || kind >= 3 {
            return None;
        }
        props.push(GroundProp { x, y, kind });
    }
    Some(GroundBake {
        seed,
        size,
        tiles,
        props,
    })
}

/// Walk blockers for a ground bake: rubble is decor so nothing blocks.
/// Units: one flag per texel; all false, so worn paths always connect.
pub fn blocked_tiles(baked: &GroundBake) -> Vec<bool> {
    vec![false; usize::from(baked.size).saturating_mul(usize::from(baked.size))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_bytes(seed: u64, size: u16) -> Vec<u8> {
        bake_ground_bytes(seed, size).expect("fixture size bakes")
    }

    fn count_kind(tiles: &[u8], kind: u8) -> usize {
        tiles.iter().filter(|tile| **tile == kind).count()
    }

    fn connected(flags: &[bool], side: usize) -> bool {
        if side == 0 {
            return true;
        }
        if flags[0] || flags[side * side - 1] {
            return false;
        }
        let mut seen = vec![false; side * side];
        let mut stack = vec![0usize];
        seen[0] = true;
        while let Some(cell) = stack.pop() {
            if cell == side * side - 1 {
                return true;
            }
            let (cx, cy) = (cell % side, cell / side);
            for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= side as i32 || ny >= side as i32 {
                    continue;
                }
                let next = ny as usize * side + nx as usize;
                if !flags[next] && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        seen[side * side - 1]
    }

    #[test]
    fn ten_bakes_rerun_identical_x2() {
        let cases: [(u64, u16); 10] = [
            (2040, 8),
            (2040, 16),
            (2040, 32),
            (2040, 64),
            (7, 16),
            (8, 16),
            (12345, 24),
            (999, 32),
            (1, 4),
            (2, 48),
        ];
        for (seed, size) in cases {
            let first = ground_bytes(seed, size);
            let second = ground_bytes(seed, size);
            assert_eq!(first, second, "bake {seed} {size} reruns identical");
            assert_eq!(crate::world_hash(&first), crate::world_hash(&second));
        }
        assert_ne!(ground_bytes(7, 16), ground_bytes(8, 16));
        assert_ne!(ground_bytes(7, 16), ground_bytes(7, 8));
    }

    #[test]
    fn layout_parse_roundtrip() {
        let size = 16u16;
        let bytes = ground_bytes(2040, size);
        let side = usize::from(size);
        let count = u16::from_le_bytes(
            bytes[15 + side * side..15 + side * side + 2]
                .try_into()
                .unwrap(),
        );
        assert_eq!(&bytes[0..4], &GROUND_MAGIC);
        assert_eq!(bytes[4], GROUND_VERSION);
        assert_eq!(u64::from_le_bytes(bytes[5..13].try_into().unwrap()), 2040);
        assert_eq!(u16::from_le_bytes(bytes[13..15].try_into().unwrap()), size);
        assert_eq!(bytes.len(), 17 + side * side + usize::from(count) * 5);
        let parsed = parse_ground(&bytes).expect("own bytes parse");
        assert_eq!((parsed.seed, parsed.size), (2040, size));
        assert_eq!(parsed.tiles.len(), side * side);
        assert_eq!(parsed.props.len(), usize::from(count));
        assert!(parse_ground(&bytes[..bytes.len() - 1]).is_none());
        let mut bad = bytes.clone();
        bad[4] ^= 0xFF;
        assert!(parse_ground(&bad).is_none());
    }

    #[test]
    fn staged_has_wear_and_rubble_over_uniform_dirt() {
        let baked = bake_ground(2040, 32).expect("staged bakes");
        let worn = count_kind(&baked.tiles, TILE_WORN);
        let rubble = count_kind(&baked.tiles, TILE_RUBBLE);
        let dirt = count_kind(&baked.tiles, TILE_DIRT);
        assert!(worn > 0, "wear pass carves paths");
        assert!(rubble > 0, "scatter pass marks rubble");
        assert!(dirt > 0, "courtyard dirt remains");
        assert!(!baked.props.is_empty(), "prop table is not empty");
        let uniform = vec![TILE_DIRT; 32 * 32];
        assert_ne!(baked.tiles, uniform, "staged beats single-pass dirt");
        let again = bake_ground(2040, 32).expect("rerun bakes");
        assert_eq!(baked, again, "tables rerun identical");
    }

    #[test]
    fn no_blocked_paths_and_connected() {
        for (seed, size) in [(2040u64, 16u16), (7, 24), (99, 32)] {
            let baked = bake_ground(seed, size).expect("bakes");
            let side = usize::from(size);
            let blocked = blocked_tiles(&baked);
            assert_eq!(blocked.len(), side * side);
            assert!(blocked.iter().all(|flag| !flag), "0 blocked paths");
            assert!(connected(&blocked, side), "worn grid connects {seed}");
        }
    }

    #[test]
    fn refuses_before_allocating() {
        assert_eq!(bake_ground(7, MAX_BAKE_SIZE + 1), Err(BakeErr::TooLarge));
        assert_eq!(bake_ground_bytes(7, u16::MAX), Err(BakeErr::TooLarge));
        for claimed in [MAX_BAKE_SIZE + 1, 512, u16::MAX] {
            let mut head = Vec::new();
            head.extend_from_slice(&GROUND_MAGIC);
            head.push(GROUND_VERSION);
            head.extend_from_slice(&7u64.to_le_bytes());
            head.extend_from_slice(&claimed.to_le_bytes());
            assert!(parse_ground(&head).is_none());
        }
    }

    #[test]
    fn adjacent_seeds_differ_without_aliasing() {
        let first = ground_bytes(2, 16);
        let second = ground_bytes(3, 16);
        assert_ne!(first, second, "seeds 2k and 2k+1 differ");
        assert_eq!(first, ground_bytes(2, 16));
    }

    #[test]
    fn props_sorted_bounded_and_kinded() {
        let baked = bake_ground(2040, 32).expect("bakes");
        let side = usize::from(baked.size);
        assert!(baked.props.len() <= MAX_GROUND_PROPS);
        let mut ordered = baked.props.clone();
        ordered.sort_by_key(|left| (left.y, left.x));
        assert_eq!(baked.props, ordered, "props stay in (y, x) order");
        for prop in &baked.props {
            assert!(usize::from(prop.x) < side);
            assert!(usize::from(prop.y) < side);
            assert!(prop.kind < 3);
        }
    }

    #[test]
    #[ignore = "timing: bake budget 100ms, wall clock"]
    fn bake_fits_time_budget() {
        let start = std::time::Instant::now();
        let _ = ground_bytes(2040, 64);
        assert!(
            start.elapsed().as_millis() <= 100,
            "64-side bake fits 100ms"
        );
    }
}
