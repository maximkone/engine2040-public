//! manifest: versioned map assembly bytes - seed, cell size, per-cell asset lists -
//! plus a provenance gate over the EVIDENCE ledger rows.
//!
//! Consumer: the map assembler (one manifest per map, handed to the cell stream)
//! and the content audit, which walks every cell asset before a build. Data
//! owner: `engine-world`; the asset paths belong to `content/maps/` and the
//! ledger belongs to `docs/EVIDENCE.md`, this module only reads both. Units:
//! bytes in and out, cells and asset paths as counts. Determinism: cells are
//! kept sorted by coordinate, assets keep their authored order, every number is
//! little-endian, so the same manifest encodes the same bytes on every PC.
//! Errors: typed [`ManifestError`]; corrupt bytes name their check, and the gate
//! names the refusing cell and the asset path.
//!
//! The provenance gate is a completeness check, not a permission check: a
//! ledger row documents an asset; it never grants or blocks a port. A row
//! proves an asset when one of its table cells equals the path or is a parent
//! directory of it (a trailing `/` is ignored, backticks are dropped). The
//! ledger text is passed in, so `engine-world` stays std-only and never reads
//! a file from the tick.
//!
//! Budget: `to_bytes` is linear in cells times assets; the gate is one ledger
//! scan plus one lookup per asset, bake-time only.

use crate::cells::{CellCoord, CELL_MAX_SIZE, CELL_MIN_SIZE};
use std::fmt;

/// Magic at the head of every manifest; its own layout, never a `W204` bake bump.
pub const MANIFEST_MAGIC: [u8; 4] = *b"WCM1";
/// Manifest layout version; bump on any layout change, refuse anything else.
pub const MANIFEST_VERSION: u8 = 1;
/// Header bytes: magic(4) + version(1) + seed(8) + cell size(2) + cell count(4).
pub const MANIFEST_HEADER_LEN: usize = 19;
/// Most cells one manifest names; parity with the cell registry admission cap.
pub const MAX_MANIFEST_CELLS: usize = 4096;
/// Most assets one cell may name.
pub const MAX_CELL_ASSETS: usize = 64;
/// Longest asset path in bytes; paths are ASCII, so bytes equal chars.
pub const MAX_ASSET_CHARS: usize = 96;
/// Largest blob `parse` accepts; a full manifest stays far below it.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;

/// One cell of a map assembly: its address plus the assets it draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellEntry {
    coord: CellCoord,
    assets: Vec<String>,
}

impl CellEntry {
    /// Cell address the stream uses.
    pub fn coord(&self) -> CellCoord {
        self.coord
    }

    /// Asset paths in authored order; never empty paths.
    pub fn assets(&self) -> &[String] {
        &self.assets
    }
}

/// A map assembly: one seed and cell size, plus every cell and its assets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    seed: u64,
    cell_size: u16,
    cells: Vec<CellEntry>,
}

impl Manifest {
    /// Empty manifest for one seed and one cell side in texels.
    /// Errors: [`ManifestError::BadSize`] outside the cell size range.
    pub fn new(seed: u64, cell_size: u16) -> Result<Self, ManifestError> {
        if !(CELL_MIN_SIZE..=CELL_MAX_SIZE).contains(&cell_size) {
            return Err(ManifestError::BadSize { got: cell_size });
        }
        Ok(Self {
            seed,
            cell_size,
            cells: Vec::new(),
        })
    }

    /// Bake seed shared by every cell.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Cell side in texels shared by every cell.
    pub fn cell_size(&self) -> u16 {
        self.cell_size
    }

    /// Cells in coordinate order (x-major, then z), never two of one coord.
    pub fn cells(&self) -> &[CellEntry] {
        &self.cells
    }

    /// One cell by address, when the manifest names it.
    pub fn cell(&self, coord: CellCoord) -> Option<&CellEntry> {
        self.cells
            .binary_search_by_key(&coord, |entry| entry.coord)
            .ok()
            .map(|at| &self.cells[at])
    }

