//! Animation timeline (E6): a localized-edit sheet over joint tracks.
//!
//! Owner: the dock's timeline tab. The sheet is plain data: joint id, key
//! times and thousandth values, so no animation clip crate is imported here.
//! The seam is the host: it converts a sheet to and from `engine_anim` clips
//! (the editor crate has no dependency on that crate) and the two only share
//! the numbers this module names.
//! Units: time is a caller tick, values are thousandths ([`MILLI`] = 1.0), so
//! sampling is integer math and never a float.
//! Determinism: tracks keep sheet order, keys keep tick order, and
//! [`Sheet::digest`] hashes exactly those bytes.
//! Errors: [`SheetError`]; a refused edit leaves the sheet untouched.

use engine_core::hash_bytes;

/// Value unit: `1000` milli is `1.0`.
pub const MILLI: i32 = 1000;
/// Smoothing steps a Bezier parameter solve runs; fixed, so sampling is
/// deterministic.
const BEZIER_STEPS: usize = 64;
/// Denominator used by the Bezier parameter solve (one unit of parameter).
const PARAM_SCALE: i128 = 1_000_000;
/// Denominator of a Bezier axis value: the solve's parameter scale to the
/// fourth, because each of the three curve factors carries one scale and the
/// control point carries one more.
const PARAM_QUAD: i128 = PARAM_SCALE * PARAM_SCALE * PARAM_SCALE * PARAM_SCALE;

/// How one key hands its value to the next key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    /// Hold the key value until the next key.
    Step,
    /// Straight line between the two values.
    Linear,
    /// Slow start, fast end (`t^2`).
    EaseIn,
    /// Fast start, slow end (`2t - t^2`).
    EaseOut,
    /// Cubic Bezier on the unit square, control points in thousandths.
    Bezier {
        /// First control point x.
        cx1_milli: i16,
        /// First control point y.
        cy1_milli: i16,
        /// Second control point x.
        cx2_milli: i16,
        /// Second control point y.
        cy2_milli: i16,
    },
}

impl Easing {
    /// Refuse a Bezier whose control points leave the unit square.
    fn check(self) -> bool {
        /// True when one control lane sits in `0..=MILLI`.
        fn inside(value: i16) -> bool {
            value >= 0 && i32::from(value) <= MILLI
        }
        match self {
            Self::Bezier {
                cx1_milli,
                cy1_milli,
                cx2_milli,
                cy2_milli,
            } => inside(cx1_milli) && inside(cy1_milli) && inside(cx2_milli) && inside(cy2_milli),
            _ => true,
        }
    }

    /// Short tag used by panel rows and the digest.
    pub fn label(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Linear => "linear",
            Self::EaseIn => "ease-in",
            Self::EaseOut => "ease-out",
            Self::Bezier { .. } => "bezier",
        }
    }

    /// Digest tag and payload bytes.
    fn encode(self) -> Vec<u8> {
        match self {
            Self::Step => vec![0],
            Self::Linear => vec![1],
            Self::EaseIn => vec![2],
            Self::EaseOut => vec![3],
            Self::Bezier {
                cx1_milli,
                cy1_milli,
                cx2_milli,
                cy2_milli,
            } => {
                let mut out = vec![4];
                for value in [cx1_milli, cy1_milli, cx2_milli, cy2_milli] {
                    out.extend_from_slice(&value.to_le_bytes());
                }
                out
            }
        }
    }
}

/// One key on a joint track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// Tick the key sits on, strictly inside the clip.
    pub tick: u32,
    /// Joint value in thousandths per axis.
    pub value_milli: [i32; 3],
    /// How this key eases into the next one.
    pub easing: Easing,
}

/// One joint track: a joint id and its keys in tick order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// Rig joint this track drives.
    pub joint: u16,
    /// Keys, strictly ascending by tick.
    pub keys: Vec<Key>,
}

/// One animation sheet: a clip length, a name and joint tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    /// Sheet name, also the first digest field.
    pub name: String,
    /// Clip length in ticks; every key tick is below it.
    pub ticks: u32,
    /// Joint tracks in sheet order.
    pub tracks: Vec<Track>,
}

