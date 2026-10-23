//! The golden bless flow: every staged still against its filed golden, with
//! two tiers and a known-failure ratchet.
//!
//! Consumer: `tools/q1-check.mjs` runs the bit-exact leg over the Q1 capture
//! tree, and a delivery host runs [`run`] after a capture pass to bless what
//! it means to keep. Data owner: this module owns the bless artifacts
//! (`.new.png`, `.diff.png`, `.old.png`), the per-adapter pooled bar ([`BARS`])
//! and the known-failure ratchet. Pixels never enter a simulation hash.
//!
//! Two tiers, after the shapes the look plans name:
//! - [`Tier::BitExact`] is the software adapter's promise — the same draw
//!   order twice gives the same bytes — so a still passes only when its PNG
//!   bytes equal the golden's. It is the tier llvmpipe and swiftshader run at.
//! - [`Tier::Perceptual`] scores each still with [`super::metric::flip`] and
//!   gates the *pooled* p95 and p99 against the adapter's row in [`BARS`], so
//!   a real GPU is held to its own tolerance instead of another device's.
//!
//! A bless run never edits a golden in place. A still that changed leaves
//! `{still}.new.png` (the candidate), `{still}.diff.png` (the per-pixel
//! difference as an image) and `{still}.old.png` (a copy of the golden it is
//! about to replace) next to the golden, and only [`RunConfig::bless`] moves
//! the candidate into `{still}.png`. The `.old` copy survives the bless, so
//! the frame that was filed stays reviewable after the new one is taken.
//!
//! Refusals, not silence: a still with no golden is
//! [`GoldenError::MissingGolden`] unless the run blesses (a bless seeds a
//! first golden), and a still filed as a known failure that no longer differs
//! is [`GoldenError::StaleKnownFailure`] — the ratchet that keeps the table
//! short.
//!
//! Units: FLIP visibility in ten-thousandths, where `super::metric::FLIP_ONE`
//! is the full black/white difference; digests are the FNV-1a hex of
//! [`crate::frame_check::digest_hex`] over decoded RGBA pixels, so a golden is
//! pinned by its pixels and not by an encoder's bytes. Determinism: pure over
//! the bytes handed in, integer-only pooling; the same two directories twice
//! give the same report or the same refusal. Bounds: [`MAX_STILLS`] stills per
//! run, [`super::judge::MAX_PNG_BYTES`] bytes per PNG, [`MAX_NAME_LEN`]
//! characters per still name. Errors: every [`GoldenError`] variant names the
//! still it refused; nothing here panics.

use std::fmt;
use std::path::Path;

use super::judge::MAX_PNG_BYTES;
use super::metric::{flip, Frame, Region};
use crate::frame_check::digest_hex;

/// Most stills one bless run accepts.
pub const MAX_STILLS: usize = 256;
/// Longest still name this flow accepts, characters.
pub const MAX_NAME_LEN: usize = 96;
/// The golden file itself: `{still}.png`.
pub const GOLDEN_SUFFIX: &str = ".png";
/// The candidate a changed still leaves behind.
pub const NEW_SUFFIX: &str = ".new.png";
/// The per-pixel difference image a changed still leaves behind.
pub const DIFF_SUFFIX: &str = ".diff.png";
/// A copy of the golden the candidate is about to replace.
pub const OLD_SUFFIX: &str = ".old.png";

/// What a run demands of a still against its golden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// The PNG bytes must match: the software adapter's promise.
    BitExact,
    /// The pixels must match, or the pooled FLIP must stay inside the
    /// adapter's bar.
    Perceptual,
}

impl Tier {
    /// The tier's name as it reads in a report line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BitExact => "bit-exact",
            Self::Perceptual => "perceptual",
        }
    }
}

/// One adapter's pooled tolerance, in ten-thousandths of FLIP visibility.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AdapterBar {
    /// Adapter the row is filed for, or `"default"` for the fallback.
    pub adapter: &'static str,
    /// Highest p95 the run may pool.
    pub p95_x10000: u32,
    /// Highest p99 the run may pool.
    pub p99_x10000: u32,
}

/// The per-adapter table. The software legs promise the same bytes twice, so
/// they carry no slop; the fallback row holds FLIP's own 2% and 4% visibility
/// thresholds for a real GPU until its adapter gets a row of its own.
pub const BARS: [AdapterBar; 3] = [
    AdapterBar {
        adapter: "llvmpipe",
        p95_x10000: 0,
        p99_x10000: 0,
    },
    AdapterBar {
        adapter: "swiftshader",
        p95_x10000: 0,
        p99_x10000: 0,
    },
    AdapterBar {
        adapter: "default",
        p95_x10000: 200,
        p99_x10000: 400,
    },
];

