//! Play-in-editor: the game-window-equivalent fixed-tick loop inside the viewport.
//!
//! The consumer is the editor session (`app.rs` owns a session next and
//! drives `frame` per redraw). The owner is [`PlayInSession`]: one
//! [`App`](engine_app::App) pacer plus one [`InstantPlay`] round. Units
//! are fixed ticks and presentation frames. Determinism: the same input tape
//! in any frame chunking gives the same snapshots (tested three ways).
//! Errors are typed [`PlayInError`]; a refused restart leaves the live round.
//!
//! Per-frame budget: at most 5 fixed ticks of integer-only round math per
//! `frame` call (the anti-spiral cap shared with the game window); the live texture
//! reuses [`ViewportImage`] and re-rasterizes only on revision/size change
//! (the editor app's rule, applied by the host); tunes apply between frames only, never
//! inside `frame`.

use std::fmt;

use engine_app::{App, AppBuilder, AppConfig, AppError};
use engine_arena::spine_step::SpineMatch;
use engine_arena::world::MatchWorld;
use engine_arena::{ArenaConfig, BodyTape};
use engine_ecs::spine::SpineSnapshot;

use crate::commands::Document;
use crate::instant::{InstantPlay, Restored, RoundInput, RoundSnapshot, SnapshotError};
use crate::play::PlayEntrypoint;
use crate::shell::Shell;
use crate::viewport_image::{ViewportImage, ViewportImageError};

/// Ticks one inspector spine match runs before it restarts (one minute at 60 Hz).
const SPINE_MATCH_TICKS: u32 = 3_600;
/// Bodies the inspector's spine match fields (the arena's default duel).
const SPINE_BODIES: u8 = 2;
/// Arena half-extent for the inspector's spine match, in meters.
const SPINE_HALF_M: f32 = 20.0;

/// What rejected a play-in-editor action. Refusals move no round state: a
/// bad frame dt steps nothing, a bad snapshot restores nothing, and a
/// stopped session reports [`Closed`](Self::Closed) without touching the app.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayInError {
    /// The app lifecycle refused (bad frame dt, sealed journal, ...).
    App(AppError),
    /// A play snapshot refused (bad magic, truncated, corrupt, tampered).
    Snapshot(SnapshotError),
    /// The inspector's spine match refused to start (arena config or spawn).
    Spine,
    /// Frame, step, snapshot or restart on a stopped session.
    Closed {
        /// The refused operation.
        what: &'static str,
    },
}

impl fmt::Display for PlayInError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::App(error) => write!(f, "play-in-editor app refused: {error}"),
            Self::Snapshot(error) => write!(f, "play-in-editor snapshot refused: {error}"),
            Self::Spine => write!(f, "play-in-editor spine match refused to start"),
            Self::Closed { what } => write!(f, "play-in-editor {what} on a stopped session"),
        }
    }
}

impl std::error::Error for PlayInError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::App(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::Spine | Self::Closed { .. } => None,
        }
    }
}

impl From<AppError> for PlayInError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<SnapshotError> for PlayInError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

/// One play-in-editor checkpoint: the fixed tick plus the play digest of the
/// sealed round state there. The digest is the parity value: the same input
/// tape in-viewport and through the child-process path seals the same digests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayCheckpoint {
    /// Fixed round tick.
    pub tick: u64,
    /// Play digest of the sealed round state at `tick`.
    pub digest: [u8; 32],
}

impl From<&RoundSnapshot> for PlayCheckpoint {
    fn from(snapshot: &RoundSnapshot) -> Self {
        Self {
            tick: snapshot.tick,
            digest: snapshot.digest,
        }
    }
}

impl fmt::Display for PlayCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "t{} {:02x}{:02x}{:02x}{:02x}..",
            self.tick, self.digest[0], self.digest[1], self.digest[2], self.digest[3]
        )
    }
}

/// Receipt for one driven viewport frame: how many fixed ticks it stepped
/// and where the round stands. Zero steps while paused; at most 5 running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayFrame {
    /// Fixed round ticks stepped for this frame.
    pub steps: u32,
    /// Live round tick after the frame.
    pub tick: u64,
    /// Presentation frames driven, paused ones included.
    pub frame: u64,
}