/// What refused a timeline edit or a sample. Nothing changed on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SheetError {
    /// The sheet name held no text.
    EmptyName,
    /// The sheet has no track.
    EmptyTracks,
    /// A track holds no key.
    EmptyTrack {
        /// Track joint.
        joint: u16,
    },
    /// Two tracks drive the same joint.
    DuplicateJoint {
        /// Repeated joint.
        joint: u16,
    },
    /// Two keys of one track share a tick.
    DuplicateTick {
        /// Track joint.
        joint: u16,
        /// Repeated tick.
        tick: u32,
    },
    /// A key tick is not above the previous key.
    KeysOutOfOrder {
        /// Track joint.
        joint: u16,
        /// Key position that broke the order.
        at: usize,
    },
    /// A key tick is at or past the clip length.
    TickPastClip {
        /// Track joint.
        joint: u16,
        /// Key tick.
        tick: u32,
        /// Clip length.
        ticks: u32,
    },
    /// A Bezier control point left the unit square.
    BezierOutOfRange {
        /// Track joint.
        joint: u16,
        /// Key tick.
        tick: u32,
    },
    /// No track sits at that index.
    UnknownTrack {
        /// Index asked for.
        index: usize,
    },
    /// No track drives that joint.
    UnknownJoint {
        /// Joint asked for.
        joint: u16,
    },
    /// No key sits on that tick of the track.
    UnknownKey {
        /// Track joint.
        joint: u16,
        /// Tick asked for.
        tick: u32,
    },
    /// The region is empty or runs past the clip.
    BadRegion {
        /// First tick.
        from: u32,
        /// Last tick (exclusive).
        to: u32,
        /// Clip length.
        ticks: u32,
    },
}

impl std::fmt::Display for SheetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("sheet name holds no text; name it"),
            Self::EmptyTracks => f.write_str("sheet has no track; add one joint track"),
            Self::EmptyTrack { joint } => write!(
                f,
                "track for joint {joint} has no key; insert a key or drop the track"
            ),
            Self::DuplicateJoint { joint } => write!(
                f,
                "joint {joint} has two tracks; keep one track per joint"
            ),
            Self::DuplicateTick { joint, tick } => write!(
                f,
                "joint {joint} already has a key at t{tick:04}; move or delete it"
            ),
            Self::KeysOutOfOrder { joint, at } => write!(
                f,
                "keys of joint {joint} are not ascending at position {at}; sort by tick"
            ),
            Self::TickPastClip { joint, tick, ticks } => write!(
                f,
                "key t{tick:04} of joint {joint} sits past the {ticks}-tick clip; keep ticks below it"
            ),
            Self::BezierOutOfRange { joint, tick } => write!(
                f,
                "bezier key t{tick:04} of joint {joint} leaves 0..{MILLI}; keep control points inside"
            ),
            Self::UnknownTrack { index } => write!(
                f,
                "sheet has no track {index}; sample 0..{}",
                index.saturating_sub(1)
            ),
            Self::UnknownJoint { joint } => {
                write!(f, "no track drives joint {joint}; add the track first")
            }
            Self::UnknownKey { joint, tick } => write!(
                f,
                "joint {joint} has no key at t{tick:04}; pick a key tick"
            ),
            Self::BadRegion { from, to, ticks } => write!(
                f,
                "region {from}..{to} is empty or past the {ticks}-tick clip; keep from < to <= {ticks}"
            ),
        }
    }
}

impl std::error::Error for SheetError {}