/// The bar `adapter` runs under: its own row, else the table's last row.
#[must_use]
pub fn bar_for(adapter: &str) -> AdapterBar {
    BARS.iter()
        .find(|bar| bar.adapter == adapter)
        .copied()
        .unwrap_or_else(|| BARS[BARS.len() - 1])
}

/// Why a bless run could not be scored or completed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GoldenError {
    /// A still has no golden and the run does not bless.
    MissingGolden {
        /// Still with nothing filed against it.
        still: String,
    },
    /// A still filed as a known failure now matches its golden: the table row
    /// is stale.
    StaleKnownFailure {
        /// Still whose known-failure row no longer buys anything.
        still: String,
    },
    /// The still and its golden are drawn on different canvases.
    CanvasMismatch {
        /// Still that changed size.
        still: String,
        /// Width and height the candidate holds.
        capture: (u32, u32),
        /// Width and height the golden holds.
        golden: (u32, u32),
    },
    /// The run offers more stills than the bound.
    TooManyStills {
        /// Stills offered.
        count: usize,
    },
    /// A still name is longer than [`MAX_NAME_LEN`] or has no `.png` tail.
    BadName {
        /// Name as read from the directory.
        name: String,
    },
    /// A PNG is over the capture bound.
    PngTooLarge {
        /// Still it was read for.
        still: String,
        /// Bytes on disk.
        bytes: usize,
    },
    /// A PNG did not decode as a frame.
    Undecodable {
        /// Still it was read for.
        still: String,
        /// The decoder's own words.
        reason: String,
    },
    /// The metric refused the pair.
    Metric {
        /// Still the pair belongs to.
        still: String,
        /// The metric's own words.
        reason: String,
    },
    /// The difference image did not encode.
    Encode {
        /// Still it was written for.
        still: String,
        /// The encoder's own words.
        reason: String,
    },
    /// A directory or file could not be read or written.
    Io {
        /// Path (for diagnostics).
        path: String,
        /// IO failure kind.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingGolden { still } => write!(
                f,
                "{still} has no golden filed; run with bless to seed it, or \
                 file the frame the last green build drew"
            ),
            Self::StaleKnownFailure { still } => write!(
                f,
                "{still} is filed as a known failure but now matches its \
                 golden; delete its row from the known-failure table"
            ),
            Self::CanvasMismatch {
                still,
                capture,
                golden,
            } => write!(
                f,
                "{still} draws {}x{} but its golden is {}x{}; \
                 re-file the golden at the run's canvas",
                capture.0, capture.1, golden.0, golden.1
            ),
            Self::TooManyStills { count } => write!(
                f,
                "{count} stills offered; at most {MAX_STILLS} per bless run"
            ),
            Self::BadName { name } => write!(
                f,
                "still name {name} is not accepted: a {GOLDEN_SUFFIX} file of \
                 at most {MAX_NAME_LEN} characters"
            ),
            Self::PngTooLarge { still, bytes } => write!(
                f,
                "{still} is {bytes} bytes; at most {MAX_PNG_BYTES} per PNG"
            ),
            Self::Undecodable { still, reason } => {
                write!(f, "{still} does not decode as a PNG frame: {reason}")
            }
            Self::Metric { still, reason } => {
                write!(
                    f,
                    "{still} could not be scored against its golden: {reason}"
                )
            }
            Self::Encode { still, reason } => {
                write!(f, "{still} difference image did not encode: {reason}")
            }
            Self::Io { path, kind } => write!(f, "cannot read or write {path} ({kind:?})"),
        }
    }
}

impl std::error::Error for GoldenError {}

/// One still measured against its golden.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StillScore {
    /// Still name, the capture file's own name.
    pub still: String,
    /// True when the two PNG byte streams are identical.
    pub bytes_equal: bool,
    /// True when the two decoded frames are identical.
    pub pixels_equal: bool,
    /// FLIP mean visibility over the frame, ten-thousandths.
    pub flip_x10000: u32,
    /// FLIP worst pixel in the same units.
    pub flip_max_x10000: u32,
    /// FNV-1a hex over the candidate's decoded pixels.
    pub capture_fnv: String,
    /// FNV-1a hex over the golden's decoded pixels.
    pub golden_fnv: String,
}

impl StillScore {
    /// True when this still met `tier`'s own equality test.
    #[must_use]
    pub fn met(&self, tier: Tier) -> bool {
        match tier {
            Tier::BitExact => self.bytes_equal,
            Tier::Perceptual => self.pixels_equal,
        }
    }
}