impl fmt::Display for PlayFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "play frame {}: {} ticks -> t{}",
            self.frame, self.steps, self.tick
        )
    }
}

/// The arena match the inspector shows: one real spine stepped beside the
/// round.
///
/// The play-in round carries no spine columns (it is the editor's placeholder
/// sim), so this is the arena's own seeded duel on an empty world: brains
/// decide every slot and one round tick is one spine tick. The inspector is a
/// read-only view of these columns; nothing writes back to the match. A
/// finished match (the tick cap) or a refused step starts over at tick 0, so a
/// long play-in run never freezes the panel. Same seed gives the same match,
/// no wall clock.
pub struct LiveSpine {
    config: ArenaConfig,
    sim: SpineMatch,
}

impl LiveSpine {
    /// An arena spine at tick 0 on `seed`; `None` when the arena refuses the
    /// compile-time match (config bounds or a blocked spawn).
    pub fn start(seed: u64) -> Option<Self> {
        let config =
            ArenaConfig::try_new(seed, SPINE_MATCH_TICKS, SPINE_BODIES, SPINE_HALF_M).ok()?;
        let sim = SpineMatch::build(&config, &BodyTape(Vec::new()), MatchWorld::empty())?;
        Some(Self { config, sim })
    }

    /// Step one fixed tick; a finished or refused match starts over at tick 0.
    pub fn step(&mut self) {
        if self.sim.step().is_some() {
            return;
        }
        if let Some(sim) =
            SpineMatch::build(&self.config, &BodyTape(Vec::new()), MatchWorld::empty())
        {
            self.sim = sim;
        }
    }

    /// Ticks stepped since the match started (a restart counts from zero).
    pub fn tick(&self) -> u32 {
        self.sim.tick()
    }

    /// The spine columns after the last step, as the inspector reads them.
    pub fn snapshot(&self) -> SpineSnapshot {
        self.sim.spine().snapshot()
    }
}

/// Play-in-editor session: the game-window-equivalent loop hosted in the viewport.
///
/// Owns the [`App`](engine_app::App) pacer (fixed-tick accumulator,
/// phase order, extraction), the [`InstantPlay`] round (the sim the
/// child binary runs, driven here in-process) and the [`LiveSpine`] the
/// inspector paints. `frame` is the only tick advancer besides the single-tick
/// `step_once`; tuning batches are never touched here, so staged tuning values
/// can only land between frames at the host's `tick` cursor. The session never
/// spawns the arena child itself: child-process play stays the fallback owned
/// by the host.
pub struct PlayInSession {
    app: App,
    play: InstantPlay,
    spine: LiveSpine,
    paused: bool,
    frames: u64,
    closed: bool,
}

impl PlayInSession {
    /// Start a session on a round copy of `doc`: run the app startup
    /// sequence (registries, phase order, scene load, the verified headless
    /// frame) and seed the round from the document bytes (the instant-play rule).
    /// `seed` paces the app and seeds the inspector's spine match; round
    /// seeding stays document-derived.
    pub fn start(doc: Document, entry: PlayEntrypoint, seed: u64) -> Result<Self, PlayInError> {
        let app = AppBuilder::new()
            .with_config(AppConfig::new(seed))
            .startup()?;
        let spine = LiveSpine::start(seed).ok_or(PlayInError::Spine)?;
        let play = InstantPlay::new(doc, entry);
        Ok(Self {
            app,
            play,
            spine,
            paused: false,
            frames: 0,
            closed: false,
        })
    }

    /// True while ticks are frozen (frames still count, steps read zero).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Freeze fixed ticks. Frames keep counting; the round does not move.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Resume fixed ticks on the next frame.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// True after [`stop`](Self::stop): frame/step/snapshot/restart refuse.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Live round tick (the cursor tuning batches stage against at boundaries).
    pub fn tick(&self) -> u64 {
        self.play.tick()
    }