impl Sheet {
    /// Check the whole sheet: name, joints, key order, clip bounds, Bezier
    /// control points. Every edit path ends here, so a stored sheet is sorted.
    pub fn sorted(&self) -> Result<(), SheetError> {
        if self.name.trim().is_empty() {
            return Err(SheetError::EmptyName);
        }
        if self.tracks.is_empty() {
            return Err(SheetError::EmptyTracks);
        }
        let mut seen: Vec<u16> = Vec::with_capacity(self.tracks.len());
        for track in &self.tracks {
            if seen.contains(&track.joint) {
                return Err(SheetError::DuplicateJoint { joint: track.joint });
            }
            seen.push(track.joint);
            if track.keys.is_empty() {
                return Err(SheetError::EmptyTrack { joint: track.joint });
            }
            let mut previous: Option<u32> = None;
            for (at, key) in track.keys.iter().enumerate() {
                if let Some(last) = previous {
                    if key.tick == last {
                        return Err(SheetError::DuplicateTick {
                            joint: track.joint,
                            tick: key.tick,
                        });
                    }
                    if key.tick < last {
                        return Err(SheetError::KeysOutOfOrder {
                            joint: track.joint,
                            at,
                        });
                    }
                }
                if key.tick >= self.ticks {
                    return Err(SheetError::TickPastClip {
                        joint: track.joint,
                        tick: key.tick,
                        ticks: self.ticks,
                    });
                }
                if !key.easing.check() {
                    return Err(SheetError::BezierOutOfRange {
                        joint: track.joint,
                        tick: key.tick,
                    });
                }
                previous = Some(key.tick);
            }
        }
        Ok(())
    }

    /// Sample one track at `tick`.
    ///
    /// Before the first key the first value holds; at or past the last key the
    /// last value holds. Between keys the left key's easing runs, in integer
    /// math: the fraction is exact for [`Easing::Step`] and [`Easing::Linear`],
    /// and for the curved easings each axis value is `v0 + round(delta *
    /// curve)`, halves away from zero. A Bezier solves its own parameter by
    /// fixed bisection ([`BEZIER_STEPS`]) and evaluates y at that parameter, so
    /// control points on the diagonal replay Linear exactly.
    pub fn sample(&self, track: usize, tick: u32) -> Result<[i32; 3], SheetError> {
        let track = self
            .tracks
            .get(track)
            .ok_or(SheetError::UnknownTrack { index: track })?;
        let first = track
            .keys
            .first()
            .ok_or(SheetError::EmptyTrack { joint: track.joint })?;
        let last = track.keys.last().unwrap_or(first);
        if tick <= first.tick {
            return Ok(first.value_milli);
        }
        if tick >= last.tick {
            return Ok(last.value_milli);
        }
        let at = track
            .keys
            .windows(2)
            .position(|pair| pair[0].tick <= tick && tick < pair[1].tick)
            .ok_or(SheetError::UnknownKey {
                joint: track.joint,
                tick,
            })?;
        let from = track.keys[at];
        let to = track.keys[at + 1];
        let span = to.tick - from.tick;
        let num = tick - from.tick;
        let frac = curve(from.easing, num, span);
        let mut out = [0; 3];
        for (axis, slot) in out.iter_mut().enumerate() {
            *slot = value_at(from.value_milli[axis], to.value_milli[axis], frac);
        }
        Ok(out)
    }

    /// Localized edit (SMART-style): a sheet over `from..to` holding only the
    /// keys in range.
    ///
    /// A span cut by a boundary gains a key sampled at that boundary, so every
    /// tick inside the range samples within one milli of the source for Step
    /// and Linear easing (the cut key is rounded to the value unit, so a
    /// re-interpolated sub-span can differ by that one unit). The cut key
    /// carries the eased span's own easing and replays it from the sampled
    /// value: for a curved span the sub-span shape is re-based, which the
    /// caller sees as a new key on that boundary.
    pub fn region(&self, from: u32, to: u32) -> Result<Sheet, SheetError> {
        if from >= to || to > self.ticks {
            return Err(SheetError::BadRegion {
                from,
                to,
                ticks: self.ticks,
            });
        }
        let mut tracks = Vec::with_capacity(self.tracks.len());
        for (index, track) in self.tracks.iter().enumerate() {
            let inside: Vec<Key> = track
                .keys
                .iter()
                .filter(|key| key.tick >= from && key.tick <= to)
                .copied()
                .collect();
            let mut keys: Vec<Key> = Vec::with_capacity(inside.len() + 2);
            let head = inside.first().map(|key| key.tick) != Some(from);
            if head {
                let value = self.sample(index, from)?;
                keys.push(Key {
                    tick: 0,
                    value_milli: value,
                    easing: easing_at(track, from),
                });
            }
            keys.extend(inside.iter().map(|key| Key {
                tick: key.tick - from,
                ..*key
            }));
            let tail = keys.last().map(|key| key.tick + from) != Some(to);
            if tail {
                let value = self.sample(index, to)?;
                keys.push(Key {
                    tick: to - from,
                    value_milli: value,
                    easing: easing_at(track, to),
                });
            }
            tracks.push(Track {
                joint: track.joint,
                keys,
            });
        }
        let sheet = Sheet {
            name: format!("{}[{from}..{to}]", self.name),
            ticks: to - from + 1,
            tracks,
        };
        sheet.sorted()?;
        Ok(sheet)
    }