/// Score one still against its golden: byte equality, pixel equality, the
/// FLIP mean and worst, and both pixel digests.
///
/// # Errors
///
/// [`GoldenError::Undecodable`] for a PNG that does not decode,
/// [`GoldenError::CanvasMismatch`] when the frames differ in size, and
/// [`GoldenError::Metric`] when the metric refuses the pair.
pub fn score_still(still: &str, capture: &[u8], golden: &[u8]) -> Result<StillScore, GoldenError> {
    let live = decode(still, capture)?;
    let filed = decode(still, golden)?;
    let (live_size, filed_size) = (size_of(&live), size_of(&filed));
    if live_size != filed_size {
        return Err(GoldenError::CanvasMismatch {
            still: still.to_owned(),
            capture: live_size,
            golden: filed_size,
        });
    }
    let visible = flip(&live, &filed, Region::Full).map_err(|error| GoldenError::Metric {
        still: still.to_owned(),
        reason: error.to_string(),
    })?;
    Ok(StillScore {
        still: still.to_owned(),
        bytes_equal: capture == golden,
        pixels_equal: live.rgba() == filed.rgba(),
        flip_x10000: visible.mean_x10000,
        flip_max_x10000: visible.max_x10000,
        capture_fnv: digest_hex(live.rgba()),
        golden_fnv: digest_hex(filed.rgba()),
    })
}

/// Nearest-rank percentile over the pooled per-still FLIP means: the value at
/// rank `ceil(p * n / 100)` of the sorted list, so `p = 0` answers the smallest
/// and `p = 100` the largest. Integer only, so the pool repeats bit for bit.
#[must_use]
pub fn percentile(values: &[u32], p: u32) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let count = u64::try_from(values.len()).unwrap_or(u64::MAX);
    let rank = (u64::from(p) * count).saturating_add(99) / 100;
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = usize::try_from(rank.saturating_sub(1))
        .unwrap_or(0)
        .min(sorted.len() - 1);
    sorted[index]
}

/// The per-pixel absolute difference as an RGBA image: black where the still
/// and its golden agree, the delta itself everywhere they part.
///
/// # Errors
///
/// [`GoldenError::Undecodable`], [`GoldenError::CanvasMismatch`] or
/// [`GoldenError::Encode`].
pub fn diff_png(still: &str, capture: &[u8], golden: &[u8]) -> Result<Vec<u8>, GoldenError> {
    let (live, filed) = (decode(still, capture)?, decode(still, golden)?);
    let (live_size, filed_size) = (size_of(&live), size_of(&filed));
    if live_size != filed_size {
        return Err(GoldenError::CanvasMismatch {
            still: still.to_owned(),
            capture: live_size,
            golden: filed_size,
        });
    }
    let mut rgba = Vec::with_capacity(live.rgba().len());
    for index in 0..live.pixels() {
        let (a, b) = (live.rgb_at(index), filed.rgb_at(index));
        rgba.extend_from_slice(&[
            a[0].abs_diff(b[0]),
            a[1].abs_diff(b[1]),
            a[2].abs_diff(b[2]),
            255,
        ]);
    }
    encode(still, &rgba, live.width(), live.height())
}

/// How one bless run compares its stills.
#[derive(Clone, Copy, Debug)]
pub struct RunConfig<'a> {
    /// Equality the run demands.
    pub tier: Tier,
    /// Adapter whose bar the pooled score is held to.
    pub adapter: &'a str,
    /// Promote each changed still's candidate into its golden.
    pub bless: bool,
    /// Stills allowed to differ this run; one that no longer differs is
    /// refused as stale.
    pub known_failures: &'a [&'a str],
}

/// One bless run's outcome: what was compared, what changed, what was written
/// and the pooled perceptual score.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// The run's tier.
    pub tier: Tier,
    /// The adapter name the run was filed under.
    pub adapter: String,
    /// The bar that applied, named or the fallback.
    pub bar: AdapterBar,
    /// Stills the capture directory offered.
    pub stills: usize,
    /// Stills scored against a filed golden.
    pub compared: usize,
    /// Stills that met the run's tier unchanged.
    pub matched: usize,
    /// Stills that did not meet the run's tier, minus the known failures.
    pub changed: Vec<String>,
    /// Changed stills the known-failure table forgives.
    pub known: Vec<String>,
    /// Stills whose first golden this bless wrote (a seed writes no artifact).
    pub seeded: Vec<String>,
    /// Stills this bless promoted into their golden.
    pub blessed: Vec<String>,
    /// Pooled p95 FLIP mean over the scored stills that are not known
    /// failures, ten-thousandths.
    pub p95_x10000: u32,
    /// Pooled p99 in the same units.
    pub p99_x10000: u32,
    /// Every scored still, capture order.
    pub scores: Vec<StillScore>,
}