    /// Tick of the inspector's spine match (a match restart counts from zero).
    pub fn spine_tick(&self) -> u32 {
        self.spine.tick()
    }

    /// The inspector's spine columns after the last step.
    pub fn spine_snapshot(&self) -> SpineSnapshot {
        self.spine.snapshot()
    }

    /// Presentation frames driven, paused ones included.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Live round document revision (the status-line value).
    pub fn revision(&self) -> u64 {
        self.play.revision()
    }

    /// Inputs kept in the bounded 30 s replay window.
    pub fn tape_len(&self) -> usize {
        self.play.tape_len()
    }

    /// Tick of the oldest kept input (the replay start).
    pub fn window_start(&self) -> u64 {
        self.play.window_start()
    }

    /// Drive one viewport frame: advance the app accumulator by `frame_dt`
    /// seconds (finite, zero or more; above 0.1 s clamps to the 5-step cap),
    /// then step the round once per due tick with `inputs` in order, padding
    /// with neutral when the slice runs short. Paused frames step nothing.
    /// Refused frames (bad dt, stopped session) move no state at all.
    pub fn frame(
        &mut self,
        frame_dt: f32,
        inputs: &[RoundInput],
    ) -> Result<PlayFrame, PlayInError> {
        if self.closed {
            return Err(PlayInError::Closed { what: "frame" });
        }
        if self.paused {
            self.frames = self.frames.saturating_add(1);
            return Ok(PlayFrame {
                steps: 0,
                tick: self.play.tick(),
                frame: self.frames,
            });
        }
        let receipt = self.app.run_frame(frame_dt)?;
        for index in 0..receipt.steps {
            let input = inputs
                .get(index as usize)
                .copied()
                .unwrap_or_else(RoundInput::neutral);
            self.play.push_input(input);
        }
        for _ in 0..receipt.steps {
            self.spine.step();
        }
        self.frames = self.frames.saturating_add(1);
        Ok(PlayFrame {
            steps: receipt.steps,
            tick: self.play.tick(),
            frame: self.frames,
        })
    }

    /// Advance exactly one fixed tick with `input`, paused or running. The
    /// Step button's path: never partial, never frame-paced, always at a
    /// tick boundary (a staged tuning batch sees the moved cursor, by design).
    pub fn step_once(&mut self, input: RoundInput) -> Result<u64, PlayInError> {
        if self.closed {
            return Err(PlayInError::Closed { what: "step" });
        }
        self.play.push_input(input);
        self.spine.step();
        Ok(self.play.tick())
    }

    /// Snapshot the live round over the play snapshot API. Refuses on a stopped
    /// session; otherwise never fails.
    pub fn snapshot(&self) -> Result<RoundSnapshot, PlayInError> {
        if self.closed {
            return Err(PlayInError::Closed { what: "snapshot" });
        }
        Ok(self.play.snapshot_round())
    }

    /// The parity checkpoint: tick plus play digest of the live round.
    pub fn checkpoint(&self) -> Result<PlayCheckpoint, PlayInError> {
        self.snapshot().map(|snap| PlayCheckpoint::from(&snap))
    }

    /// Restart the round from a play snapshot: verify its digest and fields,
    /// then restore the revision and the state with the tape cleared. A
    /// refusal leaves the live round untouched. The app pacer keeps its
    /// accumulator remainder (pacing only, never round state), so the next
    /// frame steps cleanly from the restored tick.
    pub fn restart(&mut self, snapshot: &RoundSnapshot) -> Result<Restored, PlayInError> {
        if self.closed {
            return Err(PlayInError::Closed { what: "restart" });
        }
        Ok(self.play.restart(snapshot)?)
    }

    /// Render the shell through the editor's live-texture path
    /// ([`ViewportImage`], render-validated): no second raster, same bytes for
    /// the same document plus camera. A pure view: works on a stopped
    /// session, so the panel keeps showing the document after Stop.
    pub fn render_viewport(
        &self,
        shell: &Shell,
        width: u32,
        height: u32,
    ) -> Result<ViewportImage, ViewportImageError> {
        ViewportImage::render(shell, width, height)
    }