    /// Stable digest over the name, clip length and every track and key.
    pub fn digest(&self) -> u64 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&self.ticks.to_le_bytes());
        for track in &self.tracks {
            bytes.extend_from_slice(&track.joint.to_le_bytes());
            for key in &track.keys {
                bytes.extend_from_slice(&key.tick.to_le_bytes());
                for value in key.value_milli {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes.extend_from_slice(&key.easing.encode());
            }
        }
        hash_bytes(&bytes)
    }

    /// One panel line per key, tracks in sheet order: `joint 3  t0010
    /// [1000, 0, 500]  linear`.
    pub fn rows(&self) -> Vec<String> {
        let mut out = Vec::new();
        for track in &self.tracks {
            for key in &track.keys {
                out.push(format!(
                    "joint {}  t{:04}  [{}, {}, {}]  {}",
                    track.joint,
                    key.tick,
                    key.value_milli[0],
                    key.value_milli[1],
                    key.value_milli[2],
                    key.easing.label()
                ));
            }
        }
        out
    }

    /// Insert one key in tick order. A duplicate tick refuses; the sheet is
    /// unchanged on error.
    pub fn insert_key(&mut self, joint: u16, key: Key) -> Result<(), SheetError> {
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.joint == joint)
            .ok_or(SheetError::UnknownJoint { joint })?;
        if key.tick >= self.ticks {
            return Err(SheetError::TickPastClip {
                joint,
                tick: key.tick,
                ticks: self.ticks,
            });
        }
        if !key.easing.check() {
            return Err(SheetError::BezierOutOfRange {
                joint,
                tick: key.tick,
            });
        }
        if track.keys.iter().any(|held| held.tick == key.tick) {
            return Err(SheetError::DuplicateTick {
                joint,
                tick: key.tick,
            });
        }
        let at = track
            .keys
            .iter()
            .position(|held| held.tick > key.tick)
            .unwrap_or(track.keys.len());
        track.keys.insert(at, key);
        Ok(())
    }

    /// Delete the key at `tick`, returning it. A missing key refuses.
    pub fn delete_key(&mut self, joint: u16, tick: u32) -> Result<Key, SheetError> {
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.joint == joint)
            .ok_or(SheetError::UnknownJoint { joint })?;
        let at = track
            .keys
            .iter()
            .position(|key| key.tick == tick)
            .ok_or(SheetError::UnknownKey { joint, tick })?;
        Ok(track.keys.remove(at))
    }
}

/// The easing that governs `tick`: the left key of its segment, or the first
/// key's easing at or before the first key.
fn easing_at(track: &Track, tick: u32) -> Easing {
    track
        .keys
        .iter()
        .rev()
        .find(|key| key.tick <= tick)
        .or_else(|| track.keys.first())
        .map(|key| key.easing)
        .unwrap_or(Easing::Linear)
}

