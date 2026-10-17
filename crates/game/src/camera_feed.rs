//! camera_feed: presentation feed over the camera director.
//!
//! Consumer: the window frame loop calls [`PresentationFeed::win_frame`] once
//! per accepted fixed tick with the tick's [`TickOutcome`](super::TickOutcome).
//! Data owner: this module owns the feed (director plus the per-tick trauma
//! and frame receipts); match authority stays in `engine-arena`, tuning in
//! [`DirectorParams`](super::camera::DirectorParams). Units: meters, radians,
//! fixed ticks; trauma in permille (`0..=1000`). Determinism: same cue script
//! plus same ticks gives identical frame bytes twice; shake never feeds sim
//! state (the golden `d57b5c8ae3096614` stands). Errors:
//! [`CameraError`](super::camera::CameraError) from the frozen director API.
//! Budget: 0.05 ms per frame, zero heap bytes per update (counted test).

use core::f32::consts::PI;

use engine_audio::FightCue;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{hash_bytes, splitmix64};
use engine_render::present::MAX_FRAME_DIM;
use sha2::{Digest, Sha256};

use super::camera::{
    count_unframed, off_screen_arrows, shake_displacement_m, trauma_gain, unshaken_eye,
    CameraError, CameraFrame, DampedSync, DirectorParams, FighterView, LookIntent, OffScreenArrow,
    StagedRig, Vec3, BYPASS_LOCK, BYPASS_SHAKE,
};
use super::TickOutcome;

/// Rolling window length in receipts, in the unit the receipt buffers are
/// sized and compared in (`usize`). The window is owned once per unit: this
/// one for the buffers and the trim, [`FEED_TICKS`] as the public `u32` tick
/// count, with `feed_window_owner_agrees_with_the_public_tick_count` pinning
/// the pair. `From` has no `u32` → `usize` lane (`try_from` is not `const`
/// either), and a bare `as` at each site would hide the bound this pair
/// already states.
const FEED_WINDOW: usize = 600;
/// Ticks the acceptance orbit covers (matches the director's 600-tick gate).
pub const FEED_TICKS: u32 = 600;
/// Per-frame budget in milliseconds (the director's 0.05 ms rail).
pub const FEED_BUDGET_MS: f64 = 0.05;
/// Scripted orbit radius in meters (both fighters stay framable).
pub const ORBIT_RADIUS_M: f32 = 5.0;
/// Scripted orbit height in meters (feet plus 1 m).
pub const ORBIT_HEIGHT_M: f32 = 1.0;
/// Scripted orbit advance in radians per tick.
pub const ORBIT_STEP_RAD: f32 = 0.02;

/// In-sim trauma for a landed hit (permille).
pub const TRAUMA_HIT: u32 = 180;
/// In-sim trauma for a soaked block (chip only).
pub const TRAUMA_BLOCKED: u32 = 60;
/// In-sim trauma for a parry (no damage, attacker staggered).
pub const TRAUMA_PARRIED: u32 = 40;
/// In-sim trauma for a knockdown (victim pinned to zero).
pub const TRAUMA_KNOCKDOWN: u32 = 350;

/// Scripted two-fighter orbit for a tick: radius 5 m, opposite phases, 1 m
/// height. Presentation positions only, never sim input; the default director
/// keeps both framed every tick. Pure.
#[must_use]
pub fn orbit_views(tick: u32) -> [FighterView; 2] {
    let angle = tick as f32 * ORBIT_STEP_RAD;
    [
        FighterView {
            id: 0,
            pos_m: Vec3::new(
                ORBIT_RADIUS_M * angle.cos(),
                ORBIT_HEIGHT_M,
                ORBIT_RADIUS_M * angle.sin(),
            ),
            alive: true,
        },
        FighterView {
            id: 1,
            pos_m: Vec3::new(
                ORBIT_RADIUS_M * (angle + PI).cos(),
                ORBIT_HEIGHT_M,
                ORBIT_RADIUS_M * (angle + PI).sin(),
            ),
            alive: true,
        },
    ]
}

/// In-sim trauma for one fight cue, in permille. Integer mapping only, no
/// trig, no wall clock: the same cue script records the same trauma twice.
/// Clean evasions and stamina whiffs carry zero (nothing landed). Pure.
#[must_use]
pub fn trauma_for_cue(cue: FightCue) -> u32 {
    match cue {
        FightCue::Hit { .. } => TRAUMA_HIT,
        FightCue::Blocked { .. } => TRAUMA_BLOCKED,
        FightCue::Parried { .. } => TRAUMA_PARRIED,
        FightCue::Dodged { .. } | FightCue::StaminaOut { .. } => 0,
        FightCue::Knockdown { .. } => TRAUMA_KNOCKDOWN,
    }
}

/// In-sim trauma for a tick's cue slice: saturating sum of
/// [`trauma_for_cue`] in order (bursts add, the director clamps at full).
/// Pure.
#[must_use]
pub fn trauma_for_cues(cues: &[FightCue]) -> u32 {
    let mut total = 0u32;
    for cue in cues {
        total = total.saturating_add(trauma_for_cue(*cue));
    }
    total
}

/// Presentation feed: the staged rig over the frozen arena director, driven
/// once per fixed tick. Trauma is recorded in-sim (integer permille folded
/// from the tick's cues before the update, replaying exactly); shake is
/// presentation-side (the rig's pure shake stage moves the presented eye,
/// never sim state). With no stage bypassed the rig serves byte-identical
/// frames to the raw director. Frame and trauma receipts are pre-sized at
/// construction, then trimmed to the rolling [`FEED_TICKS`] window, so
/// steady-state updates allocate nothing and long sessions retain the
/// latest window instead of the whole run.
pub struct PresentationFeed {
    rig: StagedRig,
    frames: Vec<CameraFrame>,
    trauma: Vec<u32>,
}

impl PresentationFeed {
    /// Build a feed following `focus` under `params`. Receipt buffers are
    /// pre-sized for [`FEED_TICKS`] ticks at the construction boundary
    /// (never per tick).
    pub fn new(focus: u8, params: DirectorParams) -> Result<Self, CameraError> {
        // One slot past the window: each push lands before the head-drop.
        let capacity = FEED_WINDOW + 1;
        Ok(Self {
            rig: StagedRig::new(focus, params)?,
            frames: Vec::with_capacity(capacity),
            trauma: Vec::with_capacity(capacity),
        })
    }

    /// Followed fighter id.
    #[must_use]
    pub fn focus_id(&self) -> u8 {
        self.rig.focus_id()
    }

    /// Presented frames in tick order.
    #[must_use]
    pub fn frames(&self) -> &[CameraFrame] {
        &self.frames
    }

    /// Drop receipts past the rolling [`FEED_TICKS`] window. At most one
    /// entry overflows per tick, so one head-drop keeps the latest window
    /// with no allocation.
    fn trim_receipts(&mut self) {
        if self.frames.len() > FEED_WINDOW {
            self.frames.remove(0);
            self.trauma.remove(0);
        }
    }

    /// Recorded in-sim trauma per presented tick, in tick order.
    #[must_use]
    pub fn trauma_log(&self) -> &[u32] {
        &self.trauma
    }

    /// Frames presented so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no frame has been presented yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Last presented frame, if any tick ran.
    #[must_use]
    pub fn last_frame(&self) -> Option<CameraFrame> {
        self.frames.last().copied()
    }

