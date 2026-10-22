//! sync: GGRS-style rollback proof plus the per-system hash split.
//!
//! Consumer: netcode rollback (`run_prefix`/`resume`), playtest forks and
//! `tools/desync-diff.mjs`, which prints the first diverging system and dumps
//! its canonical bytes into a Factorio-style report dir. Data owner: this
//! module owns [`SystemHashes`] and [`SyncError`]; match state stays owned by
//! the arena sim. Units: fixed ticks at [`TICK_HZ`](super::TICK_HZ), meters,
//! permille. Determinism scope: same-machine reproducibility — same config
//! plus tape twice gives identical per-tick systems, and snapshot/restore
//! resimulates identical checkpoints. Error policy: invalid config/tape or a
//! split past the end maps to `None`; a resimulation mismatch returns
//! [`SyncError`] naming the first tick, the first diverging system and the
//! tape inputs at that tick, never a bare hash.
//!
//! The authority hash is untouched: the five subsystem hashes below are
//! diagnostic only and never feed [`Checkpoint`](super::Checkpoint). Byte
//! layout mirrors the authority sections so a diverging system points at the
//! real field family.
//!
//! Beside the system split, [`SlotHashes`] folds each fighter's own bytes
//! into one digest per slot, so a report can name the fighter and not only
//! the system. Slice layout: a slot digest covers the world, combat and ai
//! bytes of that slot; shared rows (the tick's event list, the limb pool and
//! the destruction prop) stay in the system digest, because no single slot
//! owns them.

use super::MAX_SLOTS;
use super::{canon_zero, event_bytes, ArenaConfig, Checkpoint, MatchSnapshot};
use engine_anim::motion_net::state_code;
use engine_core::Fnv64;
use engine_destruction::bonded::event_bytes as break_bytes;
use engine_input::{FighterInput, FighterTape};

/// Per-system diagnostic hashes for one post-step tick, in check order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemHashes {
    /// Kinematic authority: poses, yaw, latched wishes and buttons.
    pub world: u64,
    /// Spatial authority: destruction prop plus controller hashes (`sim`).
    pub space: u64,
    /// Fight authority: hp/stamina plus the tick's combat events.
    pub combat: u64,
    /// Deterministic driver: tick index plus the `sim` tag byte.
    pub rng: u64,
    /// Brain-driven motion: locomotion machine bytes per slot.
    pub ai: u64,
}

/// System names in [`SystemHashes`] field order.
pub const SYSTEM_NAMES: [&str; 5] = ["world", "space", "combat", "rng", "ai"];

/// First system whose hashes differ, in [`SYSTEM_NAMES`] order.
pub fn first_diverging_system(a: &SystemHashes, b: &SystemHashes) -> Option<&'static str> {
    if a.world != b.world {
        return Some("world");
    }
    if a.space != b.space {
        return Some("space");
    }
    if a.combat != b.combat {
        return Some("combat");
    }
    if a.rng != b.rng {
        return Some("rng");
    }
    if a.ai != b.ai {
        return Some("ai");
    }
    None
}

/// GGRS-style resimulation failure: the first diverging tick, system, both
/// checkpoint hashes and the tape inputs that ran at that tick.
#[derive(Clone, PartialEq, Debug)]
pub struct SyncError {
    /// Zero-based fixed tick that first diverged.
    pub tick: u32,
    /// First diverging system in [`SYSTEM_NAMES`] order.
    pub system: &'static str,
    /// First fighter slot whose slice digest differs, when the split can see
    /// one. `None` when the divergence is in a shared row (the tick's event
    /// list, the limb pool) or in the driver, where no single slot owns it.
    pub slot: Option<u8>,
    /// Checkpoint hash of the uninterrupted run at [`SyncError::tick`].
    pub expected: u64,
    /// Checkpoint hash of the snapshot/resume continuation there.
    pub actual: u64,
    /// Tape inputs with `tick == tick`, in fighter order.
    pub inputs: Vec<FighterInput>,
}

impl core::fmt::Display for SyncError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "sync diverged at tick {} (system {}",
            self.tick, self.system
        )?;
        if let Some(slot) = self.slot {
            write!(f, ", slot {slot}")?;
        }
        write!(
            f,
            "): expected {:016x}, got {:016x} ({} inputs)",
            self.expected,
            self.actual,
            self.inputs.len()
        )
    }
}