/// Eased fraction of one segment as `(numerator, denominator)`.
fn curve(easing: Easing, num: u32, den: u32) -> (i128, i128) {
    let num = i128::from(num);
    let den = i128::from(den);
    match easing {
        Easing::Step => (0, 1),
        Easing::Linear => (num, den),
        Easing::EaseIn => (num * num, den * den),
        Easing::EaseOut => (2 * num * den - num * num, den * den),
        Easing::Bezier {
            cx1_milli,
            cy1_milli,
            cx2_milli,
            cy2_milli,
        } => {
            let x1 = i128::from(cx1_milli) * PARAM_SCALE / i128::from(MILLI);
            let x2 = i128::from(cx2_milli) * PARAM_SCALE / i128::from(MILLI);
            let y1 = i128::from(cy1_milli) * PARAM_SCALE / i128::from(MILLI);
            let y2 = i128::from(cy2_milli) * PARAM_SCALE / i128::from(MILLI);
            let target = num * PARAM_QUAD / den;
            let param = bezier_param(x1, x2, target);
            let y = bezier_at(param, y1, y2);
            (y, PARAM_QUAD)
        }
    }
}

/// One cubic Bezier axis at parameter `param` (scaled by [`PARAM_SCALE`]),
/// with the curve starting at 0 and ending at `PARAM_SCALE`; the value is
/// scaled by [`PARAM_QUAD`].
fn bezier_at(param: i128, p1: i128, p2: i128) -> i128 {
    let rest = PARAM_SCALE - param;
    3 * rest * rest * param * p1
        + 3 * rest * param * param * p2
        + param * param * param * PARAM_SCALE
}

/// Parameter whose x equals `target` (scaled by [`PARAM_QUAD`]), found by
/// fixed bisection.
fn bezier_param(x1: i128, x2: i128, target: i128) -> i128 {
    let mut low = 0;
    let mut high = PARAM_SCALE;
    for _ in 0..BEZIER_STEPS {
        let mid = (low + high) / 2;
        if bezier_at(mid, x1, x2) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2
}

/// `v0 + round(delta * frac)`, halves away from zero.
fn value_at(v0: i32, v1: i32, frac: (i128, i128)) -> i32 {
    let delta = i128::from(v1) - i128::from(v0);
    let scaled = delta * frac.0;
    let out = i128::from(v0) + round_div(scaled, frac.1);
    i32::try_from(out).unwrap_or(v0)
}

/// Round `num / den` to nearest, halves away from zero.
fn round_div(num: i128, den: i128) -> i128 {
    if den == 0 {
        return 0;
    }
    if num >= 0 {
        (num + den / 2) / den
    } else {
        (num - den / 2) / den
    }
}

/// Keys the timeline panel handles; the dock maps egui keys onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKey {
    /// Cursor up (newer key row).
    Up,
    /// Cursor down.
    Down,
    /// Playhead one tick back.
    Left,
    /// Playhead one tick forward.
    Right,
    /// Insert a key for the selected joint at the playhead.
    Insert,
    /// Delete the selected key.
    Delete,
}

/// What one panel key asked the host to apply. The panel edits the sheet it is
/// handed and reports the change; it never calls a command service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SheetEdit {
    /// A key now sits at this joint and tick.
    Insert {
        /// Track joint.
        joint: u16,
        /// Key tick.
        tick: u32,
    },
    /// The key at this joint and tick left the sheet.
    Delete {
        /// Track joint.
        joint: u16,
        /// Key tick.
        tick: u32,
    },
    /// The key was refused; the text is the panel row to draw.
    Refused {
        /// Display line of the refusal.
        text: String,
    },
}

/// Timeline panel state: a key cursor, a playhead and the last refusal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SheetPanel {
    cursor: usize,
    playhead: u32,
    notice: Option<String>,
}

impl SheetPanel {
    /// Fresh panel: cursor on the first row, playhead at `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Row the cursor sits on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Playhead tick.
    pub fn playhead(&self) -> u32 {
        self.playhead
    }

    /// Last refusal line, drawn under the rows.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Move the cursor within `len` rows.
    pub fn move_cursor(&mut self, down: bool, len: usize) {
        crate::shell::panels::move_cursor(&mut self.cursor, down, len);
    }

    /// Move the playhead by `delta` ticks, clamped inside the clip.
    pub fn step_playhead(&mut self, delta: i32, ticks: u32) -> u32 {
        let highest = i64::from(ticks.saturating_sub(1));
        let next = i64::from(self.playhead) + i64::from(delta);
        self.playhead = u32::try_from(next.clamp(0, highest)).unwrap_or(self.playhead);
        self.playhead
    }