    /// Add one cell with its assets; keeps coordinate order.
    /// Errors: [`ManifestError::DuplicateCell`], [`ManifestError::TooManyCells`],
    /// [`ManifestError::TooManyAssets`], [`ManifestError::DuplicateAsset`] and
    /// [`ManifestError::BadAsset`]. Nothing is stored on refusal.
    pub fn push_cell(&mut self, coord: CellCoord, assets: &[&str]) -> Result<(), ManifestError> {
        if self.cells.len() >= MAX_MANIFEST_CELLS {
            return Err(ManifestError::TooManyCells {
                limit: MAX_MANIFEST_CELLS,
            });
        }
        if assets.len() > MAX_CELL_ASSETS {
            return Err(ManifestError::TooManyAssets {
                limit: MAX_CELL_ASSETS,
            });
        }
        let mut list: Vec<String> = Vec::with_capacity(assets.len());
        for path in assets {
            check_asset(path)?;
            if list.iter().any(|seen| seen.as_str() == *path) {
                return Err(ManifestError::DuplicateAsset {
                    path: (*path).to_string(),
                });
            }
            list.push((*path).to_string());
        }
        match self.cells.binary_search_by_key(&coord, |entry| entry.coord) {
            Ok(_) => Err(ManifestError::DuplicateCell { coord }),
            Err(at) => {
                self.cells.insert(
                    at,
                    CellEntry {
                        coord,
                        assets: list,
                    },
                );
                Ok(())
            }
        }
    }