    /// Present one fixed tick: fold the tick's cues into in-sim trauma, then
    /// run the staged rig over `views` with no stage bypassed. Ticks run
    /// forward only; every position is validated, so a NaN fails loudly
    /// instead of hiding. Borrows plus pre-sized pushes only, no heap per
    /// call at steady state.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        views: &[FighterView],
        cues: &[FightCue],
    ) -> Result<CameraFrame, CameraError> {
        self.rig.set_bypass(0);
        self.rig.add_trauma(trauma_for_cues(cues));
        let frame = self.rig.update(views, tick)?;
        debug_assert_eq!(
            feed_trauma_gain(frame.trauma_permille),
            trauma_gain(frame.trauma_permille),
            "feed curve matches rig"
        );
        debug_assert_eq!(
            feed_shake_displacement_m(&frame),
            shake_displacement_m(&frame),
            "feed displacement matches rig"
        );
        debug_assert_eq!(
            feed_restored_eye(&frame),
            unshaken_eye(&frame),
            "feed restore matches rig"
        );
        self.frames.push(frame);
        self.trauma.push(frame.trauma_permille);
        self.trim_receipts();
        Ok(frame)
    }

    /// Present one scripted-orbit tick: [`orbit_views`] framing plus the
    /// tick's live cues for trauma. The window loop calls this per fixed tick
    /// so the acceptance orbits stay framed while real hits shake.
    pub fn orbit_tick(&mut self, tick: u32, cues: &[FightCue]) -> Result<CameraFrame, CameraError> {
        let views = orbit_views(tick);
        self.observe_tick(tick, &views, cues)
    }

    /// Present one window frame from its tick outcome: orbit framing at the
    /// outcome tick plus the outcome's cues for trauma. One call per accepted
    /// fixed tick; rejected ticks never reach the feed (same discipline as
    /// the crowd clock).
    pub fn win_frame(&mut self, outcome: &TickOutcome) -> Result<CameraFrame, CameraError> {
        self.orbit_tick(outcome.tick, &outcome.cues)
    }

    /// GP19 staged tick: fold the tick's cues into in-sim trauma, then serve
    /// the frame through the staged rig with the bypassed reporting stages
    /// dropped (lock and/or shake). Receipts and sim handling match
    /// [`Self::observe_tick`]; the unshaken pose still frames, so bypassing
    /// a stage loses no framed tick.
    pub fn observe_tick_with_bypass(
        &mut self,
        tick: u32,
        views: &[FighterView],
        cues: &[FightCue],
        bypass: u8,
    ) -> Result<CameraFrame, CameraError> {
        self.rig.set_bypass(bypass & (BYPASS_LOCK | BYPASS_SHAKE));
        self.rig.add_trauma(trauma_for_cues(cues));
        let frame = self.rig.update(views, tick)?;
        debug_assert_eq!(
            feed_trauma_gain(frame.trauma_permille),
            trauma_gain(frame.trauma_permille),
            "feed curve matches rig"
        );
        debug_assert_eq!(
            feed_shake_displacement_m(&frame),
            shake_displacement_m(&frame),
            "feed displacement matches rig"
        );
        debug_assert_eq!(
            feed_restored_eye(&frame),
            unshaken_eye(&frame),
            "feed restore matches rig"
        );
        self.frames.push(frame);
        self.trauma.push(frame.trauma_permille);
        self.trim_receipts();
        Ok(frame)
    }

    /// GP19 staged orbit tick: [`orbit_views`] framing plus bypassed stages.
    pub fn orbit_tick_with_bypass(
        &mut self,
        tick: u32,
        cues: &[FightCue],
        bypass: u8,
    ) -> Result<CameraFrame, CameraError> {
        let views = orbit_views(tick);
        self.observe_tick_with_bypass(tick, &views, cues, bypass)
    }

    /// Staged window frame: orbit framing at the outcome tick with
    /// bypassed stages plus the outcome's cues for trauma.
    pub fn win_frame_with_bypass(
        &mut self,
        outcome: &TickOutcome,
        bypass: u8,
    ) -> Result<CameraFrame, CameraError> {
        self.orbit_tick_with_bypass(outcome.tick, &outcome.cues, bypass)
    }

    /// Fighters outside the presented `frame`: `0` means every fighter is
    /// framed, any remainder carries an [`OffScreenArrow`] (see
    /// [`Self::edge_arrows`]). Thin delegate over the camera oracle under
    /// the feed's tuning table, so the window loop reads framing status
    /// from the same feed it already drives once per tick. Pure over the
    /// frame plus views: same inputs give the same answer twice, and
    /// per-fighter answers never depend on slice order. Errors: bad tuning
    /// names its row, a non-finite position names its fighter id.
    pub fn unframed_count(
        &self,
        frame: &CameraFrame,
        views: &[FighterView],
    ) -> Result<usize, CameraError> {
        count_unframed(frame, views, &self.rig.params())
    }

    /// Edge arrows for the presented `frame`: writes one [`OffScreenArrow`]
    /// per unframed fighter into `out` in slice order and returns the count
    /// written. `out` must hold at least `views.len()` slots; a short
    /// buffer fails with [`CameraError::BadParam`] naming `arrows` before
    /// any framing work. Pure, slices in and out, no heap.
    pub fn edge_arrows(
        &self,
        frame: &CameraFrame,
        views: &[FighterView],
        out: &mut [OffScreenArrow],
    ) -> Result<usize, CameraError> {
        off_screen_arrows(frame, views, &self.rig.params(), out)
    }
}

/// Smoothed feed: the [`PresentationFeed`] plus the
/// intent/smoothed split. The raw frame (intent) is recorded every tick;
/// [`DampedSync`] folds it into the smoothed scene transform at the
/// presentation rate only, so 30/60/120 Hz sync strides share every sim byte.
/// Receipt buffers are pre-sized at construction, so steady-state updates
/// allocate nothing.
pub struct SmoothedFeed {
    inner: PresentationFeed,
    sync: DampedSync,
    smoothed: Vec<LookIntent>,
}

impl SmoothedFeed {
    /// Build a smoothed feed following `focus` under `params` with per-sync
    /// gain `1..=1000` (1000 snaps every sync). Receipt buffers are pre-sized
    /// for [`FEED_TICKS`] ticks at the construction boundary (never per tick).
    pub fn new(focus: u8, params: DirectorParams, gain_permille: u32) -> Result<Self, CameraError> {
        Ok(Self {
            inner: PresentationFeed::new(focus, params)?,
            sync: DampedSync::new(gain_permille)?,
            // One slot past the window: each push lands before the head-drop.
            smoothed: Vec::with_capacity(FEED_WINDOW + 1),
        })
    }

    /// Per-sync gain permille.
    #[must_use]
    pub fn gain_permille(&self) -> u32 {
        self.sync.gain_permille()
    }

    /// Raw presented frames in tick order (the intent source).
    #[must_use]
    pub fn frames(&self) -> &[CameraFrame] {
        self.inner.frames()
    }

    /// Recorded in-sim trauma per presented tick, in tick order.
    #[must_use]
    pub fn trauma_log(&self) -> &[u32] {
        self.inner.trauma_log()
    }

    /// Smoothed scene transforms in tick order, one per windowed tick and
    /// aligned with `frames`.
    #[must_use]
    pub fn smoothed(&self) -> &[LookIntent] {
        &self.smoothed
    }

    /// Ticks presented so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True when no tick ran yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Present one fixed tick and fold its intent into the smoothed
    /// transform. Borrows plus pre-sized pushes only, no heap per call at
    /// steady state.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        views: &[FighterView],
        cues: &[FightCue],
    ) -> Result<(CameraFrame, LookIntent), CameraError> {
        let frame = self.inner.observe_tick(tick, views, cues)?;
        let intent = LookIntent::from_frame(&frame);
        let smoothed = self.sync.sync(&intent);
        self.smoothed.push(smoothed);
        // The frames this log parallels are a rolling `FEED_WINDOW` window:
        // keep the smoothed log the same length with the frame trim owner,
        // so the report's frame/smoothed zip stays aligned and the pre-sized
        // capacity never regrows.
        while self.smoothed.len() > self.inner.len() {
            self.smoothed.remove(0);
        }
        Ok((frame, smoothed))
    }

    /// Present one scripted-orbit tick plus its smoothed transform.
    pub fn orbit_tick(
        &mut self,
        tick: u32,
        cues: &[FightCue],
    ) -> Result<(CameraFrame, LookIntent), CameraError> {
        let views = orbit_views(tick);
        self.observe_tick(tick, &views, cues)
    }

    /// Present one window frame from its tick outcome, plus the smoothed
    /// transform. One call per accepted fixed tick.
    pub fn win_frame(
        &mut self,
        outcome: &TickOutcome,
    ) -> Result<(CameraFrame, LookIntent), CameraError> {
        self.orbit_tick(outcome.tick, &outcome.cues)
    }
}

/// Shake summary over presented frames: the numbers the playtester
/// confirms (small-hit vs super displacement separation). Pure booking over
/// a frame slice; no heap.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShakeReport {
    /// Frames covered.
    pub ticks: u32,
    /// Peak shake displacement in meters.
    pub max_displacement_m: f32,
    /// Mean shake displacement in meters.
    pub mean_displacement_m: f32,
    /// Peak trauma permille observed.
    pub max_trauma_permille: u32,
    /// Squared trauma gain at peak trauma (`0..=1`).
    pub peak_gain: f32,
}

/// Summarize one frame slice as a [`ShakeReport`]: peak and mean presented
/// displacement plus the peak trauma and its curve gain. Empty slices report
/// zeros; all math is presentation-only.
#[must_use]
pub fn shake_report(frames: &[CameraFrame]) -> ShakeReport {
    let mut peak = 0.0f32;
    let mut sum = 0.0f64;
    let mut peak_trauma = 0u32;
    for frame in frames {
        let displacement = feed_shake_displacement_m(frame);
        if displacement > peak {
            peak = displacement;
        }
        sum += f64::from(displacement);
        if frame.trauma_permille > peak_trauma {
            peak_trauma = frame.trauma_permille;
        }
    }
    let count = frames.len() as f64;
    ShakeReport {
        ticks: engine_core::checked::saturating_usize_to_u32(frames.len()),
        max_displacement_m: peak,
        mean_displacement_m: if count > 0.0 {
            (sum / count) as f32
        } else {
            0.0
        },
        max_trauma_permille: peak_trauma,
        peak_gain: feed_trauma_gain(peak_trauma),
    }
}

/// Feed-local trauma response curve: squared trauma gain (`t * t`), so a jab
/// moves far less than a super. Owns the GP21 shake stage beside the rig:
/// same clamped permille math as the staged rig, presentation-only, never
/// sim state. Same inputs give the same gain twice. Pure, no heap.
#[must_use]
pub fn feed_trauma_gain(trauma_permille: u32) -> f32 {
    let clamped = trauma_permille.min(1000);
    let t = f32::from(u16::try_from(clamped).unwrap_or(1000)) / 1000.0;
    t * t
}

/// Feed-local shake displacement in meters for one presented frame: length
/// of the presented shake offset. Presentation-only, never sim state.
/// Same frame gives the same length twice. Pure, no heap.
#[must_use]
pub fn feed_shake_displacement_m(frame: &CameraFrame) -> f32 {
    frame.shake_m.len()
}

/// Feed-local restore of the unshaken pose eye from a presented frame: the
/// shake offset is presentation-only, so removing it returns the pose eye
/// the next tick selects from (within one float rounding step). Same frame
/// gives the same eye twice. Pure, no heap.
#[must_use]
pub fn feed_restored_eye(frame: &CameraFrame) -> Vec3 {
    frame.eye_m - frame.shake_m
}

// ---------------------------------------------------------------------------
// Wired game capture: the single pixel supplier for `capture_with` receipts.
// ---------------------------------------------------------------------------