impl std::error::Error for SyncError {}

/// One digest per fighter slot over that slot's own bytes, for the report
/// that has to name the fighter. Diagnostics only: never folded into
/// [`Checkpoint`](super::Checkpoint).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotHashes {
    /// One digest per slot in fighter order; `None` past the fighter count.
    pub slots: [Option<u64>; MAX_SLOTS],
}

/// First slot whose slice digests differ. A slot present on one side and
/// absent on the other counts as a difference: the two runs did not fight the
/// same cast, which is itself the finding.
pub fn first_diverging_slot(a: &SlotHashes, b: &SlotHashes) -> Option<u8> {
    for (slot, (left, right)) in a.slots.iter().zip(b.slots.iter()).enumerate() {
        if left != right {
            return u8::try_from(slot).ok();
        }
    }
    None
}

/// One post-step tick: its checkpoint plus its system split.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TickHashes {
    checkpoint: Checkpoint,
    systems: SystemHashes,
    slots: SlotHashes,
}

/// Tagged dump (Factorio `level_with_tags` idea-only): same bytes plus tags.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaggedDump {
    pub tick: u32,
    pub system: &'static str,
    pub offset: usize,
    pub len: usize,
    pub bytes: Vec<u8>,
}

/// Tagged dump at post-`steps`. `None` on bad config/tape/system/steps.
pub fn system_tagged_bytes(
    config: &ArenaConfig,
    tape: &FighterTape,
    steps: u32,
    system: &str,
) -> Option<TaggedDump> {
    if steps > config.ticks {
        return None;
    }
    let mut sim = super::Sim::build(config, tape, false, false)?;
    while sim.tick < steps {
        sim.step(tape)?;
    }
    let mut bytes = Vec::new();
    append_system_bytes(&sim, system, &mut bytes)?;
    let name = SYSTEM_NAMES.iter().find(|name| **name == system)?;
    Some(TaggedDump {
        tick: steps,
        system: name,
        offset: 0,
        len: bytes.len(),
        bytes,
    })
}

/// Canonical bytes for one system at the post-`steps` state, rebuilt from
/// scratch (`steps` ticks stepped from tick 0). `None` on invalid
/// config/tape, unknown system, or `steps > config.ticks`.
///
/// Consumer: `tools/desync-diff.mjs` dumps these bytes for the reported
/// system. Units: the same LE/bit layouts the subsystem hashes fold.
pub fn system_canonical_bytes(
    config: &ArenaConfig,
    tape: &FighterTape,
    steps: u32,
    system: &str,
) -> Option<Vec<u8>> {
    system_tagged_bytes(config, tape, steps, system).map(|tagged| tagged.bytes)
}

/// Per-slot slice digests at the post-`steps` state, rebuilt from scratch
/// (`steps` ticks stepped from tick 0). `None` on invalid config/tape or
/// `steps > config.ticks`.
///
/// Consumer: a desync report that has to name the fighter, not only the
/// system (`tools/desync-diff.mjs` prints [`SyncError::slot`]). Units: one
/// digest per slot over that slot's world, combat and ai bytes.
pub fn slot_hashes_at(config: &ArenaConfig, tape: &FighterTape, steps: u32) -> Option<SlotHashes> {
    if steps > config.ticks {
        return None;
    }
    let mut sim = super::Sim::build(config, tape, false, false)?;
    while sim.tick < steps {
        sim.step(tape)?;
    }
    Some(slot_hashes_of(&sim))
}

/// Per-tick system hashes for a full run, post-step per tick. Empty on
/// invalid config/tape, never partial.
pub fn run_system_hashes(config: &ArenaConfig, tape: &FighterTape) -> Vec<SystemHashes> {
    drive(config, tape, config.ticks)
        .map(|rows| rows.iter().map(|row| row.systems).collect())
        .unwrap_or_default()
}