    /// Select the cursor row of a sheet as `(joint, tick)`.
    pub fn selected(&self, sheet: &Sheet) -> Option<(u16, u32)> {
        key_pairs(sheet).get(self.cursor).copied()
    }

    /// One panel key.
    ///
    /// Up/Down move the cursor, Left/Right move the playhead, `I` inserts the
    /// sampled value at the playhead for the selected joint, `Delete` removes
    /// the selected key. A refusal becomes a [`SheetEdit::Refused`] row and a
    /// notice, never a panic.
    pub fn press(&mut self, sheet: &mut Sheet, key: SheetKey) -> Option<SheetEdit> {
        match key {
            SheetKey::Up => {
                self.move_cursor(false, key_pairs(sheet).len());
                None
            }
            SheetKey::Down => {
                self.move_cursor(true, key_pairs(sheet).len());
                None
            }
            SheetKey::Left => {
                self.step_playhead(-1, sheet.ticks);
                None
            }
            SheetKey::Right => {
                self.step_playhead(1, sheet.ticks);
                None
            }
            SheetKey::Insert => {
                let joint = match self.selected(sheet) {
                    Some((joint, _)) => joint,
                    None => match sheet.tracks.first() {
                        Some(track) => track.joint,
                        None => {
                            return Some(self.refuse(SheetError::EmptyTracks.to_string()));
                        }
                    },
                };
                let track = sheet
                    .tracks
                    .iter()
                    .position(|track| track.joint == joint)
                    .unwrap_or(0);
                let value = match sheet.sample(track, self.playhead) {
                    Ok(value) => value,
                    Err(error) => return Some(self.refuse(error.to_string())),
                };
                let key = Key {
                    tick: self.playhead,
                    value_milli: value,
                    easing: Easing::Linear,
                };
                match sheet.insert_key(joint, key) {
                    Ok(()) => {
                        self.cursor = key_pairs(sheet)
                            .iter()
                            .position(|pair| *pair == (joint, self.playhead))
                            .unwrap_or(0);
                        Some(SheetEdit::Insert {
                            joint,
                            tick: self.playhead,
                        })
                    }
                    Err(error) => Some(self.refuse(error.to_string())),
                }
            }
            SheetKey::Delete => {
                let Some((joint, tick)) = self.selected(sheet) else {
                    let error = SheetError::UnknownJoint { joint: 0 };
                    return Some(self.refuse(error.to_string()));
                };
                match sheet.delete_key(joint, tick) {
                    Ok(_) => {
                        let len = key_pairs(sheet).len();
                        self.cursor = self.cursor.min(len.saturating_sub(1));
                        Some(SheetEdit::Delete { joint, tick })
                    }
                    Err(error) => Some(self.refuse(error.to_string())),
                }
            }
        }
    }

    /// Record a refusal and hand back its row.
    fn refuse(&mut self, text: String) -> SheetEdit {
        self.notice = Some(text.clone());
        SheetEdit::Refused { text }
    }
}