/// Most PNG bytes one game capture receipt may carry: the 4 MB receipt
/// budget from the eyes plan (E1), enforced here after encoding.
pub const GAME_CAPTURE_MAX_BYTES: usize = 4 << 20;
/// Frames in the EV20 gold showcase set (seed-fixed orbit, sha-pinned).
pub const GAME_SHOWCASE_FRAMES: u32 = 15;
/// Renderer label the software path files in showcase manifests. Receipts
/// say what ran, where, so reference-GPU runs file their adapter instead.
pub const GAME_CAPTURE_RENDERER: &str = "game-camera-feed";
/// PNG file magic: every receipt starts with these 8 bytes.
const GAME_PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// Largest stored-deflate block the PNG writer emits (zlib ceiling).
const GAME_PNG_BLOCK_MAX: usize = 65535;
/// Adler-32 chunking bound from zlib (block sums stay below 2^32).
const GAME_ADLER_CHUNK: usize = 5552;

/// Validate capture sides: zero names its side, over the presenter bound
/// names the size (the render owner enforces 2048 there too).
fn check_capture_dims(width: u32, height: u32) -> Result<(), CameraError> {
    if width == 0 {
        return Err(CameraError::BadParam("capture_width"));
    }
    if height == 0 {
        return Err(CameraError::BadParam("capture_height"));
    }
    if width > MAX_FRAME_DIM || height > MAX_FRAME_DIM {
        return Err(CameraError::BadParam("capture_size"));
    }
    Ok(())
}

/// Narrow a pixel-domain `u64` to `u8`: every call site holds a value at
/// or under 255 by construction, so out-of-range fails loudly with the
/// pixel row instead of wrapping. Pure.
fn narrow_u8(value: u64) -> Result<u8, CameraError> {
    u8::try_from(value).map_err(|_| CameraError::BadParam("capture_pixels"))
}

/// Paint the two trauma-keyed state discs over the raster: calm sets read
/// pale, loud sets read hot. Seats come from the frame stream; seats past
/// the edge clip per pixel. Slices in and out, no heap. Errors:
/// [`CameraError::BadParam`] naming `capture_size` (every narrowing holds
/// for validated sides).
fn paint_discs(
    rgb: &mut [u8],
    w: usize,
    h: usize,
    discs: &[(u64, u64)],
    trauma: u64,
) -> Result<(), CameraError> {
    let narrow_side =
        |side: usize| i32::try_from(side).map_err(|_| CameraError::BadParam("capture_size"));
    let wi = narrow_side(w)?;
    let hi = narrow_side(h)?;
    let rad = narrow_side((w.min(h) / 7).max(3))?;
    let heat = narrow_u8(trauma * 135 / 1000)?;
    for (index, (px, py)) in discs.iter().enumerate() {
        let core = if index == 0 {
            [232u8, 228u8, 220u8]
        } else {
            [200u8, 200u8, 210u8]
        };
        let pxi = i32::try_from(*px).map_err(|_| CameraError::BadParam("capture_size"))?;
        let pyi = i32::try_from(*py).map_err(|_| CameraError::BadParam("capture_size"))?;
        for dy in -rad..=rad {
            for dx in -rad..=rad {
                let d2 = dx * dx + dy * dy;
                if d2 > rad * rad {
                    continue;
                }
                let x = pxi + dx;
                let y = pyi + dy;
                if x < 0 || y < 0 || x >= wi || y >= hi {
                    continue;
                }
                let at = (usize::try_from(y).map_err(|_| CameraError::BadParam("capture_size"))?
                    * w
                    + usize::try_from(x).map_err(|_| CameraError::BadParam("capture_size"))?)
                    * 3;
                if d2 * 4 > rad * rad {
                    rgb[at] = 120u8.saturating_add(heat);
                    rgb[at + 1] = 40;
                    rgb[at + 2] = 40;
                } else {
                    rgb[at] = core[0];
                    rgb[at + 1] = core[1];
                    rgb[at + 2] = core[2];
                }
            }
        }
    }
    Ok(())
}

/// Paint the tick/digest word along the top row so consecutive frames never
/// render identical. Pure over the buffer prefix, no heap.
fn paint_tick_bits(rgb: &mut [u8], w: usize, tick: u32, digest: u64, seed: u64) {
    let bits = u64::from(tick) ^ digest ^ seed;
    for b in 0..32.min(w) {
        let at = b * 3;
        if (bits >> b) & 1 == 1 {
            rgb[at] = 255;
            rgb[at + 1] = 255;
            rgb[at + 2] = 255;
        } else {
            rgb[at] = 8;
            rgb[at + 1] = 8;
            rgb[at + 2] = 10;
        }
    }
}

/// Render the live presented `frame` into `rgb` (RGB8, row-major): slate
/// gradient plus checker keyed by the shaken eye and look, two bright discs
/// placed by the state stream, a trauma vignette so shake state reads in
/// the pixels, and the tick/digest bits along the top row so consecutive
/// frames never render identical (a frozen capture fails the showcase gate
/// instead of passing as clean). Integer math over the stream plus
/// quantized pose only; no wall clock, no threads. `rgb` must hold exactly
/// `width * height * 3` bytes. Pure, slices in and out, no heap. Errors:
/// [`CameraError::BadParam`] naming `capture_width`, `capture_height`,
/// `capture_size` or `capture_pixels`.
pub fn render_capture_rgb(
    frame: &CameraFrame,
    tick: u32,
    digest: u64,
    seed: u64,
    width: u32,
    height: u32,
    rgb: &mut [u8],
) -> Result<(), CameraError> {
    check_capture_dims(width, height)?;
    let w = usize::try_from(width).map_err(|_| CameraError::BadParam("capture_width"))?;
    let h = usize::try_from(height).map_err(|_| CameraError::BadParam("capture_height"))?;
    if rgb.len() != w * h * 3 {
        return Err(CameraError::BadParam("capture_pixels"));
    }
    // One stream per frame: the presented bytes bind the camera, the digest
    // binds the sim, the tick orders the set. Same inputs replay the same
    // pixels twice; any bit of difference paints different pixels.
    let mut rng =
        seed ^ digest ^ u64::from(tick).wrapping_mul(GOLDEN_GAMMA) ^ hash_bytes(&frame.to_bytes());
    let tiny = w < 16 || h < 16;
    let mut discs = [(0u64, 0u64); 2];
    let w64 = u64::try_from(w).map_err(|_| CameraError::BadParam("capture_size"))?;
    let h64 = u64::try_from(h).map_err(|_| CameraError::BadParam("capture_size"))?;
    if !tiny {
        for disc in discs.iter_mut() {
            let draw_x = splitmix64(rng);
            rng = rng.wrapping_add(GOLDEN_GAMMA);
            let draw_y = splitmix64(rng);
            rng = rng.wrapping_add(GOLDEN_GAMMA);
            *disc = (6 + draw_x % (w64 - 12), 6 + draw_y % (h64 - 12));
        }
    }
    let cx64 = w64 / 2;
    let cy64 = h64 / 2;
    let denom = cx64 * cx64 + cy64 * cy64 + 1;
    let trauma = u64::from(frame.trauma_permille.min(1000));
    for y in 0..h {
        let y64 = u64::try_from(y).map_err(|_| CameraError::BadParam("capture_pixels"))?;
        for x in 0..w {
            let x64 = u64::try_from(x).map_err(|_| CameraError::BadParam("capture_pixels"))?;
            let base = 16 + y64 * 36 / h64;
            let checker = if ((x / 8) + (y / 8)) % 2 == 1 { 8 } else { 0 };
            let dx = x64.abs_diff(cx64);
            let dy = y64.abs_diff(cy64);
            let vig = ((dx * dx + dy * dy) * trauma * 24 / (denom * 1000)).min(255);
            let luma = narrow_u8(base + checker)?.saturating_sub(narrow_u8(vig)?);
            let at = (y * w + x) * 3;
            rgb[at] = luma;
            rgb[at + 1] = luma.saturating_add(4);
            rgb[at + 2] = luma.saturating_add(16);
        }
    }
    if !tiny {
        paint_discs(rgb, w, h, &discs, trauma)?;
    }
    paint_tick_bits(rgb, w, tick, digest, seed);
    Ok(())
}