/// GGRS-style sync test: snapshot at `snapshot_at`, step to `config.ticks`,
/// restore there, re-step the same inputs, compare per-tick hashes.
/// `None` on invalid config/tape or `snapshot_at > config.ticks`; otherwise
/// the full checkpoint row, or the first divergence with its tick, system
/// and inputs. Run twice: same inputs twice give the same result.
pub fn verify_rollback(
    config: &ArenaConfig,
    tape: &FighterTape,
    snapshot_at: u32,
) -> Option<Result<Vec<Checkpoint>, SyncError>> {
    if snapshot_at > config.ticks {
        return None;
    }
    let full = drive(config, tape, config.ticks)?;
    let verdict = resume_matches(&full, config, tape, snapshot_at, config.ticks)?;
    Some(verdict.map(|()| full.iter().map(|row| row.checkpoint).collect()))
}

/// Rebuild a run to `snapshot_at`, snapshot and resume it, step the resumed
/// sim to `until` and compare every resumed tick with `full`, the
/// uninterrupted run. `until - snapshot_at` is the rollback depth. `None` on
/// invalid config/tape or a window outside `full`; otherwise the first
/// divergence with its tick, system and inputs.
fn resume_matches(
    full: &[TickHashes],
    config: &ArenaConfig,
    tape: &FighterTape,
    snapshot_at: u32,
    until: u32,
) -> Option<Result<(), SyncError>> {
    let mut sim = super::Sim::build(config, tape, false, false)?;
    while sim.tick < snapshot_at {
        sim.step(tape)?;
    }
    let mut resumed = resume_sim(take_snapshot(sim)?, config, tape)?;
    for tick in snapshot_at..until {
        resumed.step(tape)?;
        let row = TickHashes {
            checkpoint: resumed.checkpoints.last().copied()?,
            systems: hashes_of(&resumed),
            slots: slot_hashes_of(&resumed),
        };
        let want = full.get(tick as usize)?;
        if *want != row {
            return Some(Err(dual_save_report(tick, want, &row, tape)));
        }
    }
    Some(Ok(()))
}

/// Dual-save envelope (Factorio idea-only): reference plus desynced plus inputs.
/// The slot is named only for a per-slot system (`world`, `combat`, `ai`); a
/// divergence in `space` or `rng` belongs to the shared world or the driver,
/// so reporting a fighter there would point at the wrong place.
fn dual_save_report(
    tick: u32,
    reference: &TickHashes,
    desynced: &TickHashes,
    tape: &FighterTape,
) -> SyncError {
    let system = first_diverging_system(&reference.systems, &desynced.systems).unwrap_or("world");
    let slot = matches!(system, "world" | "combat" | "ai")
        .then(|| first_diverging_slot(&reference.slots, &desynced.slots))
        .flatten();
    SyncError {
        tick,
        system,
        slot,
        expected: reference.checkpoint.hash,
        actual: desynced.checkpoint.hash,
        inputs: inputs_at(tape, tick),
    }
}

/// Drive `steps` ticks from tick 0, collecting per-tick hashes.
fn drive(config: &ArenaConfig, tape: &FighterTape, steps: u32) -> Option<Vec<TickHashes>> {
    drive_with(config, tape, steps, |_| {})
}

/// [`drive`] with `after_step` called on the live sim after every step and
/// before its hashes are taken, so a test can play state a snapshot loses.
fn drive_with(
    config: &ArenaConfig,
    tape: &FighterTape,
    steps: u32,
    mut after_step: impl FnMut(&mut super::Sim),
) -> Option<Vec<TickHashes>> {
    let mut sim = super::Sim::build(config, tape, false, false)?;
    let mut rows = Vec::with_capacity(steps as usize);
    while sim.tick < steps {
        sim.step(tape)?;
        after_step(&mut sim);
        let checkpoint = sim.checkpoints.last().copied()?;
        rows.push(TickHashes {
            checkpoint,
            systems: hashes_of(&sim),
            slots: slot_hashes_of(&sim),
        });
    }
    Some(rows)
}