impl Report {
    /// True when the pooled score sits inside the adapter's bar.
    #[must_use]
    pub fn bar_breach(&self) -> bool {
        self.p95_x10000 > self.bar.p95_x10000 || self.p99_x10000 > self.bar.p99_x10000
    }

    /// The stills this run refuses to accept: on the bit-exact tier every
    /// changed still, on the perceptual tier the changed ones only once the
    /// pooled score breaches the adapter's bar.
    #[must_use]
    pub fn open(&self) -> Vec<String> {
        match self.tier {
            Tier::BitExact => self.changed.clone(),
            Tier::Perceptual => {
                if self.bar_breach() {
                    self.changed.clone()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// True when nothing is left to answer for.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.open().is_empty()
    }

    /// The run as plain lines, for a CI log or the door's reply.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "golden bless: tier {} adapter {} bar {} p95 {} p99 {} pass {}\n",
            self.tier.label(),
            self.adapter,
            self.bar.adapter,
            self.p95_x10000,
            self.p99_x10000,
            self.passes()
        );
        out.push_str(&format!(
            "stills {} compared {} matched {} changed {} known {} seeded {} blessed {}\n",
            self.stills,
            self.compared,
            self.matched,
            self.changed.len(),
            self.known.len(),
            self.seeded.len(),
            self.blessed.len()
        ));
        for (label, names) in [
            ("changed", &self.changed),
            ("known", &self.known),
            ("seeded", &self.seeded),
            ("blessed", &self.blessed),
        ] {
            for still in names {
                out.push_str(&format!("{still}: {label}\n"));
            }
        }
        for score in &self.scores {
            out.push_str(&format!(
                "{}: flip {} max {} bytes {} pixels {}\n",
                score.still,
                score.flip_x10000,
                score.flip_max_x10000,
                score.bytes_equal,
                score.pixels_equal
            ));
        }
        out
    }
}

/// Compare every still in `capture_dir` against the golden of the same name in
/// `golden_dir`, write the bless artifacts for the ones that changed and
/// return the [`Report`].
///
/// A changed still writes `{still}.new.png`, `{still}.diff.png` and
/// `{still}.old.png` beside its golden; with [`RunConfig::bless`] the candidate
/// also takes `{still}.png` and is named in [`Report::blessed`], while a
/// missing golden is seeded instead of refused. Artifact copies are never
/// themselves scanned as stills.
///
/// # Errors
///
/// [`GoldenError::MissingGolden`] for a still with no golden on a non-bless
/// run, [`GoldenError::StaleKnownFailure`] for a table row that no longer
/// differs (or names no still at all), [`GoldenError::TooManyStills`] past the
/// bound, and the read, decode, metric, encode and write refusals of the stills
/// it walks.
pub fn run(
    capture_dir: &Path,
    golden_dir: &Path,
    config: &RunConfig<'_>,
) -> Result<Report, GoldenError> {
    let stills = list_stills(capture_dir)?;
    if stills.len() > MAX_STILLS {
        return Err(GoldenError::TooManyStills {
            count: stills.len(),
        });
    }
    let bar = bar_for(config.adapter);
    let mut report = Report {
        tier: config.tier,
        adapter: config.adapter.to_owned(),
        bar,
        stills: stills.len(),
        compared: 0,
        matched: 0,
        changed: Vec::new(),
        known: Vec::new(),
        seeded: Vec::new(),
        blessed: Vec::new(),
        p95_x10000: 0,
        p99_x10000: 0,
        scores: Vec::new(),
    };
    for still in &stills {
        let capture = read_png(&capture_dir.join(still), still)?;
        let Some(golden) = read_golden(golden_dir, still)? else {
            if config.bless {
                write_png(golden_dir, still, &capture)?;
                report.seeded.push(still.clone());
            } else {
                return Err(GoldenError::MissingGolden {
                    still: still.clone(),
                });
            }
            continue;
        };
        let score = score_still(still, &capture, &golden)?;
        report.compared += 1;
        report.scores.push(score.clone());
        if score.met(config.tier) {
            report.matched += 1;
            continue;
        }
        write_artifacts(golden_dir, still, &capture, &golden)?;
        if config.known_failures.contains(&still.as_str()) {
            report.known.push(still.clone());
        } else {
            report.changed.push(still.clone());
            if config.bless {
                write_png(golden_dir, still, &capture)?;
                report.blessed.push(still.clone());
            }
        }
    }
    for name in config.known_failures {
        if !report.known.iter().any(|known| known.as_str() == *name) {
            return Err(GoldenError::StaleKnownFailure {
                still: (*name).to_owned(),
            });
        }
    }
    let pool: Vec<u32> = report
        .scores
        .iter()
        .filter(|score| !report.known.iter().any(|known| known == &score.still))
        .map(|score| score.flip_x10000)
        .collect();
    report.p95_x10000 = percentile(&pool, 95);
    report.p99_x10000 = percentile(&pool, 99);
    Ok(report)
}

/// Every still name in `dir`, sorted: PNG files only, bless artifacts skipped,
/// names past [`MAX_NAME_LEN`] refused.
fn list_stills(dir: &Path) -> Result<Vec<String>, GoldenError> {
    let entries = std::fs::read_dir(dir).map_err(|error| io_error(dir, error))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error(dir, error))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |raw| raw.to_string_lossy().into_owned());
        if is_artifact(&name) {
            continue;
        }
        if !name.ends_with(GOLDEN_SUFFIX) || name.len() > MAX_NAME_LEN {
            return Err(GoldenError::BadName { name });
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// True for a bless artifact (`{still}.new.png`, `.diff.png`, `.old.png`),
/// which records a change instead of being a still itself.
fn is_artifact(name: &str) -> bool {
    [NEW_SUFFIX, DIFF_SUFFIX, OLD_SUFFIX]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// The `{still}.png` next to `still` under `golden_dir`, `None` when the
/// golden has not been filed yet.
fn read_golden(dir: &Path, still: &str) -> Result<Option<Vec<u8>>, GoldenError> {
    match read_png(&dir.join(still), still) {
        Ok(png) => Ok(Some(png)),
        Err(GoldenError::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// One PNG under its bound, refusing to read an oversized file into memory.
fn read_png(path: &Path, still: &str) -> Result<Vec<u8>, GoldenError> {
    let meta = std::fs::metadata(path).map_err(|error| io_error(path, error))?;
    let bytes = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if bytes > MAX_PNG_BYTES {
        return Err(GoldenError::PngTooLarge {
            still: still.to_owned(),
            bytes,
        });
    }
    std::fs::read(path).map_err(|error| io_error(path, error))
}

/// Write `png` to `{dir}/{name}`, creating nothing else.
fn write_png(dir: &Path, name: &str, png: &[u8]) -> Result<(), GoldenError> {
    let path = dir.join(name);
    std::fs::write(&path, png).map_err(|error| io_error(&path, error))
}

/// Leave the three bless artifacts for one changed still beside its golden.
fn write_artifacts(
    dir: &Path,
    still: &str,
    capture: &[u8],
    golden: &[u8],
) -> Result<(), GoldenError> {
    let stem = still.strip_suffix(GOLDEN_SUFFIX).unwrap_or(still);
    write_png(dir, &format!("{stem}{NEW_SUFFIX}"), capture)?;
    write_png(dir, &format!("{stem}{OLD_SUFFIX}"), golden)?;
    let diff = diff_png(still, capture, golden)?;
    write_png(dir, &format!("{stem}{DIFF_SUFFIX}"), &diff)
}

/// Decode PNG bytes into a validated frame, naming the still on refusal.
fn decode(still: &str, png: &[u8]) -> Result<Frame, GoldenError> {
    Frame::decode_png(png).map_err(|error| GoldenError::Undecodable {
        still: still.to_owned(),
        reason: error.to_string(),
    })
}

/// Encode RGBA8 bytes as PNG, naming the still on refusal.
fn encode(still: &str, rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, GoldenError> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|error| GoldenError::Encode {
            still: still.to_owned(),
            reason: error.to_string(),
        })?;
    Ok(out)
}

/// `(width, height)` of a frame, as a report reads it.
fn size_of(frame: &Frame) -> (u32, u32) {
    (frame.width(), frame.height())
}

fn io_error(path: &Path, error: std::io::Error) -> GoldenError {
    GoldenError::Io {
        path: path.display().to_string(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    use std::path::PathBuf;

    const W: u32 = 32;
    const H: u32 = 24;

    /// The run's still list: the Q1 layout's four staged and thirty orbit
    /// frames, by name.
    fn q1_stills() -> Vec<String> {
        let mut names: Vec<String> = (0..4).map(|i| format!("synth_{i}.png")).collect();
        names.extend((0..30).map(|i| format!("orbit_{i:02}.png")));
        names
    }

    /// One test's still list, from the names it names.
    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// A textured canvas, integer only, so a window carries real variance.
    fn tone_at(x: u32, y: u32) -> [u8; 3] {
        let wobble = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).unwrap_or(0) - 11;
        [194i32, 160, 104].map(|c| u8::try_from((c + wobble).clamp(0, 255)).unwrap_or(255))
    }

    /// The canvas with its lower band brightened by a fifth: a real look
    /// change, not a one-pixel wobble.
    fn tone_changed(x: u32, y: u32) -> [u8; 3] {
        if y >= H / 2 {
            tone_at(x, y)
                .map(|c| u8::try_from(((u32::from(c) * 6 + 2) / 5).min(255)).unwrap_or(255))
        } else {
            tone_at(x, y)
        }
    }

    fn rgba_of(paint: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity(usize::try_from(W * H * 4).unwrap_or(0));
        for y in 0..H {
            for x in 0..W {
                let [r, g, b] = paint(x, y);
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        out
    }

    fn png(rgba: &[u8], color: ExtendedColorType) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(rgba, W, H, color)
            .expect("fixture encodes");
        out
    }

    /// The same pixels as `png`, filed under another color type: different
    /// bytes, the same decoded frame.
    fn png_rgb(rgba: &[u8]) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(usize::try_from(W * H * 3).unwrap_or(0));
        for chunk in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&chunk[..3]);
        }
        png(&rgb, ExtendedColorType::Rgb8)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-golden-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir builds");
        dir
    }

    /// A capture and a golden directory holding `stills`, every frame painted
    /// the same until `changed` names a different paint per still.
    fn tree(
        tag: &str,
        stills: &[String],
        changed: &[&str],
    ) -> (PathBuf, PathBuf, Vec<u8>, Vec<u8>) {
        let captures = temp_dir(&format!("{tag}-captures"));
        let goldens = temp_dir(&format!("{tag}-goldens"));
        let plain = png(&rgba_of(tone_at), ExtendedColorType::Rgba8);
        let bright = png(&rgba_of(tone_changed), ExtendedColorType::Rgba8);
        for still in stills {
            let candidate = if changed.contains(&still.as_str()) {
                &bright
            } else {
                &plain
            };
            std::fs::write(captures.join(still), candidate).expect("capture writes");
            std::fs::write(goldens.join(still), &plain).expect("golden writes");
        }
        (captures, goldens, plain, bright)
    }

    fn config<'a>(tier: Tier, bless: bool, known_failures: &'a [&'a str]) -> RunConfig<'a> {
        RunConfig {
            tier,
            adapter: "llvmpipe",
            bless,
            known_failures,
        }
    }

    #[test]
    fn golden_bless_compares_the_34_stills_of_a_q1_run() {
        let stills = q1_stills();
        assert_eq!(stills.len(), 34, "the Q1 suite is 4 staged plus 30 orbit");
        let (captures, goldens, _plain, _bright) = tree("count", &stills, &[]);
        let report = run(&captures, &goldens, &config(Tier::BitExact, false, &[])).expect("run");
        assert_eq!(
            (report.stills, report.compared, report.matched),
            (34, 34, 34),
            "every filed golden compared and matched"
        );
        assert!(report.changed.is_empty() && report.known.is_empty());
        assert_eq!((report.p95_x10000, report.p99_x10000), (0, 0));
        assert!(report.passes());
        let again = run(&captures, &goldens, &config(Tier::BitExact, false, &[])).expect("rerun");
        assert_eq!(report, again, "the same tree twice reports the same");
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_writes_new_and_diff_and_keeps_the_old_golden() {
        let stills = q1_stills();
        let changed = "orbit_07.png";
        let (captures, goldens, plain, bright) = tree("bless", &stills, &[changed]);
        let report = run(&captures, &goldens, &config(Tier::Perceptual, true, &[])).expect("bless");
        assert_eq!(report.blessed, vec![changed.to_owned()]);
        assert_eq!(report.changed, vec![changed.to_owned()]);
        let stem = changed
            .strip_suffix(GOLDEN_SUFFIX)
            .expect("the fixture still ends in .png");
        let on_disk = |name: &str| std::fs::read(goldens.join(name));
        // The candidate took the golden, and the frame it replaced is kept.
        assert_eq!(on_disk(changed).expect("golden"), bright, "golden blessed");
        assert_eq!(
            on_disk(&format!("{stem}{OLD_SUFFIX}")).expect("old"),
            plain,
            "the blessed change keeps the golden it replaced"
        );
        assert_eq!(
            on_disk(&format!("{stem}{NEW_SUFFIX}")).expect("new"),
            bright,
            "the candidate is left as a receipt"
        );
        let diff = on_disk(&format!("{stem}{DIFF_SUFFIX}")).expect("diff");
        let frame = decode(changed, &diff).expect("diff decodes");
        assert_eq!(size_of(&frame), (W, H), "the diff keeps the canvas");
        let brightened = frame.rgb_at(
            usize::try_from((H / 2 + 1) * W + 2).unwrap_or(0), // row inside the changed band
        );
        assert!(
            brightened[0] > 0,
            "the diff image lights the band that changed, got {brightened:?}"
        );
        // Nothing else in the tree was touched.
        for still in &stills {
            if still.as_str() == changed {
                continue;
            }
            assert_eq!(
                std::fs::read(goldens.join(still)).expect("untouched golden"),
                plain,
                "{still} stayed filed"
            );
        }
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_refuses_a_missing_golden_and_writes_nothing() {
        let (captures, goldens, plain, _bright) = tree("missing", &names(&["synth_0.png"]), &[]);
        std::fs::remove_file(goldens.join("synth_0.png")).expect("golden removed");
        let error = run(&captures, &goldens, &config(Tier::BitExact, false, &[]))
            .expect_err("a still without a golden refuses");
        assert_eq!(
            error,
            GoldenError::MissingGolden {
                still: "synth_0.png".to_owned()
            }
        );
        let left: Vec<String> = std::fs::read_dir(&goldens)
            .expect("golden dir lists")
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .collect();
        assert!(
            left.is_empty(),
            "a refusal writes no artifact, got {left:?}"
        );
        assert_eq!(
            std::fs::read(captures.join("synth_0.png")).expect("capture"),
            plain
        );
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_seeds_a_first_golden_instead_of_refusing() {
        let (captures, goldens, plain, _bright) = tree("seed", &names(&["synth_1.png"]), &[]);
        std::fs::remove_file(goldens.join("synth_1.png")).expect("golden removed");
        let report = run(&captures, &goldens, &config(Tier::BitExact, true, &[])).expect("seed");
        assert_eq!(report.seeded, vec!["synth_1.png".to_owned()]);
        assert!(report.changed.is_empty());
        assert_eq!(
            std::fs::read(goldens.join("synth_1.png")).expect("seeded golden"),
            plain
        );
        let left: Vec<String> = std::fs::read_dir(&goldens)
            .expect("golden dir lists")
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(
            left,
            vec!["synth_1.png".to_owned()],
            "a seed has no previous frame to keep"
        );
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_refuses_a_stale_known_failure() {
        let (captures, goldens, _plain, _bright) = tree("stale", &names(&["orbit_00.png"]), &[]);
        let error = run(
            &captures,
            &goldens,
            &config(Tier::BitExact, false, &["orbit_00.png"]),
        )
        .expect_err("a forgiven still that matches is stale");
        assert_eq!(
            error,
            GoldenError::StaleKnownFailure {
                still: "orbit_00.png".to_owned()
            }
        );
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_forgives_a_known_failure_and_keeps_it_out_of_the_pool() {
        let stills = q1_stills();
        let known = "orbit_11.png";
        let (captures, goldens, _plain, _bright) = tree("known", &stills, &[known]);
        let report = run(
            &captures,
            &goldens,
            &config(Tier::Perceptual, false, &[known]),
        )
        .expect("run");
        assert_eq!(report.known, vec![known.to_owned()]);
        assert!(report.changed.is_empty(), "the table forgives it");
        assert!(report.passes());
        assert_eq!(
            (report.p95_x10000, report.p99_x10000),
            (0, 0),
            "a forgiven still does not enter the pooled score"
        );
        // The artifact receipt is still written for the forgiven frame.
        let stem = known
            .strip_suffix(GOLDEN_SUFFIX)
            .expect("the fixture still ends in .png");
        assert!(goldens.join(format!("{stem}{NEW_SUFFIX}")).is_file());
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_bit_exact_tier_refuses_a_reencode_the_pixels_survive() {
        let rgba = rgba_of(tone_at);
        let filed = png(&rgba, ExtendedColorType::Rgba8);
        let reencoded = png_rgb(&rgba);
        assert_ne!(filed, reencoded, "the two streams differ in bytes");
        let score = score_still("synth_0.png", &reencoded, &filed).expect("score");
        assert!(!score.bytes_equal, "bytes differ");
        assert!(score.pixels_equal, "the pixels do not");
        assert_eq!((score.flip_x10000, score.flip_max_x10000), (0, 0));
        assert_eq!(score.capture_fnv, score.golden_fnv, "same pixel digest");
        assert!(score.met(Tier::Perceptual), "the pixel tier passes");
        assert!(!score.met(Tier::BitExact), "the byte tier refuses");
    }

    #[test]
    fn golden_bless_perceptual_run_is_held_to_the_adapter_bar() {
        let stills = q1_stills();
        let changed = "orbit_03.png";
        let (captures, goldens, _plain, bright) = tree("bar", &stills, &[changed]);
        let score = score_still(
            changed,
            &bright,
            &std::fs::read(goldens.join(changed)).expect("golden reads"),
        )
        .expect("score");
        assert!(score.flip_x10000 > 0, "a fifth brighter is visible");
        let report = run(&captures, &goldens, &config(Tier::Perceptual, false, &[])).expect("run");
        assert_eq!(report.changed, vec![changed.to_owned()]);
        assert_eq!(report.bar, bar_for("llvmpipe"));
        assert_eq!((report.bar.p95_x10000, report.bar.p99_x10000), (0, 0));
        assert!(report.bar_breach(), "the software leg promises no slop");
        assert!(!report.passes());
        assert_eq!(report.open(), vec![changed.to_owned()]);
        assert_eq!(report.p99_x10000, score.flip_x10000);
        // The fallback row carries FLIP's own visibility thresholds.
        let gpu = bar_for("some-unlisted-rtx");
        assert_eq!(gpu.adapter, "default");
        assert_eq!((gpu.p95_x10000, gpu.p99_x10000), (200, 400));
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_refuses_a_canvas_that_changed_size() {
        let (captures, goldens, plain, _bright) = tree("canvas", &names(&["synth_2.png"]), &[]);
        let mut rgba = Vec::new();
        for y in 0..(H - 4) {
            for x in 0..(W - 4) {
                let [r, g, b] = tone_at(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let smaller = {
            let mut out = Vec::new();
            PngEncoder::new(&mut out)
                .write_image(&rgba, W - 4, H - 4, ExtendedColorType::Rgba8)
                .expect("smaller encodes");
            out
        };
        std::fs::write(captures.join("synth_2.png"), &smaller).expect("capture writes");
        let error = run(&captures, &goldens, &config(Tier::BitExact, false, &[]))
            .expect_err("a resized still refuses");
        assert_eq!(
            error,
            GoldenError::CanvasMismatch {
                still: "synth_2.png".to_owned(),
                capture: (W - 4, H - 4),
                golden: (W, H),
            }
        );
        assert_eq!(
            std::fs::read(goldens.join("synth_2.png")).expect("golden"),
            plain
        );
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }

    #[test]
    fn golden_bless_pool_nearest_rank_pins_the_percentiles() {
        // Eleven stills with visibility 0, 10, 20 ... 100, integer-pinned.
        let pool: Vec<u32> = (0..11)
            .map(|i| u32::try_from(i).unwrap_or(0) * 10)
            .collect();
        assert_eq!(percentile(&pool, 0), 0);
        assert_eq!(percentile(&pool, 50), 50);
        assert_eq!(percentile(&pool, 95), 100);
        assert_eq!(percentile(&pool, 99), 100);
        assert_eq!(percentile(&pool, 100), 100);
        assert_eq!(percentile(&[], 95), 0, "an empty pool answers zero");
        // Rank ceil(0.95 * 20) = 19 is the twentieth-largest of twenty values.
        let wide: Vec<u32> = (1..=20).collect();
        assert_eq!(percentile(&wide, 95), 19);
        assert_eq!(percentile(&wide, 90), 18);
    }

    #[test]
    fn golden_bless_run_skips_the_artifact_copies_it_wrote() {
        let stills = q1_stills();
        let changed = "orbit_05.png";
        let (captures, goldens, _plain, _bright) = tree("rescan", &stills, &[changed]);
        let first = run(&captures, &goldens, &config(Tier::Perceptual, true, &[])).expect("bless");
        assert_eq!(first.stills, 34);
        // The golden dir now holds `.new`, `.diff` and `.old` copies beside the
        // 34 filed frames; a second run scans the goldens and finds 34 stills.
        let second = run(&goldens, &goldens, &config(Tier::BitExact, false, &[])).expect("rescan");
        assert_eq!(
            (second.stills, second.compared, second.matched),
            (34, 34, 34),
            "artifacts are receipts, not stills"
        );
        let _ = std::fs::remove_dir_all(&captures);
        let _ = std::fs::remove_dir_all(&goldens);
    }
}