/// Every `(joint, tick)` pair in sheet order, matching [`Sheet::rows`].
pub fn key_pairs(sheet: &Sheet) -> Vec<(u16, u32)> {
    let mut out = Vec::new();
    for track in &sheet.tracks {
        for key in &track.keys {
            out.push((track.joint, key.tick));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tick: u32, value: i32, easing: Easing) -> Key {
        Key {
            tick,
            value_milli: [value; 3],
            easing,
        }
    }

    fn clip() -> Sheet {
        Sheet {
            name: "swing".to_string(),
            ticks: 60,
            tracks: vec![Track {
                joint: 3,
                keys: vec![
                    key(0, 0, Easing::Linear),
                    key(15, 300, Easing::Linear),
                    key(30, -200, Easing::Linear),
                    key(45, 900, Easing::Step),
                    key(59, 100, Easing::Linear),
                ],
            }],
        }
    }

    #[test]
    fn unsorted_or_duplicate_keys_refuse() {
        let mut sheet = clip();
        sheet.sorted().unwrap();
        sheet.tracks[0].keys.swap(1, 2);
        assert_eq!(
            sheet.sorted(),
            Err(SheetError::KeysOutOfOrder { joint: 3, at: 2 })
        );
        let mut sheet = clip();
        sheet.tracks[0].keys[2].tick = 15;
        assert_eq!(
            sheet.sorted(),
            Err(SheetError::DuplicateTick { joint: 3, tick: 15 })
        );
        let mut sheet = clip();
        sheet.tracks[0].keys[4].tick = 60;
        assert_eq!(
            sheet.sorted(),
            Err(SheetError::TickPastClip {
                joint: 3,
                tick: 60,
                ticks: 60
            })
        );
        let mut sheet = clip();
        sheet.tracks.push(Track {
            joint: 3,
            keys: vec![key(1, 0, Easing::Linear)],
        });
        assert_eq!(sheet.sorted(), Err(SheetError::DuplicateJoint { joint: 3 }));
    }

    #[test]
    fn sampling_holds_at_the_ends_and_eases_between() {
        let sheet = clip();
        assert_eq!(sheet.sample(0, 0).unwrap(), [0; 3], "first key");
        assert_eq!(sheet.sample(0, 59).unwrap(), [100; 3], "last key");
        assert_eq!(sheet.sample(0, 60).unwrap(), [100; 3], "past the clip");
        // Linear 0 -> 300 over 15 ticks: t=5 is a third.
        assert_eq!(sheet.sample(0, 5).unwrap(), [100; 3]);
        // Exactly on a key returns it.
        assert_eq!(sheet.sample(0, 30).unwrap(), [-200; 3]);
        // Linear -200 -> 900 over 15 ticks: t=40 is two thirds along.
        assert_eq!(sheet.sample(0, 40).unwrap(), [533; 3]);
        // Step holds 900 from its key until the next one.
        assert_eq!(sheet.sample(0, 46).unwrap(), [900; 3]);
        assert_eq!(sheet.sample(0, 58).unwrap(), [900; 3]);
        assert_eq!(
            sheet.sample(1, 10),
            Err(SheetError::UnknownTrack { index: 1 })
        );
    }

    #[test]
    fn bezier_on_the_diagonal_replays_linear() {
        let linear = Sheet {
            name: "lin".to_string(),
            ticks: 10,
            tracks: vec![Track {
                joint: 0,
                keys: vec![key(0, 0, Easing::Linear), key(10, 1000, Easing::Linear)],
            }],
        };
        let curved = Sheet {
            name: "bez".to_string(),
            ticks: 10,
            tracks: vec![Track {
                joint: 0,
                keys: vec![
                    key(
                        0,
                        0,
                        Easing::Bezier {
                            cx1_milli: 250,
                            cy1_milli: 250,
                            cx2_milli: 750,
                            cy2_milli: 750,
                        },
                    ),
                    key(10, 1000, Easing::Linear),
                ],
            }],
        };
        for tick in 0..=10 {
            assert_eq!(
                curved.sample(0, tick).unwrap(),
                linear.sample(0, tick).unwrap(),
                "diagonal bezier equals linear at t{tick}"
            );
        }
        // Ease-in is below the line, ease-out above it.
        let mut eased = linear.clone();
        eased.tracks[0].keys[0].easing = Easing::EaseIn;
        assert!(eased.sample(0, 5).unwrap()[0] < 500);
        eased.tracks[0].keys[0].easing = Easing::EaseOut;
        assert!(eased.sample(0, 5).unwrap()[0] > 500);
    }

    #[test]
    fn region_keeps_boundary_keys_and_same_samples() {
        let sheet = clip();
        assert_eq!(
            sheet.region(20, 20),
            Err(SheetError::BadRegion {
                from: 20,
                to: 20,
                ticks: 60
            })
        );
        let region = sheet.region(10, 20).unwrap();
        assert_eq!(region.ticks, 11, "both cut ticks are inside the region");
        assert_eq!(region.name, "swing[10..20]");
        let keys = &region.tracks[0].keys;
        assert_eq!(keys.first().unwrap().tick, 0, "the left cut key");
        assert_eq!(keys.last().unwrap().tick, 10, "the right cut key");
        assert_eq!(keys.len(), 3, "the cut key plus the source key at t15");
        for tick in 10..=20 {
            let region_value = region.sample(0, tick - 10).unwrap();
            let source = sheet.sample(0, tick).unwrap();
            for (region_axis, source_axis) in region_value.iter().zip(source.iter()) {
                assert!(
                    (region_axis - source_axis).abs() <= 1,
                    "the region replays t{tick} within one milli: {region_value:?} vs {source:?}"
                );
            }
        }
        // Two runs of the same edit agree byte for byte.
        assert_eq!(sheet.region(10, 20).unwrap().digest(), region.digest());
        assert_eq!(region.rows().len(), 3);
    }

    #[test]
    fn insert_and_delete_keys_refuse_duplicates_and_misses() {
        let mut sheet = clip();
        let digest = sheet.digest();
        assert_eq!(
            sheet.insert_key(3, key(15, 1, Easing::Linear)),
            Err(SheetError::DuplicateTick { joint: 3, tick: 15 })
        );
        assert_eq!(sheet.digest(), digest, "a refused insert changes nothing");
        sheet.insert_key(3, key(16, 42, Easing::EaseOut)).unwrap();
        assert_eq!(sheet.sample(0, 16).unwrap(), [42; 3]);
        assert_eq!(sheet.rows()[2], "joint 3  t0016  [42, 42, 42]  ease-out");
        assert_eq!(
            sheet.insert_key(9, key(1, 0, Easing::Linear)),
            Err(SheetError::UnknownJoint { joint: 9 })
        );
        let removed = sheet.delete_key(3, 16).unwrap();
        assert_eq!(removed.value_milli, [42; 3]);
        assert_eq!(
            sheet.delete_key(3, 16),
            Err(SheetError::UnknownKey { joint: 3, tick: 16 })
        );
        assert_eq!(sheet.digest(), digest, "delete restores the digest");
    }

    #[test]
    fn the_timeline_panel_edits_at_the_playhead_and_echoes_refusals() {
        let mut sheet = clip();
        let mut panel = SheetPanel::new();
        panel.press(&mut sheet, SheetKey::Right);
        panel.press(&mut sheet, SheetKey::Right);
        assert_eq!(panel.playhead(), 2, "playhead steps by one tick");
        // Cursor starts on the first row, so I inserts into joint 3.
        let edit = panel.press(&mut sheet, SheetKey::Insert).unwrap();
        assert_eq!(edit, SheetEdit::Insert { joint: 3, tick: 2 });
        assert_eq!(
            sheet.sample(0, 2).unwrap(),
            [40; 3],
            "sample held the value"
        );
        assert_eq!(
            panel.selected(&sheet),
            Some((3, 2)),
            "the new key is selected"
        );
        let edit = panel.press(&mut sheet, SheetKey::Delete).unwrap();
        assert_eq!(edit, SheetEdit::Delete { joint: 3, tick: 2 });
        assert_eq!(
            sheet.rows().len(),
            5,
            "insert then delete restores the rows"
        );
        // A duplicate tick refuses as a typed row and changes nothing.
        for _ in 0..10 {
            panel.press(&mut sheet, SheetKey::Left);
        }
        assert_eq!(panel.playhead(), 0);
        let edit = panel.press(&mut sheet, SheetKey::Insert).unwrap();
        assert!(matches!(edit, SheetEdit::Refused { .. }));
        assert!(panel.notice().unwrap().contains("already has a key"));
        assert_eq!(sheet.rows().len(), 5);
        // The playhead clamps inside the clip at both ends.
        for _ in 0..100 {
            panel.press(&mut sheet, SheetKey::Right);
        }
        assert_eq!(panel.playhead(), sheet.ticks - 1);
        // A sheet with no track refuses the insert with its own line.
        let mut empty = Sheet {
            name: "untitled".to_string(),
            ticks: 60,
            tracks: Vec::new(),
        };
        let edit = panel.press(&mut empty, SheetKey::Insert).unwrap();
        assert!(matches!(edit, SheetEdit::Refused { .. }));
        assert!(panel.notice().unwrap().contains("no track"));
    }
}