/// CRC-32 over PNG chunk bytes (tag plus body), bitwise, no tables.
fn game_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Adler-32 over the zlib payload, chunked like the sink codec so block
/// sums stay below 2^32.
fn game_adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for piece in data.chunks(GAME_ADLER_CHUNK) {
        for byte in piece {
            a += u32::from(*byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Append one PNG chunk (length plus tag plus body plus CRC). Errors:
/// [`CameraError::BadParam`] naming `capture_bytes` (chunk lengths always
/// fit for validated sides).
fn game_push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) -> Result<(), CameraError> {
    let len = u32::try_from(body.len()).map_err(|_| CameraError::BadParam("capture_bytes"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut key = Vec::with_capacity(4 + body.len());
    key.extend_from_slice(tag);
    key.extend_from_slice(body);
    out.extend_from_slice(&game_crc32(&key).to_be_bytes());
    Ok(())
}

/// Encode RGB8 rows as a PNG (8-bit, filter 0, stored deflate, one IDAT):
/// the same pixels give the same bytes on every machine. Same stored
/// shape as the capture sink codec, which validates the same magic plus
/// IHDR; the sink stays the only writer to disk. Errors:
/// [`CameraError::BadParam`] naming `capture_width`, `capture_height`,
/// `capture_size` or `capture_pixels`.
pub fn encode_capture_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, CameraError> {
    check_capture_dims(width, height)?;
    let w = usize::try_from(width).map_err(|_| CameraError::BadParam("capture_width"))?;
    let h = usize::try_from(height).map_err(|_| CameraError::BadParam("capture_height"))?;
    if rgb.len() != w * h * 3 {
        return Err(CameraError::BadParam("capture_pixels"));
    }
    let mut raw = Vec::with_capacity(h * (1 + w * 3));
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * w * 3..(y + 1) * w * 3]);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(GAME_PNG_BLOCK_MAX).len().max(1);
    for (index, piece) in raw.chunks(GAME_PNG_BLOCK_MAX).enumerate() {
        zlib.push(u8::from(index + 1 == blocks));
        let len = u16::try_from(piece.len()).map_err(|_| CameraError::BadParam("capture_bytes"))?;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(piece);
    }
    zlib.extend_from_slice(&game_adler32(&raw).to_be_bytes());
    let mut out = Vec::with_capacity(GAME_PNG_MAGIC.len() + 64 + zlib.len());
    out.extend_from_slice(&GAME_PNG_MAGIC);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    game_push_chunk(&mut out, b"IHDR", &ihdr)?;
    game_push_chunk(&mut out, b"IDAT", &zlib)?;
    game_push_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

/// Capture one presented frame as PNG bytes: the single game capture path
/// into `capture_with` receipts. Pilots wrap this in their one-shot hook
/// callback (asked size in, these bytes out); the endpoint validates magic
/// plus dims plus the byte bound, and the capture sink stays the only
/// writer to disk, so no second path can drift. Same frame plus tick plus
/// digest plus seed gives the same bytes twice; a new tick, digest, eye or
/// trauma gives new bytes. Service path, never the fixed tick: one RGB
/// frame plus one PNG live at a time. Units: pixels, bytes. Errors:
/// [`CameraError::BadParam`] naming the culprit row (`capture_width`,
/// `capture_height`, `capture_size`, `capture_bytes`).
pub fn game_capture_png(
    frame: &CameraFrame,
    tick: u32,
    digest: u64,
    seed: u64,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CameraError> {
    check_capture_dims(width, height)?;
    let count = usize::try_from(width).map_err(|_| CameraError::BadParam("capture_width"))?
        * usize::try_from(height).map_err(|_| CameraError::BadParam("capture_height"))?;
    let mut rgb = vec![0u8; count * 3];
    render_capture_rgb(frame, tick, digest, seed, width, height, &mut rgb)?;
    let png = encode_capture_png(width, height, &rgb)?;
    if png.len() > GAME_CAPTURE_MAX_BYTES {
        return Err(CameraError::BadParam("capture_bytes"));
    }
    Ok(png)
}

/// One sha-pinned showcase row: tick plus file hash plus byte length.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShowcaseEntry {
    /// Fixed tick the frame presents.
    pub tick: u32,
    /// SHA-256 over the exact PNG bytes (per-file tamper binding).
    pub sha256: [u8; 32],
    /// PNG byte length.
    pub bytes: u64,
}

/// Showcase set fields for [`showcase_manifest`]: seed plus tape plus rows
/// plus build SHA plus fps plus frame size plus the sha-pinned rows. One
/// struct because the manifest takes more than seven fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShowcaseSet<'a> {
    /// Session seed the frames were captured under.
    pub seed: u64,
    /// Tape identity (file name, never an absolute path).
    pub tape: &'a str,
    /// Board rows the set evidences (non-empty valid IDs).
    pub rows: &'a [&'a str],
    /// Build SHA hex (7-128 chars).
    pub sha: &'a str,
    /// Playback fps (`1..=120`).
    pub fps: u32,
    /// Frame width in pixels (uniform across the set).
    pub width: u32,
    /// Frame height in pixels (uniform across the set).
    pub height: u32,
    /// One row per frame, in `frame_{:06}.png` order.
    pub frames: &'a [ShowcaseEntry],
}

/// SHA-256 over PNG bytes for a showcase row (the `sha2` dependency owns
/// the digest; never hand-rolled crypto). Same bytes give the same hash on
/// every machine. Pure.
#[must_use]
pub fn showcase_sha256(png: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(png);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Lowercase hex of bytes into `out` (manifest rows, test bindings).
fn hex_into(out: &mut String, bytes: &[u8]) {
    const DIGITS: &[char; 16] = &[
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)]);
        out.push(DIGITS[usize::from(byte & 0xF)]);
    }
}

/// Escape one manifest string (quotes, backslashes, control chars).
fn showcase_escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ if u32::from(c) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
            _ => out.push(c),
        }
    }
}

/// One board-row ID: 1-4 uppercase letters, 1-3 digits, optional lowercase
/// suffix (the stitch validator contract, same shape as the sink checks).
fn check_showcase_row(id: &str) -> bool {
    let letters = id.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let rest = &id[letters..];
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    let tail = &rest[digits..];
    (1..=4).contains(&letters)
        && (1..=3).contains(&digits)
        && (tail.is_empty() || (tail.len() == 1 && tail.chars().all(|c| c.is_ascii_lowercase())))
        && id.len() == letters + digits + tail.len()
}

/// Build SHA hex for the manifest: 7-128 hex chars, no `0x`.
fn check_showcase_sha(sha: &str) -> bool {
    let sha = sha.trim();
    (7..=128).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit())
}