/// Capture the rollback snapshot the way [`run_prefix`](super::run_prefix)
/// does: live controllers move through under `sim`, so the continuation is
/// bit-identical. `None` when the snapshot cannot be built.
fn take_snapshot(sim: super::Sim) -> Option<MatchSnapshot> {
    #[cfg(feature = "sim")]
    let controller_snaps = sim.controllers.iter().map(|c| c.snapshot()).collect();
    Some(MatchSnapshot {
        config: sim.config,
        tick: sim.tick,
        states: sim.states,
        wishes: sim.wishes,
        buttons: sim.buttons,
        fight: sim.fight,
        match_state: *sim.match_track.state(),
        match_version: sim.match_track.version(),
        phased: *sim.match_track.phased(),
        brains: sim.brains,
        locos: sim.locos,
        destruction: sim.destruction,
        rise: sim.rise,
        height: sim.height,
        #[cfg(feature = "sim")]
        controller_snaps,
        #[cfg(feature = "sim")]
        controllers: sim.controllers,
    })
}

/// Rebuild the continuation the way [`resume`](super::resume) does, without
/// stepping yet. `None` on config/tape/snapshot mismatch.
fn resume_sim(
    snapshot: MatchSnapshot,
    config: &ArenaConfig,
    tape: &FighterTape,
) -> Option<super::Sim> {
    super::Sim::from_snapshot_parts(snapshot, config, tape)
}

/// Tape inputs running at `tick`, in fighter order.
fn inputs_at(tape: &FighterTape, tick: u32) -> Vec<FighterInput> {
    let mut out: Vec<FighterInput> = tape
        .0
        .iter()
        .filter(|input| input.tick == tick)
        .copied()
        .collect();
    out.sort_by_key(|input| input.fighter.0);
    out
}

/// Subsystem hashes over the post-step sim state. Diagnostic only.
fn hashes_of(sim: &super::Sim) -> SystemHashes {
    let mut world_bytes = Vec::new();
    append_world(
        &sim.states,
        &sim.wishes,
        &sim.buttons,
        sim.fighters,
        &mut world_bytes,
    );
    let mut world = Fnv64::new();
    world.write(&world_bytes);

    let mut space_bytes = Vec::new();
    append_space(sim, &mut space_bytes);
    let mut space = Fnv64::new();
    space.write(&space_bytes);

    let mut combat_bytes = Vec::new();
    append_combat(sim, &mut combat_bytes);
    let mut combat = Fnv64::new();
    combat.write(&combat_bytes);

    let mut rng = Fnv64::new();
    rng.write(&sim.tick.to_le_bytes());
    rng.write_u8(super::SIM_TRACK as u8);

    let mut ai_bytes = Vec::new();
    append_ai(sim, &mut ai_bytes);
    let mut ai = Fnv64::new();
    ai.write(&ai_bytes);

    SystemHashes {
        world: world.finish(),
        space: space.finish(),
        combat: combat.finish(),
        rng: rng.finish(),
        ai: ai.finish(),
    }
}

/// Append one system's canonical bytes. `None` on unknown system.
fn append_system_bytes(sim: &super::Sim, system: &str, out: &mut Vec<u8>) -> Option<()> {
    match system {
        "world" => append_world(&sim.states, &sim.wishes, &sim.buttons, sim.fighters, out),
        "space" => append_space(sim, out),
        "combat" => append_combat(sim, out),
        "rng" => {
            out.extend_from_slice(&sim.tick.to_le_bytes());
            out.push(super::SIM_TRACK as u8);
        }
        "ai" => append_ai(sim, out),
        _ => return None,
    }
    Some(())
}

/// Kinematic authority bytes: id, pos bits, yaw bits, wish pair, button.
fn append_world(
    states: &[super::FighterState],
    wishes: &[[i8; 2]; MAX_SLOTS],
    buttons: &[u8; MAX_SLOTS],
    fighters: usize,
    out: &mut Vec<u8>,
) {
    for slot in 0..fighters.min(states.len()) {
        push_world_slot(states, wishes, buttons, slot, out);
    }
}