    /// Canonical bytes: magic, version, seed, cell size, cell count, then each
    /// cell as coord + asset count + length-prefixed paths. Same manifest,
    /// same bytes, twice.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MANIFEST_HEADER_LEN);
        out.extend_from_slice(&MANIFEST_MAGIC);
        out.push(MANIFEST_VERSION);
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.cell_size.to_le_bytes());
        let cells = u32::try_from(self.cells.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&cells.to_le_bytes());
        for entry in &self.cells {
            out.extend_from_slice(&entry.coord.x.to_le_bytes());
            out.extend_from_slice(&entry.coord.z.to_le_bytes());
            let count = u16::try_from(entry.assets.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&count.to_le_bytes());
            for path in &entry.assets {
                let len = u16::try_from(path.len()).unwrap_or(u16::MAX);
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(path.as_bytes());
            }
        }
        out
    }

    /// Refuse the manifest when one cell asset has no ledger row.
    /// Rows come from [`ledger_rows`]; the first gap in coordinate and asset
    /// order is named. Errors: [`ManifestError::Unprovenanced`].
    pub fn gate(&self, ledger: &str) -> Result<(), ManifestError> {
        let rows = ledger_rows(ledger);
        for entry in &self.cells {
            for asset in &entry.assets {
                if !rows.iter().any(|row| row_covers(row, asset)) {
                    return Err(ManifestError::Unprovenanced {
                        coord: entry.coord,
                        asset: asset.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Decode a manifest. Refuses other magic or version, a cell size outside the
/// range, unsorted or duplicate cells, bad paths, trailing bytes and any short
/// or long read. Errors name the failing check.
pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::Corrupt("size"));
    }
    let mut at = 0;
    if fixed::<4>(bytes, &mut at, "magic")? != MANIFEST_MAGIC {
        return Err(ManifestError::Corrupt("magic"));
    }
    if fixed::<1>(bytes, &mut at, "version")?[0] != MANIFEST_VERSION {
        return Err(ManifestError::Corrupt("version"));
    }
    let seed = u64::from_le_bytes(fixed::<8>(bytes, &mut at, "seed")?);
    let cell_size = u16::from_le_bytes(fixed::<2>(bytes, &mut at, "cell size")?);
    let count = usize::try_from(u32::from_le_bytes(fixed::<4>(
        bytes,
        &mut at,
        "cell count",
    )?))
    .map_err(|_| ManifestError::Corrupt("cell count"))?;
    if count > MAX_MANIFEST_CELLS {
        return Err(ManifestError::Corrupt("cell count"));
    }
    let mut manifest = Manifest::new(seed, cell_size)?;
    let mut previous = None;
    for _ in 0..count {
        let x = i32::from_le_bytes(fixed::<4>(bytes, &mut at, "cell x")?);
        let z = i32::from_le_bytes(fixed::<4>(bytes, &mut at, "cell z")?);
        let coord = CellCoord::new(x, z).map_err(|_| ManifestError::BadCoord)?;
        if previous.is_some_and(|last| coord <= last) {
            return Err(ManifestError::Corrupt("cell order"));
        }
        previous = Some(coord);
        let asset_count = usize::from(u16::from_le_bytes(fixed::<2>(
            bytes,
            &mut at,
            "asset count",
        )?));
        if asset_count > MAX_CELL_ASSETS {
            return Err(ManifestError::Corrupt("asset count"));
        }
        let mut assets = Vec::with_capacity(asset_count);
        for _ in 0..asset_count {
            let len = usize::from(u16::from_le_bytes(fixed::<2>(bytes, &mut at, "asset len")?));
            let raw = take(bytes, &mut at, len, "asset")?;
            let path =
                std::str::from_utf8(raw).map_err(|_| ManifestError::Corrupt("asset utf8"))?;
            assets.push(path.to_string());
        }
        let refs: Vec<&str> = assets.iter().map(String::as_str).collect();
        manifest.push_cell(coord, &refs)?;
    }
    if at != bytes.len() {
        return Err(ManifestError::Corrupt("trailing bytes"));
    }
    Ok(manifest)
}

/// Table rows of an EVIDENCE ledger: every non-empty cell of every `|` line,
/// with backticks dropped and separator cells skipped. Public so a caller can
/// show what was read; the gate builds the same list.
pub fn ledger_rows(ledger: &str) -> Vec<&str> {
    let mut rows = Vec::new();
    for line in ledger.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        for cell in line.split('|') {
            let cell = unquote(cell.trim());
            if cell.is_empty() || cell.bytes().all(|b| b == b'-' || b == b':') {
                continue;
            }
            rows.push(cell);
        }
    }
    rows
}

/// Why a manifest was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// Cell side outside the cell range.
    BadSize {
        /// Side offered in texels.
        got: u16,
    },
    /// Cell coordinate past the crate's coordinate bound.
    BadCoord,
    /// The same coordinate was pushed twice.
    DuplicateCell {
        /// Repeated address.
        coord: CellCoord,
    },
    /// More cells than one manifest names.
    TooManyCells {
        /// Cell cap.
        limit: usize,
    },
    /// More assets than one cell names.
    TooManyAssets {
        /// Asset cap per cell.
        limit: usize,
    },
    /// The same asset path appears twice in one cell.
    DuplicateAsset {
        /// Repeated path.
        path: String,
    },
    /// Asset path empty, too long, absolute, or with a bad segment.
    BadAsset {
        /// Path offered.
        path: String,
    },
    /// No ledger row proves this asset, so its whole cell is refused.
    Unprovenanced {
        /// Cell that carries the asset.
        coord: CellCoord,
        /// Asset path with no row.
        asset: String,
    },
    /// Bytes failed a structural check; the static names it.
    Corrupt(&'static str),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize { got } => write!(
                f,
                "manifest cell size {got} outside {CELL_MIN_SIZE}..={CELL_MAX_SIZE}"
            ),
            Self::BadCoord => write!(f, "manifest cell coord outside the crate bound"),
            Self::DuplicateCell { coord } => write!(f, "manifest duplicate {coord}"),
            Self::TooManyCells { limit } => write!(f, "manifest over {limit} cells"),
            Self::TooManyAssets { limit } => write!(f, "manifest over {limit} assets in a cell"),
            Self::DuplicateAsset { path } => write!(f, "manifest duplicate asset {path}"),
            Self::BadAsset { path } => write!(f, "manifest bad asset path {path}"),
            Self::Unprovenanced { coord, asset } => {
                write!(f, "manifest {coord} asset {asset} has no EVIDENCE.md row")
            }
            Self::Corrupt(check) => write!(f, "manifest corrupt: {check}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// A path is relative ASCII with no empty, `.` or `..` segment; `|` stays out
/// so a path can never pose as a ledger column.
fn check_asset(path: &str) -> Result<(), ManifestError> {
    let bad = path.is_empty()
        || path.len() > MAX_ASSET_CHARS
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('|')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        || !path.bytes().all(|b| b.is_ascii_graphic());
    if bad {
        return Err(ManifestError::BadAsset {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// One ledger row cell proves the exact path or anything under it; a trailing
/// `/` is ignored and the boundary must be a real path segment.
fn row_covers(row: &str, asset: &str) -> bool {
    let base = row.trim_end_matches('/');
    if base.is_empty() {
        return false;
    }
    asset == base
        || asset
            .strip_prefix(base)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Drop one pair of surrounding backticks; other cell marks stay.
fn unquote(cell: &str) -> &str {
    cell.strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
        .unwrap_or(cell)
}

/// Fixed-width field; a short read names the field.
fn fixed<const N: usize>(
    bytes: &[u8],
    at: &mut usize,
    check: &'static str,
) -> Result<[u8; N], ManifestError> {
    let slice = take(bytes, at, N, check)?;
    slice.try_into().map_err(|_| ManifestError::Corrupt(check))
}

/// Borrow `len` bytes from `at`, advancing it; a short read names the field.
fn take<'a>(
    bytes: &'a [u8],
    at: &mut usize,
    len: usize,
    check: &'static str,
) -> Result<&'a [u8], ManifestError> {
    let end = at.checked_add(len).ok_or(ManifestError::Corrupt(check))?;
    let slice = bytes.get(*at..end).ok_or(ManifestError::Corrupt(check))?;
    *at = end;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::hash_bytes;

    const LUDUS_SEED: u64 = 20_260_925;
    const LUDUS_SIZE: u16 = 16;
    const LUDUS_LEDGER: &str = include_str!("../../../docs/EVIDENCE.md");

    const LUDUS_ASSETS: [&str; 13] = [
        "content/maps/src/yard_floor.gltf",
        "content/maps/textures/ground_albedo.png",
        "content/maps/hdri/maps_day.hdr",
        "content/maps/src/barracks.gltf",
        "content/maps/textures/plaster_albedo.png",
        "content/maps/textures/flat_normal.png",
        "content/maps/src/blacksmith.gltf",
        "content/maps/textures/stone_albedo.png",
        "content/maps/src/training_post.gltf",
        "content/maps/src/notice_board.gltf",
        "content/maps/src/weapon_rack.gltf",
        "content/maps/src/street_wall.gltf",
        "content/maps/textures/timber_albedo.png",
    ];

    fn coord(x: i32, z: i32) -> CellCoord {
        CellCoord::new(x, z).expect("test coordinate within the bound")
    }

    fn ludus_manifest() -> Manifest {
        let mut manifest = Manifest::new(LUDUS_SEED, LUDUS_SIZE).expect("test cell size is legal");
        let cells: [((i32, i32), &[&str]); 4] = [
            ((0, 0), &[LUDUS_ASSETS[0], LUDUS_ASSETS[1], LUDUS_ASSETS[2]]),
            ((1, 0), &[LUDUS_ASSETS[3], LUDUS_ASSETS[4], LUDUS_ASSETS[5]]),
            ((0, 1), &[LUDUS_ASSETS[6], LUDUS_ASSETS[7], LUDUS_ASSETS[8]]),
            (
                (1, 1),
                &[
                    LUDUS_ASSETS[9],
                    LUDUS_ASSETS[10],
                    LUDUS_ASSETS[11],
                    LUDUS_ASSETS[12],
                ],
            ),
        ];
        for ((x, z), assets) in cells {
            manifest
                .push_cell(coord(x, z), assets)
                .expect("test cell is well formed");
        }
        manifest
    }

    #[test]
    fn ludus_cells_round_trip_and_bytes_identical_twice() {
        let manifest = ludus_manifest();
        let bytes = manifest.to_bytes();
        let again = manifest.to_bytes();
        assert_eq!(bytes, again);
        assert_eq!(hash_bytes(&bytes), hash_bytes(&again));
        let decoded = parse(&bytes).expect("ludus manifest parses");
        assert_eq!(decoded, manifest);
        assert_eq!(decoded.to_bytes(), bytes);
        assert_eq!(decoded.cell_size(), 16);
        assert_eq!(decoded.seed(), LUDUS_SEED);
        assert_eq!(decoded.cells().len(), 4);
        let last: Vec<String> = LUDUS_ASSETS[9..]
            .iter()
            .map(|path| (*path).to_string())
            .collect();
        assert_eq!(
            decoded.cell(coord(1, 1)).map(CellEntry::assets),
            Some(last.as_slice())
        );
    }

    #[test]
    fn push_order_never_changes_the_bytes() {
        let mut forward = Manifest::new(7, 16).expect("test cell size is legal");
        forward
            .push_cell(coord(0, 0), &[LUDUS_ASSETS[0]])
            .expect("valid cell");
        forward
            .push_cell(coord(1, 0), &[LUDUS_ASSETS[3]])
            .expect("valid cell");
        let mut backward = Manifest::new(7, 16).expect("test cell size is legal");
        backward
            .push_cell(coord(1, 0), &[LUDUS_ASSETS[3]])
            .expect("valid cell");
        backward
            .push_cell(coord(0, 0), &[LUDUS_ASSETS[0]])
            .expect("valid cell");
        assert_eq!(forward.to_bytes(), backward.to_bytes());
        assert_eq!(forward, backward);
        assert_eq!(forward.cells()[0].coord(), coord(0, 0));
        assert_eq!(forward.cells()[1].coord(), coord(1, 0));
    }

    #[test]
    fn one_unprovenanced_asset_refuses_the_cell_and_names_its_path() {
        let ledger = LUDUS_LEDGER;
        ludus_manifest()
            .gate(ledger)
            .expect("every ludus asset has a row");
        let bad = "content/incoming/unprovenanced_prop.glb";
        let mut refused = ludus_manifest();
        refused
            .push_cell(coord(2, 0), &[bad])
            .expect("path is well formed");
        let err = refused.gate(ledger).expect_err("no row proves the prop");
        assert_eq!(
            err,
            ManifestError::Unprovenanced {
                coord: coord(2, 0),
                asset: bad.to_string(),
            }
        );
        let text = err.to_string();
        assert!(text.contains(bad), "error names the asset path: {text}");
        assert!(text.contains("cell(2, 0)"), "error names the cell: {text}");
        // The good cells alone still pass: one bad cell refuses the manifest,
        // not the other cells' rows.
        ludus_manifest()
            .gate(ledger)
            .expect("good cells stay gated");
    }

    #[test]
    fn ledger_rows_prove_exact_paths_and_parent_directories_only() {
        let ledger = "\
| Asset | License |
|---|---|
| `content/maps/` | CC0-1.0 |
| `content/incoming/studio_small_09_1k.hdr` | CC0-1.0 |
";
        let rows = ledger_rows(ledger);
        assert!(rows.contains(&"content/maps/"));
        assert!(row_covers(
            "content/maps/",
            "content/maps/src/yard_floor.gltf"
        ));
        assert!(row_covers(
            "content/incoming/studio_small_09_1k.hdr",
            "content/incoming/studio_small_09_1k.hdr"
        ));
        assert!(!row_covers(
            "content/maps",
            "content/mapsx/src/yard_floor.gltf"
        ));
        assert!(!row_covers(
            "content/incoming/studio_small_09_1k.hdr",
            "content/incoming/mystery.glb"
        ));
        let mut manifest = Manifest::new(1, 16).expect("test cell size is legal");
        manifest
            .push_cell(coord(0, 0), &["content/incoming/mystery.glb"])
            .expect("path is well formed");
        assert_eq!(
            manifest.gate(ledger),
            Err(ManifestError::Unprovenanced {
                coord: coord(0, 0),
                asset: "content/incoming/mystery.glb".to_string(),
            })
        );
    }

    #[test]
    fn corrupt_bytes_and_bad_inputs_name_their_check() {
        let bytes = ludus_manifest().to_bytes();
        let mut flipped = bytes.clone();
        flipped[0] = b'X';
        assert_eq!(parse(&flipped), Err(ManifestError::Corrupt("magic")));
        let mut flipped = bytes.clone();
        flipped[4] = MANIFEST_VERSION + 1;
        assert_eq!(parse(&flipped), Err(ManifestError::Corrupt("version")));
        assert!(matches!(
            parse(&bytes[..MANIFEST_HEADER_LEN - 1]),
            Err(ManifestError::Corrupt(_))
        ));
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(parse(&long), Err(ManifestError::Corrupt("trailing bytes")));
        assert_eq!(
            Manifest::new(7, CELL_MIN_SIZE - 1),
            Err(ManifestError::BadSize {
                got: CELL_MIN_SIZE - 1
            })
        );
        let mut manifest = Manifest::new(7, 16).expect("test cell size is legal");
        assert_eq!(
            manifest.push_cell(coord(0, 0), &["../escape.gltf"]),
            Err(ManifestError::BadAsset {
                path: "../escape.gltf".to_string()
            })
        );
        manifest
            .push_cell(coord(0, 0), &[LUDUS_ASSETS[0]])
            .expect("valid cell");
        assert_eq!(
            manifest.push_cell(coord(0, 0), &[LUDUS_ASSETS[1]]),
            Err(ManifestError::DuplicateCell { coord: coord(0, 0) })
        );
        assert_eq!(
            manifest.push_cell(coord(1, 0), &[LUDUS_ASSETS[1], LUDUS_ASSETS[1]]),
            Err(ManifestError::DuplicateAsset {
                path: LUDUS_ASSETS[1].to_string()
            })
        );
        let many = vec!["content/maps/src/prop.gltf"; MAX_CELL_ASSETS + 1];
        assert_eq!(
            manifest.push_cell(coord(1, 0), &many),
            Err(ManifestError::TooManyAssets {
                limit: MAX_CELL_ASSETS
            })
        );
    }
}