/// Build the showcase `manifest.json` for one sha-pinned set: fixed field
/// order, no timestamps, file names `frame_{:06}.png` in row order, so the
/// same rows give byte-identical manifests twice. The stitch tool
/// (`tools/showcase.mjs`) cross-checks every byte; an empty row list fails
/// closed here before anything is written. Errors:
/// [`CameraError::BadParam`] naming `frames`, `rows`, `sha`, `fps`,
/// `tape` or `capture_size`.
pub fn showcase_manifest(set: &ShowcaseSet<'_>) -> Result<String, CameraError> {
    if set.frames.is_empty() {
        return Err(CameraError::BadParam("frames"));
    }
    if set.rows.is_empty() || set.rows.iter().any(|row| !check_showcase_row(row)) {
        return Err(CameraError::BadParam("rows"));
    }
    if !check_showcase_sha(set.sha) {
        return Err(CameraError::BadParam("sha"));
    }
    if !(1..=120).contains(&set.fps) {
        return Err(CameraError::BadParam("fps"));
    }
    if set.tape.is_empty() || set.tape.len() > 256 {
        return Err(CameraError::BadParam("tape"));
    }
    check_capture_dims(set.width, set.height)?;
    let mut out = String::from("{\"schema\":1");
    out.push_str(",\"renderer\":\"");
    out.push_str(GAME_CAPTURE_RENDERER);
    out.push_str("\",\"seed\":");
    out.push_str(&set.seed.to_string());
    out.push_str(",\"tape\":\"");
    showcase_escape_into(&mut out, set.tape);
    out.push_str("\",\"rows\":[");
    for (index, row) in set.rows.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(row);
        out.push('"');
    }
    out.push_str("],\"sha\":\"");
    out.push_str(set.sha.trim());
    out.push_str("\",\"fps\":");
    out.push_str(&set.fps.to_string());
    out.push_str(",\"width\":");
    out.push_str(&set.width.to_string());
    out.push_str(",\"height\":");
    out.push_str(&set.height.to_string());
    out.push_str(",\"frames\":[");
    for (index, row) in set.frames.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"tick\":");
        out.push_str(&row.tick.to_string());
        out.push_str(",\"file\":\"");
        out.push_str(&format!("frame_{index:06}.png"));
        out.push_str("\",\"width\":");
        out.push_str(&set.width.to_string());
        out.push_str(",\"height\":");
        out.push_str(&set.height.to_string());
        out.push_str(",\"sha256\":\"");
        hex_into(&mut out, &row.sha256);
        out.push_str("\",\"bytes\":");
        out.push_str(&row.bytes.to_string());
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig, HUMAN_SLOT};
    use engine_core::alloc_probe;
    use engine_input::{Buttons, FighterId, FighterInput};

    const TICKS: u32 = FEED_TICKS;

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: TICKS,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn human(tick: u32, mx: i8, mz: i8, buttons: Buttons) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx,
            mz,
            facing: 0,
            buttons,
        }
    }

    /// Close-range assault: attacks every 5 ticks so the session lands real
    /// cues (hits, blocks, knockdowns) instead of idling.
    fn assault(tick: u32) -> FighterInput {
        let phase = (tick / 15) % 4;
        let (mx, mz) = match phase {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        let buttons = if tick.is_multiple_of(5) {
            Buttons(engine_input::fighter::ATTACK)
        } else {
            Buttons::NONE
        };
        human(tick, mx, mz, buttons)
    }

    /// Assert both orbit fighters sit inside the frustum cone and range rail.
    fn assert_both_framed(frame: &CameraFrame, tick: u32) {
        let params = DirectorParams::default();
        let fwd = (frame.look_m - frame.eye_m)
            .normalize()
            .expect("non-degenerate view");
        let cos_limit = (params.fov_y_rad * 0.5).cos();
        let max_sq = params.max_dist_m * params.max_dist_m;
        for fighter in orbit_views(tick) {
            let delta = fighter.pos_m - frame.eye_m;
            let dist_sq = delta.len_sq();
            assert!(dist_sq <= max_sq, "fighter {} in range", fighter.id);
            let cos_angle = delta.dot(fwd) / dist_sq.sqrt();
            assert!(
                cos_angle >= cos_limit,
                "fighter {} framed at tick {tick}",
                fighter.id
            );
        }
    }

    /// Drive one full window-style loop: a real 600-tick session whose every
    /// accepted outcome feeds the presentation feed once. Returns the feed
    /// plus the session checkpoints for the sim-untouched gate.
    fn run_win_loop() -> (PresentationFeed, Vec<crate::Checkpoint>) {
        let mut app = App::new(config()).unwrap();
        let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        for tick in 0..TICKS {
            let outcome = app.tick(assault(tick));
            assert!(outcome.input_accepted, "assault inputs validate");
            let frame = feed.win_frame(&outcome).expect("feed presents");
            assert_eq!(frame.tick, tick, "feed serves the outcome tick");
            if tick > 0 {
                assert_eq!(frame.lock, Some(1), "rival locked at tick {tick}");
            }
            assert_both_framed(&frame, tick);
        }
        (feed, app.checkpoints().to_vec())
    }

    #[test]
    fn win1_orbits_keep_both_framed_600_ticks() {
        let (feed, _) = run_win_loop();
        assert_eq!(feed.len(), TICKS as usize);
        assert_eq!(feed.frames().len(), TICKS as usize);
        assert_eq!(feed.trauma_log().len(), TICKS as usize);
        let first = feed.frames().first().unwrap().to_bytes();
        let last = feed.frames().last().unwrap().to_bytes();
        assert!(first != last, "camera tracked the orbits");
    }

    #[test]
    fn feed_window_owner_agrees_with_the_public_tick_count() {
        // One owner for the window length: the `usize` receipt capacity and
        // the public `u32` tick count must never drift, or the trim keeps a
        // different number of receipts than the buffers were pre-sized for.
        assert_eq!(FEED_TICKS, 600, "public window ticks");
        assert_eq!(
            FEED_WINDOW,
            usize::from(u16::try_from(FEED_TICKS).expect("window fits u16")),
            "one owner for the window"
        );
    }

    #[test]
    fn win1_trauma_replays_exactly_twice() {
        let (first, first_points) = run_win_loop();
        let (second, second_points) = run_win_loop();
        let first_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            first.frames().iter().map(CameraFrame::to_bytes).collect();
        let second_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            second.frames().iter().map(CameraFrame::to_bytes).collect();
        assert_eq!(first_bytes, second_bytes, "frame bytes replay exactly");
        assert_eq!(
            first.trauma_log(),
            second.trauma_log(),
            "trauma track replays exactly"
        );
        assert!(
            first.trauma_log().iter().any(|t| *t > 0),
            "the assault recorded trauma"
        );
        assert_eq!(first_points, second_points, "the feed never steers the sim");
    }

    #[test]
    fn win1_feed_leaves_sim_checkpoints_untouched() {
        let (_, fed_points) = run_win_loop();
        let mut bare = App::new(config()).unwrap();
        for tick in 0..TICKS {
            bare.tick(assault(tick));
        }
        assert_eq!(
            fed_points,
            bare.checkpoints(),
            "driving the feed changes no checkpoint"
        );
    }

    #[test]
    fn trauma_maps_every_cue_shape() {
        assert_eq!(
            trauma_for_cue(FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120
            }),
            TRAUMA_HIT
        );
        assert_eq!(
            trauma_for_cue(FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 10
            }),
            TRAUMA_BLOCKED
        );
        assert_eq!(
            trauma_for_cue(FightCue::Parried {
                attacker: 0,
                victim: 1
            }),
            TRAUMA_PARRIED
        );
        assert_eq!(
            trauma_for_cue(FightCue::Dodged {
                attacker: 1,
                victim: 0
            }),
            0
        );
        assert_eq!(trauma_for_cue(FightCue::StaminaOut { fighter: 0 }), 0);
        assert_eq!(
            trauma_for_cue(FightCue::Knockdown {
                attacker: 0,
                victim: 1
            }),
            TRAUMA_KNOCKDOWN
        );
        assert_eq!(trauma_for_cues(&[]), 0);
        let burst = [
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 50,
            },
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
        ];
        assert_eq!(
            trauma_for_cues(&burst),
            TRAUMA_HIT + TRAUMA_KNOCKDOWN,
            "bursts add in order"
        );
        assert_eq!(trauma_for_cues(&burst), trauma_for_cues(&burst));
    }

    #[test]
    fn feed_failures_name_the_problem() {
        let mut missing = PresentationFeed::new(7, DirectorParams::default()).unwrap();
        assert_eq!(missing.focus_id(), 7);
        assert!(missing.is_empty());
        assert_eq!(missing.last_frame(), None);
        // Unknown focus never frames.
        let views = orbit_views(0);
        assert_eq!(
            missing.observe_tick(0, &views, &[]),
            Err(CameraError::FocusMissing(7)),
            "feed focus must name a fighter"
        );
        // NaN positions fail loudly instead of hiding.
        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        let nan = [
            views[0],
            FighterView {
                id: 9,
                pos_m: Vec3::new(f32::NAN, 0.0, 0.0),
                alive: true,
            },
        ];
        assert_eq!(
            feed.observe_tick(0, &nan, &[]),
            Err(CameraError::BadPosition(9))
        );
        // Ticks run forward only.
        feed.observe_tick(5, &orbit_views(5), &[]).expect("frame");
        assert_eq!(
            feed.observe_tick(4, &orbit_views(4), &[]),
            Err(CameraError::TickWentBackwards { last: 5, next: 4 })
        );
        // Bad tuning refuses at construction and keeps no feed.
        let err = PresentationFeed::new(
            0,
            DirectorParams {
                base_dist_m: 0.0,
                ..DirectorParams::default()
            },
        )
        .err()
        .expect("bad tuning refuses");
        assert_eq!(err, CameraError::BadParam("base_dist_m"));
    }

    #[test]
    fn feed_steady_state_allocates_nothing() {
        // Calibration: the shared probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        feed.orbit_tick(0, &[]).expect("warm-up frame");
        // Warm-up plus this window holds exactly FEED_TICKS receipts, inside
        // the pre-sized buffers, so no growth allocation can fire here.
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..FEED_TICKS {
                let cues = [FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 50,
                }];
                let frame = feed.orbit_tick(tick, &cues).expect("frame");
                let bytes = frame.to_bytes();
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[0..8]);
                folded ^= u64::from_le_bytes(word);
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state feed update must not allocate"
        );
        assert!(folded != 0, "sanity: frames advanced during the window");
    }

    #[test]
    #[ignore = "timing: feed per-frame budget probe, run under --perf in release"]
    fn feed_budget_probe_under_0_05ms() {
        // Wall clock lives only in this ignored probe, never in the feed.
        let cues = [FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        }];
        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        feed.orbit_tick(0, &cues).expect("warm-up frame");
        let rounds = 4000u32;
        let start = std::time::Instant::now();
        let mut folded = 0u32;
        for tick in 1..=rounds {
            let frame = feed.orbit_tick(tick, &cues).expect("frame");
            folded ^= frame.trauma_permille;
        }
        std::hint::black_box(folded);
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        assert!(
            mean_ms < FEED_BUDGET_MS,
            "mean {mean_ms:.4} ms per feed update over budget"
        );
    }

    // GP19 staged feed: bypass parity, framing, replay, 0 allocs.
    fn run_bypass_loop(bypass: u8) -> PresentationFeed {
        let mut app = App::new(config()).unwrap();
        let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        for tick in 0..TICKS {
            let outcome = app.tick(assault(tick));
            let frame = feed
                .win_frame_with_bypass(&outcome, bypass)
                .expect("staged frame");
            assert_eq!(frame.tick, tick);
            if bypass & BYPASS_LOCK != 0 {
                assert_eq!(frame.lock, None, "bypassed lock reports none");
            } else if tick > 0 {
                assert_eq!(frame.lock, Some(1), "rival locked at tick {tick}");
            }
            if bypass & BYPASS_SHAKE != 0 {
                assert_eq!(frame.shake_m, Vec3::ZERO, "bypassed shake is zero");
            }
            assert_both_framed(&frame, tick);
        }
        feed
    }

    #[test]
    fn gp19_no_bypass_matches_raw_bytes() {
        let mut app = App::new(config()).unwrap();
        let mut raw = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        let mut staged = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        for tick in 0..TICKS {
            let outcome = app.tick(assault(tick));
            let first = raw.win_frame(&outcome).expect("raw frame");
            let second = staged
                .win_frame_with_bypass(&outcome, 0)
                .expect("staged frame");
            assert_eq!(first, second, "no-bypass must equal raw at tick {tick}");
        }
        assert_eq!(raw.trauma_log(), staged.trauma_log());
    }

    #[test]
    fn gp19_bypass_still_frames_600_ticks() {
        for bypass in [BYPASS_LOCK, BYPASS_SHAKE, BYPASS_LOCK | BYPASS_SHAKE] {
            let feed = run_bypass_loop(bypass);
            assert_eq!(feed.len(), TICKS as usize, "bypass {bypass:#x} lost ticks");
        }
    }

    #[test]
    fn gp19_staged_trauma_replays_exactly_twice() {
        let first = run_bypass_loop(0);
        let second = run_bypass_loop(0);
        let first_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            first.frames().iter().map(CameraFrame::to_bytes).collect();
        let second_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            second.frames().iter().map(CameraFrame::to_bytes).collect();
        assert_eq!(first_bytes, second_bytes, "staged bytes replay exactly");
        assert_eq!(first.trauma_log(), second.trauma_log());
    }

    #[test]
    fn gp19_staged_feed_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        feed.orbit_tick_with_bypass(0, &[], BYPASS_LOCK)
            .expect("warm-up frame");
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..FEED_TICKS {
                let cues = [FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 50,
                }];
                let frame = feed
                    .orbit_tick_with_bypass(tick, &cues, BYPASS_SHAKE)
                    .expect("frame");
                let bytes = frame.to_bytes();
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[0..8]);
                folded ^= u64::from_le_bytes(word);
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state staged update must not allocate"
        );
        assert!(folded != 0, "sanity: frames advanced during the window");
    }

    // GP20 smoothed feed: bypass identity, rate-independent sim, no loss.
    fn run_smoothed_loop(stride: u32, gain: u32) -> (SmoothedFeed, Vec<crate::Checkpoint>) {
        let mut app = App::new(config()).unwrap();
        let mut feed =
            SmoothedFeed::new(HUMAN_SLOT, DirectorParams::default(), gain).expect("smoothed feed");
        assert_eq!(feed.gain_permille(), gain);
        assert!(feed.is_empty());
        for tick in 0..TICKS {
            let outcome = app.tick(assault(tick));
            // Presentation sync stride stands in for 60/30/120 Hz: the sim
            // tick always runs, only the smoothing fold is strided.
            if tick.is_multiple_of(stride) {
                let (frame, smoothed) = feed.win_frame(&outcome).expect("smoothed frame");
                assert_eq!(frame.tick, tick);
                let intent = LookIntent::from_frame(&frame);
                assert_eq!(
                    DampedSync::bypass(&intent),
                    intent,
                    "intent bypass ≡ raw at tick {tick}"
                );
                assert!(smoothed.eye_m.is_finite() && smoothed.look_m.is_finite());
            }
        }
        let points = app.checkpoints().to_vec();
        (feed, points)
    }

    #[test]
    fn gp20_rates_share_sim_bytes_x2() {
        let (_, points_60) = run_smoothed_loop(1, 250);
        let (_, points_30) = run_smoothed_loop(2, 250);
        let (_, points_120) = run_smoothed_loop(1, 1000);
        assert_eq!(points_60, points_30, "30 Hz must share 60 Hz sim bytes");
        assert_eq!(points_60, points_120, "gain must not touch sim bytes");
        let (_, again_60) = run_smoothed_loop(1, 250);
        assert_eq!(points_60, again_60, "sim bytes replay exactly");
        // Bare sim without any feed agrees: presentation never steers.
        let mut bare = App::new(config()).unwrap();
        for tick in 0..TICKS {
            bare.tick(assault(tick));
        }
        assert_eq!(points_60, bare.checkpoints(), "smoothing never steers sim");
    }

    #[test]
    fn gp20_no_framed_tick_loss_vs_gp3() {
        // Raw intent frames both fighters every tick, exactly like the director.
        let (feed, _) = run_smoothed_loop(1, 1000);
        assert_eq!(feed.len(), TICKS as usize);
        assert_eq!(feed.smoothed().len(), TICKS as usize);
        assert_eq!(feed.frames().len(), TICKS as usize);
        assert_eq!(feed.trauma_log().len(), TICKS as usize);
        for (index, frame) in feed.frames().iter().enumerate() {
            assert_both_framed(frame, index as u32);
        }
        // Snapping gain tracks the intent exactly.
        for (frame, smoothed) in feed.frames().iter().zip(feed.smoothed()) {
            let intent = LookIntent::from_frame(frame);
            assert_eq!(*smoothed, intent, "snap gain must track intent");
        }
        // Damped gain still converges: mean lag under one meter on the orbit.
        let (damped, _) = run_smoothed_loop(1, 250);
        let mut sum = 0.0f64;
        for (frame, smoothed) in damped.frames().iter().zip(damped.smoothed()) {
            sum += f64::from((smoothed.eye_m - frame.eye_m).len());
        }
        let mean = sum / f64::from(TICKS);
        assert!(mean < 1.0, "damped lag {mean} m over budget");
        assert_eq!(
            SmoothedFeed::new(0, DirectorParams::default(), 0).err(),
            Some(CameraError::BadParam("damping_gain")),
            "bad gain refuses"
        );
    }

    #[test]
    fn gp20_smoothed_feed_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut feed = SmoothedFeed::new(0, DirectorParams::default(), 250).unwrap();
        feed.orbit_tick(0, &[]).expect("warm-up");
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 1..FEED_TICKS {
                let cues = [FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 50,
                }];
                let (frame, smoothed) = feed.orbit_tick(tick, &cues).expect("frame");
                let bytes = frame.to_bytes();
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[0..8]);
                folded ^= u64::from_le_bytes(word) ^ (smoothed.eye_m.x.to_bits() as u64);
            }
            folded
        });
        assert_eq!(
            steady.calls, 0,
            "steady-state smoothed update must not allocate"
        );
        assert!(folded != 0, "sanity: frames advanced during the window");
    }

    #[test]
    fn gp20_smoothed_log_stays_in_the_feed_window() {
        let mut feed = SmoothedFeed::new(0, DirectorParams::default(), 250).unwrap();
        for tick in 0..FEED_TICKS + 5 {
            feed.orbit_tick(tick, &[]).expect("frame");
        }
        assert_eq!(feed.len(), FEED_TICKS as usize, "frames window");
        assert_eq!(
            feed.smoothed().len(),
            feed.frames().len(),
            "smoothed parallels the frames window (one intent per windowed tick)"
        );
    }

    #[test]
    fn gp20_smoothed_log_keeps_no_alloc_past_the_window() {
        let mut feed = SmoothedFeed::new(0, DirectorParams::default(), 250).unwrap();
        for tick in 0..FEED_TICKS {
            feed.orbit_tick(tick, &[]).expect("warm-up");
        }
        let ((), steady) = alloc_probe::measure(|| {
            for tick in FEED_TICKS..2 * FEED_TICKS {
                feed.orbit_tick(tick, &[]).expect("frame");
            }
        });
        assert_eq!(
            steady.calls, 0,
            "a windowed smoothed log must not allocate past FEED_TICKS"
        );
    }

    // Trauma curve at the feed boundary: numbers, restore, sim bytes.
    fn trauma_script_feed(hits_per_tick: u32, ticks: u32) -> PresentationFeed {
        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        };
        for tick in 0..ticks {
            let cues: Vec<FightCue> = (0..hits_per_tick).map(|_| hit).collect();
            feed.orbit_tick(tick, &cues).expect("frame");
        }
        feed
    }

    #[test]
    fn gp21_jab_quarter_super_with_numbers() {
        // Jab script: one light hit every 4 ticks; super script: a knockdown
        // burst every tick (saturates trauma). Same ticks, same views.
        let mut jab = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        let mut big = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        let knock = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        };
        for tick in 0..TICKS {
            let jab_cues: Vec<FightCue> = if tick.is_multiple_of(4) {
                vec![hit]
            } else {
                vec![]
            };
            jab.orbit_tick(tick, &jab_cues).expect("jab frame");
            big.orbit_tick(tick, &[knock, knock]).expect("super frame");
        }
        let jab_report = shake_report(jab.frames());
        let big_report = shake_report(big.frames());
        // Playtester artifact: small-vs-big separation numbers (deterministic;
        // visible under `-- --nocapture`). Quoted in the packet report.
        eprintln!(
            "GP21 shake numbers: jab peak {:.4} m mean {:.4} m trauma {} | super peak {:.4} m mean {:.4} m trauma {}",
            jab_report.max_displacement_m,
            jab_report.mean_displacement_m,
            jab_report.max_trauma_permille,
            big_report.max_displacement_m,
            big_report.mean_displacement_m,
            big_report.max_trauma_permille
        );
        assert_eq!(jab_report.ticks, TICKS);
        assert_eq!(big_report.ticks, TICKS);
        assert!(jab_report.max_displacement_m > 0.0, "jab must shake");
        assert!(big_report.max_displacement_m > 0.0, "super must shake");
        assert!(
            jab_report.max_displacement_m <= 0.25 * big_report.max_displacement_m,
            "jab peak {} vs super peak {}",
            jab_report.max_displacement_m,
            big_report.max_displacement_m
        );
        assert!(
            big_report.max_trauma_permille >= jab_report.max_trauma_permille,
            "super must bank at least as much trauma"
        );
        // The burst banks near-saturated trauma every tick (steady state
        // 940 after the 60-per-tick decay); the pure curve saturates at 1.
        assert!(big_report.max_trauma_permille >= 900, "super must saturate");
        assert!(
            big_report.peak_gain > 0.8,
            "super gain near saturation, {}",
            big_report.peak_gain
        );
        assert!(
            (trauma_gain(1000) - 1.0).abs() < 1e-6,
            "saturated gain is 1"
        );
        // Empty slices report zeros instead of NaN.
        assert_eq!(
            shake_report(&[]),
            ShakeReport {
                ticks: 0,
                max_displacement_m: 0.0,
                mean_displacement_m: 0.0,
                max_trauma_permille: 0,
                peak_gain: 0.0,
            }
        );
        // Playtester artifact numbers (also quoted in the packet report).
        assert!(
            jab_report.mean_displacement_m <= big_report.mean_displacement_m,
            "jab mean {} vs super mean {}",
            jab_report.mean_displacement_m,
            big_report.mean_displacement_m
        );
    }

    #[test]
    fn gp21_restore_holds_every_tick_at_feed() {
        let loud = trauma_script_feed(3, TICKS);
        let quiet = trauma_script_feed(0, TICKS);
        assert_eq!(loud.len(), TICKS as usize);
        for (index, (noisy, calm)) in loud.frames().iter().zip(quiet.frames()).enumerate() {
            let tick = index as u32;
            assert_eq!(noisy.lock, calm.lock, "shake steered lock at tick {tick}");
            assert_eq!(noisy.look_m, calm.look_m, "shake moved look at tick {tick}");
            let restored = unshaken_eye(noisy);
            let calm_eye = unshaken_eye(calm);
            assert!(
                (restored - calm_eye).len() <= 1e-5,
                "restore drift at tick {tick}"
            );
            assert_both_framed(noisy, tick);
        }
    }

    #[test]
    fn gp21_sim_bytes_replay_x2() {
        let first = trauma_script_feed(2, TICKS);
        let second = trauma_script_feed(2, TICKS);
        let first_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            first.frames().iter().map(CameraFrame::to_bytes).collect();
        let second_bytes: Vec<[u8; super::super::camera::FRAME_BYTES]> =
            second.frames().iter().map(CameraFrame::to_bytes).collect();
        assert_eq!(first_bytes, second_bytes, "shake bytes replay exactly");
        assert_eq!(first.trauma_log(), second.trauma_log());
    }

    #[test]
    fn gp21_feed_stage_matches_rig_and_separates_jab_super() {
        // Squared curve, not linear: 500 permille gains 0.25, never 0.5.
        assert_eq!(feed_trauma_gain(0), 0.0);
        assert!(
            (feed_trauma_gain(500) - 0.25).abs() < 1e-6,
            "curve must square"
        );
        assert!(
            (feed_trauma_gain(1000) - 1.0).abs() < 1e-6,
            "saturated gain is 1"
        );
        assert_eq!(feed_trauma_gain(2000), 1.0, "over-range clamps at full");
        // Jab far under super through the feed stage alone.
        let jab = feed_trauma_gain(TRAUMA_HIT);
        let sup = feed_trauma_gain(1000);
        assert!(jab <= 0.25 * sup, "jab gain {jab} vs super gain {sup}");
        // Bit-exact parity with the staged rig helpers.
        assert_eq!(feed_trauma_gain(180), trauma_gain(180));
        assert_eq!(feed_trauma_gain(1000), trauma_gain(1000));
        let loud = trauma_script_feed(3, 8);
        for frame in loud.frames() {
            assert_eq!(
                feed_shake_displacement_m(frame),
                shake_displacement_m(frame)
            );
            assert_eq!(feed_restored_eye(frame), unshaken_eye(frame));
            assert!(feed_restored_eye(frame).is_finite(), "restore stays finite");
        }
        // Zero trauma displaces nothing and restores the eye itself.
        let calm = trauma_script_feed(0, 8);
        for frame in calm.frames() {
            assert_eq!(feed_shake_displacement_m(frame), 0.0);
            assert_eq!(feed_restored_eye(frame), frame.eye_m);
        }
    }

    // Staged-rig wiring: the window feed drives the staged rig over the
    // frozen director, so raw feed frames stay byte-identical to the
    // director frames tick for tick.
    #[test]
    fn gp24_rig_matches_director_bytes_600_ticks() {
        use super::super::camera::Director;
        let mut bare = Director::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
        for tick in 0..TICKS {
            let views = orbit_views(tick);
            let first = bare.update(&views, tick).expect("director frame");
            let second = feed.observe_tick(tick, &views, &[]).expect("feed frame");
            assert_eq!(
                first, second,
                "rig must serve director bytes at tick {tick}"
            );
            assert_eq!(first.to_bytes(), second.to_bytes());
        }
        assert_eq!(feed.len(), TICKS as usize);
        assert_eq!(feed.trauma_log().len(), TICKS as usize);
    }

    // Framing proof through the window feed: the scripted 600-tick
    // 2-fighter orbit keeps both fighters framed every tick (the oracle
    // reads zero unframed with zero edge arrows), and the same cue script
    // replays frame bytes plus trauma exactly twice.
    #[test]
    fn gp24_win1_orbit_framed_every_tick_with_replay() {
        fn run() -> (Vec<[u8; super::super::camera::FRAME_BYTES]>, Vec<u32>) {
            let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).unwrap();
            let hit = FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 50,
            };
            let mut bytes = Vec::with_capacity(TICKS as usize);
            for tick in 0..TICKS {
                let cues = if tick.is_multiple_of(4) {
                    vec![hit]
                } else {
                    vec![]
                };
                let frame = feed.orbit_tick(tick, &cues).expect("orbit frame");
                let views = orbit_views(tick);
                let missing = feed.unframed_count(&frame, &views).expect("oracle runs");
                assert_eq!(missing, 0, "both framed at tick {tick}");
                let mut arrows = [OffScreenArrow {
                    id: 0,
                    dir_x: 0.0,
                    dir_y: 0.0,
                }; 2];
                let drawn = feed
                    .edge_arrows(&frame, &views, &mut arrows)
                    .expect("arrows run");
                assert_eq!(drawn, 0, "no edge arrows at tick {tick}");
                bytes.push(frame.to_bytes());
            }
            (bytes, feed.trauma_log().to_vec())
        }
        let (first_bytes, first_trauma) = run();
        let (second_bytes, second_trauma) = run();
        assert_eq!(first_bytes, second_bytes, "orbit bytes replay exactly");
        assert_eq!(first_trauma, second_trauma, "trauma replays exactly");
        assert!(
            first_trauma.iter().any(|t| *t > 0),
            "the script recorded trauma"
        );
    }

    // Edge fixtures: the t60 edge-clip tick and the t120
    // opposite-corner split stay framed through the window feed.
    #[test]
    fn gp24_r3_edge_and_corner_ticks_stay_framed() {
        for tick in [60u32, 120u32] {
            let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
            let views = orbit_views(tick);
            let frame = feed.observe_tick(tick, &views, &[]).expect("fixture frame");
            let missing = feed.unframed_count(&frame, &views).expect("oracle runs");
            assert_eq!(missing, 0, "edge fixture framed at tick {tick}");
            let mut arrows = [OffScreenArrow {
                id: 0,
                dir_x: 0.0,
                dir_y: 0.0,
            }; 2];
            assert_eq!(
                feed.edge_arrows(&frame, &views, &mut arrows)
                    .expect("arrows run"),
                0,
                "no edge arrows at tick {tick}"
            );
        }
    }

    #[test]
    fn gp24_framing_oracles_name_the_problem() {
        let mut feed = PresentationFeed::new(0, DirectorParams::default()).unwrap();
        let views = orbit_views(7);
        let frame = feed.observe_tick(7, &views, &[]).expect("frame");
        // A short arrow buffer fails before any framing work.
        let mut one = [OffScreenArrow {
            id: 0,
            dir_x: 0.0,
            dir_y: 0.0,
        }; 1];
        assert_eq!(
            feed.edge_arrows(&frame, &views, &mut one),
            Err(CameraError::BadParam("arrows"))
        );
        // A NaN position fails naming its fighter.
        let nan = [
            views[0],
            FighterView {
                id: 9,
                pos_m: Vec3::new(f32::NAN, 0.0, 0.0),
                alive: true,
            },
        ];
        assert_eq!(
            feed.unframed_count(&frame, &nan),
            Err(CameraError::BadPosition(9))
        );
    }
}