    /// Stop the session: shut the app down (inputs stop, journal seals,
    /// systems drop in reverse). Idempotent; the round document and the last
    /// snapshot are untouched.
    pub fn stop(&mut self) -> Result<(), PlayInError> {
        if self.closed {
            return Ok(());
        }
        self.app.shutdown()?;
        self.closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    use crate::instant::REPLAY_WINDOW_TICKS;
    use crate::tuning::{CombatField, LoopbackChild, TuneError, TuningPanel};

    /// Round document with one node at the origin (revision 1).
    fn fixture_doc() -> Document {
        let mut shell = Shell::new();
        let id = ObjectId::new();
        shell
            .insert_node(
                id,
                ObjectId::NIL,
                Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        shell.document().clone()
    }

    /// Entrypoint naming a program that is never launched: the session never
    /// spawns the child itself (the host keeps owning ChildPlay), so this
    /// only has to exist as a value.
    fn stub_entry() -> PlayEntrypoint {
        PlayEntrypoint::new("e2040-no-such-program-xyz", Vec::<String>::new())
    }

    /// Deterministic input stream: full lane sweep, repeating.
    fn input_tape(n: usize) -> Vec<RoundInput> {
        (0..n)
            .map(|i| {
                let lane = (i % 5) as i8 - 2;
                RoundInput::new(lane, 2 - lane)
            })
            .collect()
    }

    #[test]
    fn same_tape_same_checkpoints_chunked_vs_straight() {
        let doc = fixture_doc();
        let inputs = input_tape(90);
        // Straight: the bare instant-play sim the child binary runs.
        let mut straight = InstantPlay::new(doc.clone(), stub_entry());
        for input in &inputs {
            straight.push_input(*input);
        }
        let straight_end = straight.snapshot_round();
        // In-viewport, chunked across uneven multi-tick frames: consume what
        // the driver reports, so float rounding in the accumulator can never
        // misalign the tape.
        let mut session = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        let dt = AppConfig::new(2040).dt();
        let mut pos = 0usize;
        let mut guard = 0u32;
        while pos < inputs.len() && guard < 1000 {
            guard += 1;
            let receipt = session.frame(3.0 * dt, &inputs[pos..]).unwrap();
            assert!(
                receipt.steps <= 5,
                "RC1 anti-spiral cap holds in-viewport: {receipt}"
            );
            assert!(receipt.steps >= 1, "a 3-tick frame always advances");
            pos += receipt.steps as usize;
        }
        assert_eq!(pos, inputs.len());
        assert_eq!(session.tick(), straight.tick());
        assert_eq!(
            session.snapshot().unwrap(),
            straight_end,
            "chunked in-viewport frames seal the same checkpoints as straight play"
        );
        // Again with single-tick frames around a pause plus stepped ticks.
        let mut stepped = PlayInSession::start(doc, stub_entry(), 2040).unwrap();
        for input in inputs.iter().take(40) {
            let receipt = stepped.frame(dt, std::slice::from_ref(input)).unwrap();
            assert_eq!(receipt.steps, 1);
        }
        stepped.pause();
        assert!(stepped.is_paused());
        for input in inputs.iter().skip(40).take(5) {
            let before = stepped.tick();
            assert_eq!(stepped.step_once(*input).unwrap(), before + 1);
        }
        let frozen = stepped.frame(dt, &inputs[45..]).unwrap();
        assert_eq!(frozen.steps, 0, "paused frames freeze the round");
        assert_eq!(stepped.tick(), 45);
        assert!(!frozen.to_string().is_empty());
        stepped.resume();
        assert!(!stepped.is_paused());
        for input in inputs.iter().skip(45) {
            stepped.frame(dt, std::slice::from_ref(input)).unwrap();
        }
        assert_eq!(stepped.tick(), 90);
        assert_eq!(stepped.snapshot().unwrap(), straight_end);
        assert_eq!(
            stepped.checkpoint().unwrap(),
            PlayCheckpoint::from(&straight_end),
            "pause/step/restart framing never forks the truth"
        );
    }

    #[test]
    fn the_live_spine_steps_one_tick_per_round_tick() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 2040).unwrap();
        assert_eq!(session.spine_tick(), 0);
        let dt = AppConfig::new(2040).dt();
        let receipt = session.frame(3.0 * dt, &input_tape(3)).unwrap();
        assert_eq!(receipt.steps, 3);
        assert_eq!(session.spine_tick(), 3, "one spine tick per round tick");
        session.pause();
        session.frame(dt, &[]).unwrap();
        assert_eq!(session.spine_tick(), 3, "paused frames freeze the spine");
        session.resume();
        session.step_once(RoundInput::neutral()).unwrap();
        assert_eq!(session.spine_tick(), 4);
        // The snapshot is the spine's own bytes: it restores to itself.
        let snap = session.spine_snapshot();
        let back = engine_ecs::spine::Spine::restore(&snap.bytes).expect("spine restores");
        assert_eq!(back.snapshot(), snap);
        assert_eq!(back.live(), usize::from(SPINE_BODIES));
    }