/// One fighter's kinematic authority bytes: id, pos bits, yaw bits, wish
/// pair, button. Shared by the system split and the slot split so the two
/// layouts cannot drift apart.
fn push_world_slot(
    states: &[super::FighterState],
    wishes: &[[i8; 2]; MAX_SLOTS],
    buttons: &[u8; MAX_SLOTS],
    slot: usize,
    out: &mut Vec<u8>,
) {
    let Some(state) = states.get(slot) else {
        return;
    };
    out.push(state.id.0);
    for axis in state.pos {
        out.extend_from_slice(&canon_zero(axis).to_bits().to_le_bytes());
    }
    out.extend_from_slice(&canon_zero(state.yaw).to_bits().to_le_bytes());
    out.extend_from_slice(&wishes[slot][0].to_le_bytes());
    out.extend_from_slice(&wishes[slot][1].to_le_bytes());
    out.push(buttons[slot]);
}

/// Spatial authority bytes: destruction hash, limb pool counters and limbs,
/// break rows, controller hashes. The pool rides here so a snapshot that
/// forgets a limb counter diverges in `space` (the authority hash folds the
/// pool on v2 only).
fn append_space(sim: &super::Sim, out: &mut Vec<u8>) {
    out.extend_from_slice(&sim.destruction.state_hash().to_le_bytes());
    // Skip the pool snapshot's version and cap (bytes 0..8): a built sim and
    // a resumed one size the pool differently, and the cap is not state.
    out.extend_from_slice(sim.limbs.snapshot().get(8..).unwrap_or(&[]));
    let breaks = sim.breaks.last().map_or(&[][..], |row| &*row.events);
    out.extend_from_slice(&(breaks.len() as u64).to_le_bytes());
    for brk in breaks {
        out.extend_from_slice(&break_bytes(brk));
    }
    #[cfg(feature = "sim")]
    for controller in &sim.controllers {
        out.extend_from_slice(&controller.state_hash().to_le_bytes());
    }
}

/// Fight authority bytes: hp/stamina per slot plus tick event rows.
fn append_combat(sim: &super::Sim, out: &mut Vec<u8>) {
    for slot in 0..sim.fighters {
        push_combat_slot(sim, slot, out);
    }
    let events = sim.events.last().map_or(&[][..], |row| &*row.events);
    out.extend_from_slice(&(events.len() as u64).to_le_bytes());
    for event in events {
        out.extend_from_slice(&event_bytes(event));
    }
}

/// One fighter's fight authority bytes. Absent slot writes nothing, exactly
/// like the whole-system loop it replaces.
fn push_combat_slot(sim: &super::Sim, slot: usize, out: &mut Vec<u8>) {
    if let Some(combat) = sim.fight.fighter(slot as u8) {
        out.extend_from_slice(&combat.hp_permille.to_le_bytes());
        out.extend_from_slice(&combat.stamina.to_le_bytes());
    }
}

/// Brain-driven motion bytes: locomotion state per slot.
fn append_ai(sim: &super::Sim, out: &mut Vec<u8>) {
    for slot in 0..sim.fighters.min(sim.locos.len()) {
        push_ai_slot(sim, slot, out);
    }
}

/// One fighter's locomotion bytes; absent slot writes nothing.
fn push_ai_slot(sim: &super::Sim, slot: usize, out: &mut Vec<u8>) {
    let Some(loco) = sim.locos.get(slot) else {
        return;
    };
    let disc = u8::try_from(state_code(loco.state)).unwrap_or(u8::MAX);
    out.push(disc);
    for axis in [loco.phase(), loco.facing()[0], loco.facing()[1]] {
        out.extend_from_slice(&canon_zero(axis).to_bits().to_le_bytes());
    }
    out.push(u8::from(loco.is_airborne()));
    out.push(u8::from(loco.interrupt_active()));
}

/// Per-slot slice digests over the per-fighter bytes of one post-step state.
fn slot_hashes_of(sim: &super::Sim) -> SlotHashes {
    let mut slots = [None; MAX_SLOTS];
    for (slot, cell) in slots
        .iter_mut()
        .enumerate()
        .take(sim.fighters.min(MAX_SLOTS))
    {
        let mut bytes = Vec::new();
        push_world_slot(&sim.states, &sim.wishes, &sim.buttons, slot, &mut bytes);
        push_combat_slot(sim, slot, &mut bytes);
        push_ai_slot(sim, slot, &mut bytes);
        let mut fold = Fnv64::new();
        fold.write(&bytes);
        *cell = Some(fold.finish());
    }
    SlotHashes { slots }
}