// EV20 wired capture: behavior plus failure tests over the single game
// path into `capture_with` receipts, plus the 15-frame gold-set writer.
#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::HUMAN_SLOT;
    use engine_audio::FightCue;

    const SET_W: u32 = 320;
    const SET_H: u32 = 200;
    const SET_SEED: u64 = 2040;

    /// Presented frame at `tick`: a fresh feed runs the scripted orbit to
    /// the tick with a hit every 4th tick (`false` idles for the calm leg).
    fn frame_for(tick: u32, loud: bool) -> CameraFrame {
        let mut feed =
            PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).expect("feed builds");
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        };
        let mut frame = feed.orbit_tick(0, &[]).expect("tick 0 presents");
        for step in 1..=tick {
            let cues = if loud && step.is_multiple_of(4) {
                vec![hit]
            } else {
                vec![]
            };
            frame = feed.orbit_tick(step, &cues).expect("orbit presents");
        }
        assert_eq!(frame.tick, tick);
        frame
    }

    /// Read `(width, height)` out of PNG bytes, refusing non-PNGs loudly.
    fn png_dims(png: &[u8]) -> (u32, u32) {
        assert!(png.len() > 33, "receipt too short for IHDR");
        assert_eq!(&png[0..8], &GAME_PNG_MAGIC, "receipt carries PNG magic");
        assert_eq!(&png[12..16], b"IHDR", "IHDR leads");
        (
            u32::from_be_bytes(png[16..20].try_into().expect("ihdr width")),
            u32::from_be_bytes(png[20..24].try_into().expect("ihdr height")),
        )
    }

    #[test]
    fn ev20_capture_replays_byte_identical_twice() {
        let frame = frame_for(100, true);
        let digest = hash_bytes(&frame.to_bytes());
        let first = game_capture_png(&frame, 100, digest, SET_SEED, SET_W, SET_H).expect("capture");
        let second = game_capture_png(&frame, 100, digest, SET_SEED, SET_W, SET_H).expect("replay");
        assert_eq!(first, second, "same inputs give the same bytes twice");
        assert_eq!(png_dims(&first), (SET_W, SET_H));
    }

    #[test]
    fn ev20_capture_varies_with_tick_trauma_and_digest() {
        // Frozen captures must fail the showcase gate: consecutive ticks,
        // trauma changes and digest changes all repaint the pixels.
        let loud = frame_for(100, true);
        let digest = hash_bytes(&loud.to_bytes());
        let base = game_capture_png(&loud, 100, digest, SET_SEED, SET_W, SET_H).expect("base");
        let next_frame = frame_for(101, true);
        let next_digest = hash_bytes(&next_frame.to_bytes());
        let next =
            game_capture_png(&next_frame, 101, next_digest, SET_SEED, SET_W, SET_H).expect("next");
        assert_ne!(base, next, "consecutive ticks render different bytes");
        let calm = frame_for(100, false);
        let calm_png = game_capture_png(&calm, 100, digest, SET_SEED, SET_W, SET_H).expect("calm");
        assert_ne!(base, calm_png, "trauma change repaints the pixels");
        let skewed =
            game_capture_png(&loud, 100, digest ^ 1, SET_SEED, SET_W, SET_H).expect("skewed");
        assert_ne!(base, skewed, "digest change repaints the pixels");
    }

    #[test]
    fn ev20_capture_pixels_carry_spread_not_black() {
        // Pixel oracle, not a restated constant: loud frames must span a
        // wide luma range with a mid mean, so no all-black receipt passes.
        let frame = frame_for(120, true);
        let mut rgb =
            vec![0u8; usize::try_from(SET_W).unwrap() * usize::try_from(SET_H).unwrap() * 3];
        render_capture_rgb(&frame, 120, 0xABCD, SET_SEED, SET_W, SET_H, &mut rgb).expect("raster");
        let mut min = 255u32;
        let mut max = 0u32;
        let mut sum = 0u64;
        for byte in &rgb {
            let value = u32::from(*byte);
            min = min.min(value);
            max = max.max(value);
            sum += u64::from(value);
        }
        let mean = sum / u64::try_from(rgb.len()).unwrap();
        assert!(max - min >= 200, "luma spread {min}..{max} too flat");
        assert!(
            (8..=240).contains(&mean),
            "mean {mean} reads black or blown"
        );
    }

    #[test]
    fn ev20_capture_failures_name_the_row() {
        let frame = frame_for(10, true);
        let digest = hash_bytes(&frame.to_bytes());
        let capture = |w, h| game_capture_png(&frame, 10, digest, SET_SEED, w, h);
        assert_eq!(
            capture(0, SET_H),
            Err(CameraError::BadParam("capture_width"))
        );
        assert_eq!(
            capture(SET_W, 0),
            Err(CameraError::BadParam("capture_height"))
        );
        assert_eq!(
            capture(MAX_FRAME_DIM + 1, SET_H),
            Err(CameraError::BadParam("capture_size"))
        );
        assert_eq!(
            capture(SET_W, MAX_FRAME_DIM + 1),
            Err(CameraError::BadParam("capture_size"))
        );
        // Short pixel buffers fail before any paint work.
        let mut short = vec![0u8; 7];
        assert_eq!(
            render_capture_rgb(&frame, 10, digest, SET_SEED, SET_W, SET_H, &mut short),
            Err(CameraError::BadParam("capture_pixels"))
        );
        assert_eq!(
            encode_capture_png(SET_W, SET_H, &short),
            Err(CameraError::BadParam("capture_pixels"))
        );
        // Stored deflate cannot fit 2048x1024 RGB under the 4 MB receipt
        // budget, so the byte bound fires loudly instead of shipping it.
        assert_eq!(
            capture(2048, 1024),
            Err(CameraError::BadParam("capture_bytes"))
        );
    }

    #[test]
    fn ev20_showcase_manifest_binds_15_frames_and_refuses_gaps() {
        let rows = ["EV20"];
        let mut entries = Vec::with_capacity(usize::try_from(GAME_SHOWCASE_FRAMES).unwrap());
        for index in 0..GAME_SHOWCASE_FRAMES {
            let tick = index * 40;
            let frame = frame_for(tick, true);
            let digest = hash_bytes(&frame.to_bytes());
            let png =
                game_capture_png(&frame, tick, digest, SET_SEED, SET_W, SET_H).expect("set frame");
            entries.push(ShowcaseEntry {
                tick,
                sha256: showcase_sha256(&png),
                bytes: u64::try_from(png.len()).unwrap(),
            });
        }
        let set = ShowcaseSet {
            seed: SET_SEED,
            tape: "ev20-orbit.tape",
            rows: &rows,
            sha: "deadbeef2040",
            fps: 30,
            width: SET_W,
            height: SET_H,
            frames: &entries,
        };
        let manifest = showcase_manifest(&set).expect("manifest builds");
        assert!(manifest.contains("\"seed\":2040"), "seed row");
        assert!(
            manifest.contains("\"tape\":\"ev20-orbit.tape\""),
            "tape row"
        );
        assert!(manifest.contains("\"rows\":[\"EV20\"]"), "board rows");
        assert!(manifest.contains("\"sha\":\"deadbeef2040\""), "build sha");
        assert!(manifest.contains("\"fps\":30"), "fps row");
        assert_eq!(
            manifest.matches("\"sha256\":\"").count(),
            15,
            "15 hashed rows"
        );
        // Tamper binding: every row's hex must be the file's own digest.
        // A recomputed digest that misses its row fails this test loudly.
        for (index, entry) in entries.iter().enumerate() {
            let mut hex = String::new();
            hex_into(&mut hex, &entry.sha256);
            assert_eq!(hex.len(), 64, "sha256 hex at row {index}");
            assert!(manifest.contains(&hex), "row {index} binds its bytes");
            assert!(
                manifest.contains(&format!("\"bytes\":{}", entry.bytes)),
                "row {index} binds its length"
            );
        }
        // Fail-closed gaps: empty rows, bad IDs, bad SHAs, bad fps, no tape.
        let empty = ShowcaseSet { frames: &[], ..set };
        assert_eq!(
            showcase_manifest(&empty),
            Err(CameraError::BadParam("frames"))
        );
        let empty_rows: &[&str] = &[];
        for bad_rows in [
            empty_rows,
            &["win1"][..],
            &["TOOLONG123"][..],
            &["EV20", "nope"][..],
        ] {
            let bad = ShowcaseSet {
                rows: bad_rows,
                ..set
            };
            assert_eq!(showcase_manifest(&bad), Err(CameraError::BadParam("rows")));
        }
        for bad_sha in ["xyz", "123456", "gggggggg", ""] {
            let bad = ShowcaseSet {
                sha: bad_sha,
                ..set
            };
            assert_eq!(showcase_manifest(&bad), Err(CameraError::BadParam("sha")));
        }
        for bad_fps in [0, 121] {
            let bad = ShowcaseSet {
                fps: bad_fps,
                ..set
            };
            assert_eq!(showcase_manifest(&bad), Err(CameraError::BadParam("fps")));
        }
        let no_tape = ShowcaseSet { tape: "", ..set };
        assert_eq!(
            showcase_manifest(&no_tape),
            Err(CameraError::BadParam("tape"))
        );
    }

    /// Gold task: materialize the 15-frame sha-pinned set. The directory
    /// comes from `E2040_EV20_SET` (the judge's set path) or the temp dir
    /// by default, so plain `cargo test` stays hermetic. Every 40th orbit
    /// tick of the 600-tick gate lands one frame; the manifest binds each
    /// file's sha256 plus byte length, and the on-disk re-check below
    /// refuses magic drift, size drift or hash drift loudly.
    #[test]
    fn ev20_gold_set_materializes_15_sha_pinned_frames() {
        let dir = std::env::var("E2040_EV20_SET")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("e2040-ev20-set"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("gold set dir builds");
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        };
        let mut feed =
            PresentationFeed::new(HUMAN_SLOT, DirectorParams::default()).expect("feed builds");
        let mut entries = Vec::with_capacity(usize::try_from(GAME_SHOWCASE_FRAMES).unwrap());
        for index in 0..GAME_SHOWCASE_FRAMES {
            let tick = index * 40;
            let cues = if tick.is_multiple_of(12) {
                vec![hit]
            } else {
                vec![]
            };
            // Ticks ascend, so one feed serves the whole set in order.
            let frame = feed.orbit_tick(tick, &cues).expect("orbit presents");
            assert_eq!(frame.tick, tick, "set covers every 40th orbit tick");
            let digest = hash_bytes(&frame.to_bytes());
            let png =
                game_capture_png(&frame, tick, digest, SET_SEED, SET_W, SET_H).expect("set frame");
            std::fs::write(dir.join(format!("frame_{index:06}.png")), &png).expect("frame lands");
            entries.push(ShowcaseEntry {
                tick,
                sha256: showcase_sha256(&png),
                bytes: u64::try_from(png.len()).unwrap(),
            });
        }
        assert_eq!(
            entries.len(),
            usize::try_from(GAME_SHOWCASE_FRAMES).unwrap(),
            "15 frames, never 0"
        );
        let rows = ["EV20"];
        let manifest = showcase_manifest(&ShowcaseSet {
            seed: SET_SEED,
            tape: "ev20-orbit.tape",
            rows: &rows,
            sha: "deadbeef2040",
            fps: 30,
            width: SET_W,
            height: SET_H,
            frames: &entries,
        })
        .expect("gold manifest builds");
        std::fs::write(dir.join("manifest.json"), manifest.as_bytes()).expect("manifest lands");
        // On-disk re-check: magic, uniform size, per-file hash and length.
        let manifest_disk =
            std::fs::read_to_string(dir.join("manifest.json")).expect("manifest reads back");
        for (index, entry) in entries.iter().enumerate() {
            let bytes =
                std::fs::read(dir.join(format!("frame_{index:06}.png"))).expect("frame reads back");
            assert_eq!(
                png_dims(&bytes),
                (SET_W, SET_H),
                "uniform size at row {index}"
            );
            assert_eq!(
                u64::try_from(bytes.len()).unwrap(),
                entry.bytes,
                "length binds at row {index}"
            );
            let mut hex = String::new();
            hex_into(&mut hex, &showcase_sha256(&bytes));
            assert!(manifest_disk.contains(&hex), "hash binds at row {index}");
        }
        eprintln!(
            "EV20 gold set: {} frames in {}",
            entries.len(),
            dir.display()
        );
    }
}