    #[test]
    fn the_live_spine_restarts_at_its_tick_cap() {
        let mut spine = LiveSpine::start(2040).expect("spine match builds");
        for _ in 0..SPINE_MATCH_TICKS {
            spine.step();
        }
        assert_eq!(spine.tick(), SPINE_MATCH_TICKS);
        spine.step();
        assert_eq!(
            spine.tick(),
            0,
            "a finished match starts over, never freezes"
        );
    }

    #[test]
    fn the_live_spine_inspects_through_the_rc4_registry() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 9).unwrap();
        let dt = AppConfig::new(9).dt();
        for _ in 0..30 {
            session.frame(dt, &[]).unwrap();
        }
        let snapshot = session.spine_snapshot();
        let registry = crate::spine_inspect::spine_registry().expect("spine components register");
        let (view, nodes) =
            crate::spine_inspect::inspect_spine(session.spine_tick(), &snapshot, &registry)
                .expect("live spine inspects");
        assert_eq!(view.tick, 30);
        assert_eq!(view.arena_hash, Some(snapshot.hash));
        assert_eq!(nodes.len(), usize::from(SPINE_BODIES));
        for node in &nodes {
            assert_eq!(
                node.components.len(),
                crate::spine_inspect::COMPONENTS.len()
            );
            assert!(node.schema.iter().all(|step| step.is_current));
        }
    }

    #[test]
    fn pause_freezes_ticks_while_frames_keep_counting() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 2040).unwrap();
        let dt = AppConfig::new(2040).dt();
        let first = session.frame(dt, &[RoundInput::new(1, -1)]).unwrap();
        assert_eq!((first.steps, first.tick, first.frame), (1, 1, 1));
        session.pause();
        let held = session.frame(dt, &[RoundInput::new(2, 2)]).unwrap();
        assert_eq!((held.steps, held.tick, held.frame), (0, 1, 2));
        let held_again = session.frame(dt, &[]).unwrap();
        assert_eq!(
            (held_again.steps, held_again.tick, held_again.frame),
            (0, 1, 3)
        );
        session.resume();
        let moving = session.frame(dt, &[]).unwrap();
        assert_eq!((moving.steps, moving.tick, moving.frame), (1, 2, 4));
    }

    #[test]
    fn restart_rewinds_and_replays_identically_over_pl1_snapshots() {
        let doc = fixture_doc();
        let mut session = PlayInSession::start(doc, stub_entry(), 7).unwrap();
        let dt = AppConfig::new(7).dt();
        let inputs = input_tape(48);
        for input in inputs.iter().take(41) {
            session.frame(dt, std::slice::from_ref(input)).unwrap();
        }
        let saved = session.snapshot().unwrap();
        assert_eq!(saved.tick, 41);
        for input in inputs.iter().skip(41) {
            session.frame(dt, std::slice::from_ref(input)).unwrap();
        }
        let diverged = session.snapshot().unwrap();
        assert_ne!(diverged.digest, saved.digest);
        let restored = session.restart(&saved).unwrap();
        assert_eq!(restored.tick, 41);
        assert_eq!(restored.revision, saved.revision);
        assert_eq!(restored.digest, saved.digest);
        assert_eq!(session.tick(), 41);
        assert_eq!(session.snapshot().unwrap(), saved);
        for input in inputs.iter().skip(41) {
            session.frame(dt, std::slice::from_ref(input)).unwrap();
        }
        assert_eq!(
            session.snapshot().unwrap(),
            diverged,
            "same tape after restart replays bit-identically"
        );
        // A corrupt snapshot refuses and leaves the live round alone.
        let mut bad = saved.clone();
        bad.opaque[0] ^= 0xFF;
        let before = session.snapshot().unwrap();
        assert!(matches!(
            session.restart(&bad),
            Err(PlayInError::Snapshot(_))
        ));
        assert_eq!(session.snapshot().unwrap(), before);
    }

    #[test]
    fn tune_batches_apply_only_at_tick_boundaries() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 11).unwrap();
        let dt = AppConfig::new(11).dt();
        // Host contract: the tune child advances once per play tick, and
        // staged batches apply between frames, never inside `frame`.
        let mut child = LoopbackChild::new();
        for _ in 0..8 {
            session.frame(dt, &[]).unwrap();
            child.tick();
        }
        assert_eq!(session.tick(), 8);
        assert_eq!(child.cursor(), 8);
        let mut panel = TuningPanel::new();
        panel
            .set_combat(CombatField::ParryWindowTicks, 3)
            .expect("parry below startup stages");
        panel.stage(session.tick() as u32).unwrap();
        // The session runs on while the child still sits at the boundary:
        // the apply lands at tick 8, never mid-frame.
        session.frame(dt, &[]).unwrap();
        let receipt = panel.apply(&mut child).unwrap();
        assert_eq!(receipt.at_tick, 8);
        assert_eq!(session.tick(), 9, "tuning never moves the round");
        // Host catches the child up, stages at 9, then the child moves on
        // without an apply: the stale batch refuses and writes nothing.
        child.tick();
        panel.stage(session.tick() as u32).unwrap();
        child.tick();
        assert_eq!(
            panel.apply(&mut child).unwrap_err(),
            TuneError::StaleChild {
                expected: 10,
                got: 9
            }
        );
        assert_eq!(child.combat_commands(), 1);
    }

    #[test]
    fn viewport_render_reuses_the_ra1_path() {
        let mut shell = Shell::new();
        let id = ObjectId::new();
        shell
            .insert_node(
                id,
                ObjectId::NIL,
                Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let session = PlayInSession::start(shell.document().clone(), stub_entry(), 2040).unwrap();
        let live = session.render_viewport(&shell, 320, 200).unwrap();
        let direct = ViewportImage::render(&shell, 320, 200).unwrap();
        assert_eq!(live, direct, "no second raster: same bytes as ED1");
        assert_eq!(live.draws, 1);
        live.to_upload().validate().expect("live frame uploads");
        assert_eq!(
            session.render_viewport(&shell, 0, 200).unwrap_err(),
            ViewportImageError::Suspended
        );
        assert!(matches!(
            session.render_viewport(&shell, 9999, 8),
            Err(ViewportImageError::BadSize { .. })
        ));
    }

    #[test]
    fn huge_frame_dt_caps_at_five_ticks_like_win1() {
        let doc = fixture_doc();
        let mut session = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        // 10 s of wall time: the app accumulator clamps the input and the
        // step loop (the anti-spiral cap shared with the game window, cited not mirrored).
        let receipt = session.frame(10.0, &[]).unwrap();
        assert_eq!(receipt.steps, 5);
        assert_eq!(session.tick(), 5);
        // Short slices pad with neutral: same five neutral ticks straight.
        let mut straight = InstantPlay::new(doc, stub_entry());
        for _ in 0..5 {
            straight.push_input(RoundInput::neutral());
        }
        assert_eq!(session.snapshot().unwrap(), straight.snapshot_round());
    }

    #[test]
    fn input_window_stays_bounded_past_thirty_seconds() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 2040).unwrap();
        let total = REPLAY_WINDOW_TICKS + 100;
        for i in 0..total {
            session
                .step_once(RoundInput::new((i % 5) as i8 - 2, 0))
                .unwrap();
        }
        assert_eq!(session.tick(), total as u64);
        assert_eq!(session.tape_len(), REPLAY_WINDOW_TICKS);
        assert_eq!(session.window_start(), 100);
    }

    #[test]
    fn bad_input_and_stopped_session_fail_loudly() {
        let mut session = PlayInSession::start(fixture_doc(), stub_entry(), 2040).unwrap();
        assert!(matches!(
            session.frame(f32::NAN, &[]),
            Err(PlayInError::App(AppError::BadFrameDt { .. }))
        ));
        assert!(matches!(
            session.frame(-1.0, &[]),
            Err(PlayInError::App(AppError::BadFrameDt { .. }))
        ));
        assert_eq!(session.tick(), 0, "refused frames move no ticks");
        assert_eq!(session.frames(), 0, "refused frames count no frames");
        session.stop().unwrap();
        assert!(session.is_closed());
        assert!(session.stop().is_ok(), "stop stays idempotent");
        assert_eq!(
            session.frame(0.01, &[]).unwrap_err(),
            PlayInError::Closed { what: "frame" }
        );
        assert_eq!(
            session.step_once(RoundInput::neutral()).unwrap_err(),
            PlayInError::Closed { what: "step" }
        );
        assert_eq!(
            session.snapshot().unwrap_err(),
            PlayInError::Closed { what: "snapshot" }
        );
        assert_eq!(
            session.checkpoint().unwrap_err(),
            PlayInError::Closed { what: "snapshot" }
        );
        assert!(matches!(
            session.restart(&RoundSnapshot {
                tick: 0,
                revision: 0,
                opaque: Vec::new(),
                digest: [0; 32],
            }),
            Err(PlayInError::Closed { .. })
        ));
        // The viewport still renders after stop: pure view over the shell.
        assert!(session.render_viewport(&Shell::new(), 64, 48).is_ok());
        // Every variant names its cause.
        assert!(PlayInError::Closed { what: "frame" }
            .to_string()
            .contains("frame"));
        assert!(PlayCheckpoint {
            tick: 3,
            digest: [0xABu8; 32]
        }
        .to_string()
        .contains('3'));
    }

    /// ED16 short-fight tape: 90 deterministic ticks (lane sweep).
    fn short_fight_tape() -> Vec<RoundInput> {
        input_tape(90)
    }

    /// ED16 long-soak tape: past the 30 s window, still divisible by the
    /// 3-tick viewport chunk so chunked frames never pad with neutral.
    fn long_soak_tape() -> Vec<RoundInput> {
        input_tape(REPLAY_WINDOW_TICKS + 99)
    }

    /// ED16 tuned-fight tape: 120 ticks driven with a tune at tick 8.
    fn tuned_fight_tape() -> Vec<RoundInput> {
        input_tape(120)
    }

    /// Child path: the bare instant-play sim the arena child steps.
    fn child_checkpoints(doc: &Document, inputs: &[RoundInput], at: &[u64]) -> Vec<PlayCheckpoint> {
        let mut play = InstantPlay::new(doc.clone(), stub_entry());
        let mut out = Vec::new();
        for input in inputs {
            play.push_input(*input);
            if at.contains(&play.tick()) {
                out.push(PlayCheckpoint::from(&play.snapshot_round()));
            }
        }
        out
    }

    /// Viewport path: the same tape tick-by-tick through `PlayInSession`.
    fn viewport_checkpoints(
        doc: &Document,
        inputs: &[RoundInput],
        at: &[u64],
    ) -> Vec<PlayCheckpoint> {
        let mut session = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        let dt = AppConfig::new(2040).dt();
        let mut out = Vec::new();
        for input in inputs {
            let receipt = session.frame(dt, std::slice::from_ref(input)).unwrap();
            assert_eq!(receipt.steps, 1);
            if at.contains(&session.tick()) {
                out.push(session.checkpoint().unwrap());
            }
        }
        out
    }

    /// Viewport chunked across 3-tick frames: same end as straight play.
    fn viewport_chunked_end(doc: &Document, inputs: &[RoundInput]) -> RoundSnapshot {
        let mut session = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        let dt = AppConfig::new(2040).dt();
        let mut pos = 0usize;
        while pos < inputs.len() {
            let receipt = session.frame(3.0 * dt, &inputs[pos..]).unwrap();
            assert!((1..=5).contains(&receipt.steps));
            pos += receipt.steps as usize;
        }
        assert_eq!(pos, inputs.len());
        session.snapshot().unwrap()
    }

    /// Child end state for one tape.
    fn child_end(doc: &Document, inputs: &[RoundInput]) -> RoundSnapshot {
        let mut play = InstantPlay::new(doc.clone(), stub_entry());
        for input in inputs {
            play.push_input(*input);
        }
        play.snapshot_round()
    }

    /// Tuned viewport path: same 120-tick tape with a parry tune staged at
    /// tick 8 between frames; tuning never moves the round by design.
    fn tuned_viewport_checkpoints(
        doc: &Document,
        inputs: &[RoundInput],
        at: &[u64],
    ) -> Vec<PlayCheckpoint> {
        let mut session = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        let dt = AppConfig::new(2040).dt();
        let mut child = LoopbackChild::new();
        let mut panel = TuningPanel::new();
        panel
            .set_combat(CombatField::ParryWindowTicks, 3)
            .expect("parry below startup stages");
        let mut out = Vec::new();
        for (index, input) in inputs.iter().enumerate() {
            let receipt = session.frame(dt, std::slice::from_ref(input)).unwrap();
            assert_eq!(receipt.steps, 1);
            child.tick();
            if session.tick() == 8 && index == 7 {
                panel.stage(session.tick() as u32).unwrap();
                let applied = panel.apply(&mut child).unwrap();
                assert_eq!(applied.at_tick, 8);
            }
            if at.contains(&session.tick()) {
                out.push(session.checkpoint().unwrap());
            }
        }
        assert!(panel.replay_exact(), "tuned timeline replays exactly");
        out
    }

    #[test]
    fn three_tapes_child_and_viewport_seal_same_checkpoints_twice() {
        let doc = fixture_doc();
        let short = short_fight_tape();
        let long = long_soak_tape();
        let tuned = tuned_fight_tape();
        let short_at = [30, 60, 90];
        let long_at = [600, 1200, 1899];
        let tuned_at = [40, 80, 120];
        for _ in 0..2 {
            assert_eq!(
                child_checkpoints(&doc, &short, &short_at),
                viewport_checkpoints(&doc, &short, &short_at),
                "short fight parity"
            );
            assert_eq!(
                child_checkpoints(&doc, &long, &long_at),
                viewport_checkpoints(&doc, &long, &long_at),
                "long soak parity"
            );
            assert_eq!(
                child_checkpoints(&doc, &tuned, &tuned_at),
                tuned_viewport_checkpoints(&doc, &tuned, &tuned_at),
                "tuned fight parity"
            );
            assert_eq!(child_end(&doc, &short), viewport_chunked_end(&doc, &short));
            assert_eq!(child_end(&doc, &long), viewport_chunked_end(&doc, &long));
            assert_eq!(child_end(&doc, &tuned), viewport_chunked_end(&doc, &tuned));
        }
        let mut soaked = PlayInSession::start(doc.clone(), stub_entry(), 2040).unwrap();
        for input in &long {
            soaked.step_once(*input).unwrap();
        }
        assert_eq!(soaked.tape_len(), REPLAY_WINDOW_TICKS);
        assert_eq!(soaked.window_start(), 99);
        assert_ne!(
            child_end(&doc, &short).digest,
            child_end(&doc, &long).digest,
            "oracle: different tapes diverge"
        );
        let mut flipped = short.clone();
        flipped[10] = RoundInput::new(2, 2);
        assert_ne!(
            child_end(&doc, &short).digest,
            child_end(&doc, &flipped).digest,
            "oracle: one input flips the checkpoint"
        );
    }
}