#[cfg(test)]
mod tests {
    use super::super::{resume, run, run_prefix, ArenaConfig};
    use super::*;
    use engine_core::hash_bytes;
    use engine_input::fighter::Buttons;
    use engine_input::fighter::ATTACK;
    use engine_input::FighterId;

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 120,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    fn idle_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("course uses slots 0..2"),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                });
            }
        }
        FighterTape(inputs)
    }

    #[test]
    fn sync_rollback_resimulates_identical_x2() {
        let config = config();
        let tape = idle_tape(&config);
        let full = run(&config, &tape);
        assert_eq!(full.len(), config.ticks as usize);
        let first = verify_rollback(&config, &tape, 37)
            .expect("valid rollback")
            .expect("rollback agrees");
        let second = verify_rollback(&config, &tape, 37)
            .expect("valid rollback")
            .expect("rollback agrees");
        assert_eq!(first, full);
        assert_eq!(second, full);
        // Same through the public prefix/resume path, so the harness cannot
        // drift from the shipped rollback without this failing.
        let (snapshot, prefix) = run_prefix(&config, &tape, 37).expect("valid prefix");
        let mut rejoined = prefix;
        rejoined.extend(resume(snapshot, &config, &tape));
        assert_eq!(rejoined, full);
        // Edge splits hold too: tick 0 and the final tick.
        for at in [0, config.ticks] {
            assert_eq!(
                verify_rollback(&config, &tape, at)
                    .expect("valid edge rollback")
                    .expect("edge rollback agrees"),
                full,
                "split at {at}"
            );
        }
        assert!(verify_rollback(&config, &tape, config.ticks + 1).is_none());
    }

    #[test]
    fn sync_rollback_names_first_tick_and_system_on_perturbed_input() {
        let config = config();
        let tape = idle_tape(&config);
        // Perturb one wish at tick 60: the resume leg must fail loud on that
        // exact tick and blame the kinematic system first.
        let mut altered = tape.clone();
        let victim = altered
            .0
            .iter()
            .position(|input| input.tick == 60 && input.fighter.0 == 0)
            .expect("idle tape covers every slot");
        altered.0[victim].mx = 100;
        let (snapshot, _) = run_prefix(&config, &tape, 60).expect("valid prefix");
        let suffix = resume(snapshot, &config, &altered);
        // Oracle: the uninterrupted idle run. The rejoined leg replays the
        // idle prefix then the corrected wish, so it must diverge exactly
        // where the inputs first differ.
        let full = run(&config, &tape);
        assert_eq!(suffix.len(), full.len() - 60, "suffix covers 60..ticks");
        let rejoined: Vec<Checkpoint> = run(&config, &tape)
            .into_iter()
            .take(60)
            .chain(suffix)
            .collect();
        let first = rejoined
            .iter()
            .zip(full.iter())
            .position(|(a, b)| a != b)
            .expect("perturbed wish must move a checkpoint");
        assert_eq!(first, 60, "divergence starts at the perturbed tick");
        // The split names the same tick plus the world system plus inputs.
        let systems = run_system_hashes(&config, &altered);
        let base = run_system_hashes(&config, &tape);
        assert_eq!(
            first_diverging_system(&base[60], &systems[60]),
            Some("world")
        );
        let probe = SyncError {
            tick: 60,
            system: "world",
            slot: Some(0),
            expected: base[60].world,
            actual: systems[60].world,
            inputs: inputs_at(&altered, 60),
        };
        assert!(format!("{probe}").contains("tick 60"));
        assert!(format!("{probe}").contains("world"));
        assert!(format!("{probe}").contains("slot 0"));
        assert_eq!(probe.inputs.len(), 2);
        // A perturbed button still moves the world first (latched buttons
        // are kinematic authority), before combat answers ticks later.
        let mut attack = idle_tape(&config);
        attack.0[victim].buttons = Buttons(ATTACK);
        let attack_systems = run_system_hashes(&config, &attack);
        assert_eq!(
            first_diverging_system(&base[60], &attack_systems[60]),
            Some("world")
        );
    }

    #[test]
    fn sync_heavy_mode_clean_reports_zero_false_positives() {
        let config = config();
        let tape = idle_tape(&config);
        for at in 0..config.ticks {
            verify_rollback(&config, &tape, at)
                .expect("heavy")
                .expect("agree");
        }
        let tagged = system_tagged_bytes(&config, &tape, 60, "world").expect("tagged");
        let raw = system_canonical_bytes(&config, &tape, 60, "world").expect("raw");
        assert_eq!(tagged.bytes, raw);
        assert!((tagged.tick, tagged.system, tagged.offset) == (60, "world", 0));
        assert_eq!(tagged.len, raw.len());
        assert_eq!(hash_bytes(&tagged.bytes), hash_bytes(&raw));
        let mut altered = tape.clone();
        let victim = 60 * usize::from(config.fighters);
        altered.0[victim].mx = 100;
        let idle_rows = drive(&config, &tape, 61).expect("idle run");
        let moved_rows = drive(&config, &altered, 61).expect("moved run");
        let report = dual_save_report(60, &idle_rows[60], &moved_rows[60], &altered);
        assert!((report.tick, report.system, report.inputs.len()) == (60, "world", 2));
        assert_eq!(report.slot, Some(0), "the moved fighter is slot 0");
    }

    #[test]
    fn sync_rollback_systems_split_is_stable_and_sensitive() {
        let config = config();
        let tape = idle_tape(&config);
        let first = run_system_hashes(&config, &tape);
        let second = run_system_hashes(&config, &tape);
        assert_eq!(first.len(), config.ticks as usize);
        assert_eq!(first, second);
        for name in SYSTEM_NAMES {
            assert!(
                system_canonical_bytes(&config, &tape, 60, name).is_some(),
                "system {name} dumps bytes"
            );
        }
        assert!(system_canonical_bytes(&config, &tape, 60, "nope").is_none());
        assert!(system_canonical_bytes(&config, &tape, config.ticks + 1, "world").is_none());
        let a = system_canonical_bytes(&config, &tape, 60, "world").expect("world bytes");
        let b = system_canonical_bytes(&config, &tape, 60, "world").expect("world bytes");
        assert_eq!(a, b);
        assert_eq!(hash_bytes(&a), first[60].world);
        // A moved wish moves the world bytes but not the driver tag.
        let mut altered = tape.clone();
        altered.0[0].mx = 100;
        let moved = system_canonical_bytes(&config, &altered, 1, "world").expect("moved bytes");
        let still = system_canonical_bytes(&config, &tape, 1, "world").expect("still bytes");
        assert_ne!(moved, still);
        assert_eq!(
            system_canonical_bytes(&config, &altered, 1, "rng"),
            system_canonical_bytes(&config, &tape, 1, "rng"),
            "driver tag ignores wishes"
        );
    }

    /// `bits` mapped uniformly into `-span..=span`.
    fn spread(bits: u64, span: i64) -> i64 {
        let width = u64::try_from(2 * span + 1).expect("positive span");
        i64::try_from(bits % width).expect("below width") - span
    }

    /// Seeded moving and swinging tape: every fighter walks a drifting wish
    /// and swings on a staggered beat, so stamina and the combat split move.
    fn busy_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                let h = engine_core::mix64((u64::from(tick) << 8) | u64::from(fighter));
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("course uses slots 0..2"),
                    mx: i8::try_from(spread(h, 100)).expect("-100..=100"),
                    mz: i8::try_from(spread(h >> 16, 100)).expect("-100..=100"),
                    facing: i16::try_from(spread(h >> 32, 18_000)).expect("-18000..=18000"),
                    buttons: if tick % 23 == u32::from(fighter) * 7 {
                        Buttons(ATTACK)
                    } else {
                        Buttons::NONE
                    },
                });
            }
        }
        FighterTape(inputs)
    }

    #[test]
    fn sync_test_every_tick_at_depth_8_agrees() {
        // GGRS sync-test shape: 600 split points, each rolled back 8 ticks.
        let config = ArenaConfig {
            ticks: 600,
            ..config()
        };
        let tape = busy_tape(&config);
        let full = drive(&config, &tape, config.ticks).expect("valid run");
        assert!(
            full.windows(2)
                .any(|pair| pair[0].systems.combat != pair[1].systems.combat),
            "the tape must move combat or the sync test proves nothing"
        );
        for at in 0..config.ticks {
            let until = (at + 8).min(config.ticks);
            assert_eq!(
                resume_matches(&full, &config, &tape, at, until),
                Some(Ok(())),
                "split at {at}"
            );
        }
    }

    #[test]
    fn sync_test_catches_a_limb_counter_the_snapshot_forgot() {
        // The limb pool is not part of the match snapshot. Bump its tick
        // counter on the uninterrupted run at tick 100, as a pool that steps
        // would: the checkpoint cannot see it (v1 never folds the pool), the
        // `space` split must, at the first tick that carries it.
        let config = ArenaConfig {
            ticks: 120,
            ..config()
        };
        let tape = busy_tape(&config);
        let full = drive_with(&config, &tape, config.ticks, |sim| {
            if sim.tick == 101 {
                sim.limbs.step();
            }
        })
        .expect("valid run");
        let clean = drive(&config, &tape, config.ticks).expect("valid run");
        assert_eq!(
            full.iter().map(|row| row.checkpoint).collect::<Vec<_>>(),
            clean.iter().map(|row| row.checkpoint).collect::<Vec<_>>(),
            "the checkpoint never sees the pool"
        );
        let caught = resume_matches(&full, &config, &tape, 96, 104)
            .expect("valid window")
            .expect_err("the forgotten counter must be caught");
        assert_eq!((caught.tick, caught.system), (100, "space"));
        assert!(format!("{caught}").contains("tick 100 (system space)"));
        // A window that ends before the bump, or an unmodified run, agrees.
        assert_eq!(resume_matches(&full, &config, &tape, 40, 48), Some(Ok(())));
        assert_eq!(
            resume_matches(&clean, &config, &tape, 96, 120),
            Some(Ok(()))
        );
        // From any earlier split a full-depth check still names tick 100.
        let early = resume_matches(&full, &config, &tape, 10, config.ticks)
            .expect("valid window")
            .expect_err("caught from an early split");
        assert_eq!((early.tick, early.system), (100, "space"));
    }

    #[test]
    fn slot_split_names_the_perturbed_fighter() {
        let config = config();
        let tape = idle_tape(&config);
        let clean = drive(&config, &tape, 40).expect("clean run");
        let bent = drive_with(&config, &tape, 40, |sim| {
            if let Some(state) = sim.states.get_mut(1) {
                state.pos[0] += 0.25;
            }
        })
        .expect("bent run");
        let at = 39;
        assert_eq!(
            first_diverging_system(&clean[at].systems, &bent[at].systems),
            Some("world"),
            "the nudge must reach the world split"
        );
        assert_eq!(
            first_diverging_slot(&clean[at].slots, &bent[at].slots),
            Some(1),
            "the slot split names the fighter that moved, not slot 0"
        );
        assert_eq!(
            clean[at].slots.slots[2], None,
            "two fighters leave the rest of the slices empty"
        );
    }

    #[test]
    fn slot_split_matches_the_run_it_was_taken_from() {
        let config = config();
        let tape = idle_tape(&config);
        let rows = drive(&config, &tape, 30).expect("run");
        let direct = slot_hashes_at(&config, &tape, 30).expect("slots");
        assert_eq!(direct, rows[29].slots, "the rebuilt tick is the run's tick");
        assert_ne!(
            direct.slots[0], direct.slots[1],
            "each slice holds its own fighter's bytes"
        );
        assert!(slot_hashes_at(&config, &tape, config.ticks + 1).is_none());
    }

    #[test]
    fn a_shared_row_divergence_names_no_slot() {
        let config = config();
        let tape = idle_tape(&config);
        let rows = drive(&config, &tape, 4).expect("run");
        let reference = rows[3];
        let mut desynced = rows[3];
        desynced.systems.space ^= 1;
        assert_eq!(desynced.slots, reference.slots, "the pool is not a slot");
        let report = dual_save_report(3, &reference, &desynced, &tape);
        assert_eq!(report.system, "space");
        assert_eq!(report.slot, None, "no fighter owns the limb pool");
        assert!(format!("{report}").contains("(system space)"));
    }
}
