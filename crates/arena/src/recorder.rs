//! recorder: versioned decision datasets from any teacher (DT1).
//!
//! Consumer: DT2 student trainer reads [`Dataset`] rows of (observation,
//! action, outcome). Data owner: the caller owns match state; this module
//! owns the row schema plus the versioned binary codec. Units: meters for
//! positions, radians for yaw, health/damage in permille, fixed ticks.
//! Determinism scope: same rows twice encode byte-identical; decode never
//! touches simulation state. Error policy: structural mismatches record
//! nothing ([`on_fork_step`] returns `None`), bad bytes fail as
//! [`DecodeError`]; the worker never panics.
//!
//! Teacher-agnosticism: [`on_fork_step`] takes plain arena/combat data only
//! — never a provider, script or tape type — so fork runs, live matches,
//! script teachers and recorded human play all record the same [`Row`].
//! Binary layout (all little-endian): `EDT1` magic, `u16` version, `u32` row
//! count, then per row: tick `u32`, fighter count `u8`, per fighter (`u8`
//! id, 3x`f32` pos, `f32` yaw, `u16` hp), action (`u8` fighter plus the
//! canonical [`FighterInput`] bytes), outcome (`u16` damage before, `u16`
//! damage after, `u8` killed as 0/1). v1 is current; [`migrate`] passes v1
//! through byte-identical (checked no-op) and rejects anything else.
//!
//! [`FighterInput`]: engine_input::FighterInput

use super::{FighterState, MAX_SLOTS, MAX_TICKS};
use engine_combat::{fight::MAX_HP_PERMILLE, FightEvent, FightState};
use engine_input::{FighterError, FighterId, FighterInput, MAX_PARTICIPANTS};

/// Current dataset version; producers stamp it, the codec gates on it.
pub const DATASET_VERSION: u16 = 1;

/// Row bound: one row per fighter per tick of the longest legal match.
pub const MAX_ROWS: u32 = MAX_TICKS * (MAX_PARTICIPANTS as u32);

/// Codec magic: ASCII `EDT1` (engine decision traces).
const MAGIC: [u8; 4] = [0x45, 0x44, 0x54, 0x31];
/// Decode size bound, above the widest legal encoding (400k rows x 97 B).
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// One fighter's plain observation data: pose plus health.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FighterEntry {
    /// Fighter slot, `0..MAX_PARTICIPANTS`.
    pub id: u8,
    /// Feet position `[x, y, z]` in meters, finite.
    pub pos: [f32; 3],
    /// Facing yaw in radians, finite.
    pub yaw: f32,
    /// Health in permille, `0..=1000`.
    pub hp: u16,
}

/// Canonical filler for the unused tail of [`FighterEntries`]: slot 0 at the
/// origin with zero health. Only the live prefix ever escapes through
/// [`as_slice`](FighterEntries::as_slice).
const TAIL_FILLER: FighterEntry = FighterEntry {
    id: 0,
    pos: [0.0, 0.0, 0.0],
    yaw: 0.0,
    hp: 0,
};

/// One tick's fighter rows in fixed stack storage: `2..=MAX_PARTICIPANTS`
/// entries in slot order, no heap. [`on_fork_step`] builds these on the
/// calling frame, so row construction allocates nothing; the [`Dataset`]
/// still owns its row list, only the per-row entries move off the heap.
/// The empty value exists only for the playtest capture fallback when the
/// settle tick recorded nothing: producers always store
/// `2..=MAX_PARTICIPANTS` entries, and decoding rejects anything else.
#[derive(Clone, PartialEq, Debug)]
pub struct FighterEntries {
    /// Live prefix length of `entries`: `2..=MAX_PARTICIPANTS` from
    /// producers, `0` only from [`empty`](FighterEntries::empty).
    count: u8,
    /// Slot-order entries; only `..count` is live, the tail is [`TAIL_FILLER`].
    entries: [FighterEntry; MAX_SLOTS],
}

impl FighterEntries {
    /// Empty rows for the capture fallback: no live entries, canonical
    /// filler tail. Never enters a [`Dataset`]: the codec only encodes
    /// producer rows and [`decode`](Dataset::decode) rejects a zero count.
    pub const fn empty() -> Self {
        Self {
            count: 0,
            entries: [TAIL_FILLER; MAX_SLOTS],
        }
    }

    /// Copy a live slice into fixed storage. `None` when the count is outside
    /// `2..=MAX_PARTICIPANTS`; the tail stays canonical filler. Crate-visible
    /// because [`on_fork_step`] is the production caller, so the constructor
    /// cannot drift from the row builder it validates.
    pub(crate) fn copy_from_slice(entries: &[FighterEntry]) -> Option<Self> {
        let count = u8::try_from(entries.len()).ok()?;
        if !(2..=MAX_PARTICIPANTS).contains(&count) {
            return None;
        }
        let mut stored = [TAIL_FILLER; MAX_SLOTS];
        stored[..usize::from(count)].copy_from_slice(entries);
        Some(Self {
            count,
            entries: stored,
        })
    }

    /// Live entry count: `2..=MAX_PARTICIPANTS` from producers, `0` when empty.
    pub fn len(&self) -> usize {
        usize::from(self.count)
    }

    /// True only for [`empty`](FighterEntries::empty); a recorded observation
    /// always holds at least two entries.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Live entries in slot order.
    pub fn as_slice(&self) -> &[FighterEntry] {
        &self.entries[..usize::from(self.count)]
    }
}

impl core::ops::Deref for FighterEntries {
    type Target = [FighterEntry];

    fn deref(&self) -> &[FighterEntry] {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a FighterEntries {
    type Item = &'a FighterEntry;
    type IntoIter = core::slice::Iter<'a, FighterEntry>;

    fn into_iter(self) -> core::slice::Iter<'a, FighterEntry> {
        self.as_slice().iter()
    }
}

/// Pre-step observation for one tick: every fighter's pose plus health.
#[derive(Clone, PartialEq, Debug)]
pub struct ForkObservation {
    /// Fixed tick the observation was taken before.
    pub tick: u32,
    /// Fixed fighter rows in slot order, `2..=MAX_PARTICIPANTS` entries.
    pub fighters: FighterEntries,
}

/// The decided action one fighter applied on the observation tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecordedAction {
    /// Acting fighter slot; always equals `input.fighter.0`.
    pub fighter: u8,
    /// The validated input that ran (canonical bytes in the encoding).
    pub input: FighterInput,
}

/// What the action caused: the actor's cumulative damage around the tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutcomeDelta {
    /// Actor's cumulative damage dealt before this tick.
    pub damage_before: u16,
    /// Same counter after this tick (monotonic: saturating accumulation).
    pub damage_after: u16,
    /// True when any fighter was knocked out on this tick.
    pub killed: bool,
}

/// One training sample: what the teacher saw, did, and caused.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// Pre-step observation shared by the tick's rows.
    pub observation: ForkObservation,
    /// The acting fighter's decided input.
    pub action: RecordedAction,
    /// Damage outcome of the tick.
    pub outcome: OutcomeDelta,
}

/// Versioned row set for the DT2 trainer.
#[derive(Clone, PartialEq, Debug)]
pub struct Dataset {
    /// Codec version, always [`DATASET_VERSION`] from producers.
    pub version: u16,
    /// One row per fighter per recorded tick, tick-major.
    pub rows: Vec<Row>,
}

/// Codec failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// First four bytes are not `EDT1`.
    BadMagic,
    /// Fewer bytes than the header or a declared row needs.
    Truncated,
    /// Version newer than [`DATASET_VERSION`], culprit carried.
    FutureVersion(u16),
    /// Version older than v1 (no older version exists), culprit carried.
    UnsupportedVersion(u16),
    /// Declared row count above [`MAX_ROWS`], culprit carried.
    TooManyRows(u32),
    /// Whole buffer above the decode size bound, length carried.
    TooLarge(usize),
    /// Observation fighter count outside `2..=MAX_PARTICIPANTS`, culprit carried.
    BadFighterCount(usize),
    /// Fighter slot `>= MAX_PARTICIPANTS` (or action/input slot mismatch).
    BadFighter(u8),
    /// Health above full permille, culprit carried.
    BadHp(u16),
    /// Embedded [`FighterInput`] failed validation, source carried.
    BadInput(FighterError),
    /// Non-finite position or yaw in external bytes.
    NonFinite,
    /// Killed byte outside `0..=1`, culprit carried.
    BadKilled(u8),
    /// Damage-after below damage-before: not our encoder's invariant.
    DamageRegressed,
    /// Bytes left after the declared rows, remainder carried.
    Trailing(usize),
}

/// Plain per-step record input: pre-step observation sources plus the applied
/// action and its outcome. The teacher stays behind this struct: fork runs
/// pass brain/script inputs here, live matches pass tape inputs.
pub struct StepRecord<'a> {
    /// Fixed tick the action applied before.
    pub tick: u32,
    /// Pre-step fighter poses in slot order, `2..=MAX_PARTICIPANTS` entries.
    pub states: &'a [FighterState],
    /// Pre-step combat state (hp source); count must match `states`.
    pub fight: &'a FightState,
    /// The decided input applied on this tick for one fighter.
    pub input: FighterInput,
    /// Acting fighter's cumulative damage dealt before this tick.
    pub damage_before: u16,
    /// Same counter after this tick.
    pub damage_after: u16,
    /// True when any fighter was knocked out on this tick.
    pub killed: bool,
}

/// Build one dataset row from plain step data. `None` on structural
/// mismatch (wrong fighter count, unknown slot, invalid input, non-finite
/// pose or out-of-range hp): a corrupt teacher snapshot records nothing,
/// never a half row. Allocates nothing: the entries live in fixed stack
/// storage sized by the fighter contract, so the fixed tick can record every
/// step without touching the heap.
pub fn on_fork_step(step: &StepRecord<'_>) -> Option<Row> {
    let fighters = step.states.len();
    if !(2..=MAX_SLOTS).contains(&fighters) {
        return None;
    }
    if usize::from(step.fight.fighter_count) != fighters {
        return None;
    }
    step.input.validate().ok()?;
    let slot = usize::from(step.input.fighter.0);
    if slot >= fighters {
        return None;
    }
    let mut entries = [TAIL_FILLER; MAX_SLOTS];
    for (index, state) in step.states.iter().enumerate() {
        if !state.pos.iter().all(|axis| axis.is_finite()) || !state.yaw.is_finite() {
            return None;
        }
        let hp = step.fight.fighters[index].hp_permille;
        if hp > MAX_HP_PERMILLE {
            return None;
        }
        entries[index] = FighterEntry {
            id: u8::try_from(index).ok()?,
            pos: state.pos,
            yaw: state.yaw,
            hp,
        };
    }
    // The constructor re-checks the live count, so the row builder and its
    // validator cannot drift apart: a corrupt prefix still records nothing.
    let stored = FighterEntries::copy_from_slice(&entries[..fighters])?;
    Some(Row {
        observation: ForkObservation {
            tick: step.tick,
            fighters: stored,
        },
        action: RecordedAction {
            fighter: step.input.fighter.0,
            input: step.input,
        },
        outcome: OutcomeDelta {
            damage_before: step.damage_before,
            damage_after: step.damage_after,
            killed: step.killed,
        },
    })
}

/// Cumulative damage fold for live-match recording: hits add full damage to
/// the attacker, blocks add chip, knockdowns flag the tick. Same fold as the
/// fork metrics, so recorded outcomes agree with fork metrics over one run.
pub struct DamageLedger {
    damage: Vec<u16>,
}

impl DamageLedger {
    /// Fresh counters for `fighters` slots.
    pub fn new(fighters: usize) -> Self {
        Self {
            damage: vec![0; fighters],
        }
    }

    /// Fold one tick's combat events; returns true when the tick knocked
    /// out a fighter. Unknown slots (unreachable from validated runs) add
    /// nothing rather than trapping the recorder.
    pub fn apply(&mut self, events: &[FightEvent]) -> bool {
        let mut killed = false;
        for event in events {
            match *event {
                FightEvent::Hit {
                    attacker, damage, ..
                } => {
                    let slot = usize::from(attacker);
                    if slot < self.damage.len() {
                        self.damage[slot] = self.damage[slot].saturating_add(damage);
                    }
                }
                FightEvent::Blocked { attacker, chip, .. } => {
                    let slot = usize::from(attacker);
                    if slot < self.damage.len() {
                        self.damage[slot] = self.damage[slot].saturating_add(chip);
                    }
                }
                FightEvent::Knockdown { .. } => killed = true,
                FightEvent::Parried { .. }
                | FightEvent::Dodged { .. }
                | FightEvent::StaminaOut { .. } => {}
            }
        }
        killed
    }

    /// Cumulative damage dealt by one slot (0 when out of range).
    pub fn damage(&self, slot: usize) -> u16 {
        self.damage.get(slot).copied().unwrap_or(0)
    }

    /// Whole counters in slot order: snapshot before a tick for before/after
    /// rows, read [`damage`](DamageLedger::damage) after it. Allocates one
    /// fresh [`Vec`] per call; the per-tick recorder uses
    /// [`snapshot_into`](DamageLedger::snapshot_into) instead so steady
    /// recording costs zero heap after warmup.
    pub fn snapshot(&self) -> Vec<u16> {
        self.damage.clone()
    }

    /// Live counters in slot order, borrowed: the zero-alloc read for the
    /// per-tick recorder path. Valid until the next
    /// [`apply`](DamageLedger::apply), which mutates the counters in place.
    pub fn as_slice(&self) -> &[u16] {
        &self.damage
    }

    /// Reuse a caller-owned scratch buffer for the before-tick snapshot:
    /// clears then copies, so after one warmup fill the steady tick performs
    /// no heap allocation. The caller hoists `out` outside the tick loop.
    pub fn snapshot_into(&self, out: &mut Vec<u16>) {
        out.clear();
        out.extend_from_slice(&self.damage);
    }
}

impl Dataset {
    /// Empty v1 dataset: a 0-tick recording still encodes and round-trips.
    pub fn empty() -> Self {
        Self {
            version: DATASET_VERSION,
            rows: Vec::new(),
        }
    }

    /// Versioned binary encoding: same rows twice give identical bytes.
    /// Never truncates: every row is written, even past [`MAX_ROWS`]
    /// (decode then rejects the buffer rather than accepting a prefix).
    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(10usize.saturating_add(self.rows.len().saturating_mul(100)));
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(self.rows.len() as u32).to_le_bytes());
        for row in &self.rows {
            encode_row(&mut out, row);
        }
        out
    }

    /// Strict decode: the version gate runs first, rows parse bounded, and
    /// any trailing byte fails. See [`DecodeError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_BYTES {
            return Err(DecodeError::TooLarge(bytes.len()));
        }
        let version = header_version(bytes)?;
        if version != DATASET_VERSION {
            return Err(gate_error(version));
        }
        let mut cursor = Cursor {
            bytes,
            pos: MAGIC.len() + 2,
        };
        let count = cursor.u32()?;
        if count > MAX_ROWS {
            return Err(DecodeError::TooManyRows(count));
        }
        let mut rows = Vec::with_capacity(count as usize);
        for _ in 0..count {
            rows.push(decode_row(&mut cursor)?);
        }
        if cursor.pos != bytes.len() {
            return Err(DecodeError::Trailing(bytes.len() - cursor.pos));
        }
        Ok(Self { version, rows })
    }
}

/// Migration gate: v1 passes through byte-identical (checked no-op);
/// anything else fails, so DT2 never trains on a misread schema.
pub fn migrate(bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let version = header_version(bytes)?;
    if version == DATASET_VERSION {
        Ok(bytes.to_vec())
    } else {
        Err(gate_error(version))
    }
}

/// Version shift for any non-current version: newer is future, older is
/// unsupported (no older version exists).
fn gate_error(version: u16) -> DecodeError {
    if version > DATASET_VERSION {
        DecodeError::FutureVersion(version)
    } else {
        DecodeError::UnsupportedVersion(version)
    }
}

/// Magic plus version from any buffer: short buffers are truncated, wrong
/// leading bytes are bad magic.
fn header_version(bytes: &[u8]) -> Result<u16, DecodeError> {
    if bytes.len() < MAGIC.len() {
        return Err(DecodeError::Truncated);
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    if bytes.len() < MAGIC.len() + 2 {
        return Err(DecodeError::Truncated);
    }
    Ok(u16::from_le_bytes([
        bytes[MAGIC.len()],
        bytes[MAGIC.len() + 1],
    ]))
}

/// Bounded forward reader: every overrun is [`DecodeError::Truncated`].
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::Truncated)?;
        if end > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.take(1)?[0] as i8)
    }

    fn i16(&mut self) -> Result<i16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn f32(&mut self) -> Result<f32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(f32::from_bits(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])))
    }
}

/// One row in layout order: tick, fighter count, entries, action (fighter
/// byte plus canonical input bytes), outcome.
fn encode_row(out: &mut Vec<u8>, row: &Row) {
    out.extend_from_slice(&row.observation.tick.to_le_bytes());
    out.push(row.observation.fighters.count);
    for entry in &row.observation.fighters {
        out.push(entry.id);
        for axis in entry.pos {
            out.extend_from_slice(&axis.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&entry.yaw.to_bits().to_le_bytes());
        out.extend_from_slice(&entry.hp.to_le_bytes());
    }
    out.push(row.action.fighter);
    row.action.input.append_bytes(out);
    out.extend_from_slice(&row.outcome.damage_before.to_le_bytes());
    out.extend_from_slice(&row.outcome.damage_after.to_le_bytes());
    out.push(u8::from(row.outcome.killed));
}

/// Mirror of [`encode_row`]: every range, count and flag is validated, so
/// foreign bytes can only parse when they match our encoder's invariants.
fn decode_row(cursor: &mut Cursor<'_>) -> Result<Row, DecodeError> {
    let tick = cursor.u32()?;
    let count = cursor.u8()?;
    if !(2..=MAX_PARTICIPANTS).contains(&count) {
        return Err(DecodeError::BadFighterCount(usize::from(count)));
    }
    let mut entries = [TAIL_FILLER; MAX_SLOTS];
    for slot in entries.iter_mut().take(usize::from(count)) {
        let id = cursor.u8()?;
        if id >= MAX_PARTICIPANTS {
            return Err(DecodeError::BadFighter(id));
        }
        let pos = [cursor.f32()?, cursor.f32()?, cursor.f32()?];
        let yaw = cursor.f32()?;
        if !pos.iter().all(|axis| axis.is_finite()) || !yaw.is_finite() {
            return Err(DecodeError::NonFinite);
        }
        let hp = cursor.u16()?;
        if hp > MAX_HP_PERMILLE {
            return Err(DecodeError::BadHp(hp));
        }
        *slot = FighterEntry { id, pos, yaw, hp };
    }
    let fighter = cursor.u8()?;
    // Mirror of `encode_row` plus the canonical input bytes: the action
    // starts with the row's fighter byte, then the full `append_bytes`
    // sequence (tick, fighter, wishes, facing, buttons). The inner fighter
    // byte must repeat the outer one.
    let tick_in = cursor.u32()?;
    let inner = cursor.u8()?;
    if inner != fighter {
        return Err(DecodeError::BadFighter(inner));
    }
    let input = FighterInput {
        tick: tick_in,
        fighter: FighterId::new(fighter).ok_or(DecodeError::BadFighter(fighter))?,
        mx: cursor.i8()?,
        mz: cursor.i8()?,
        facing: cursor.i16()?,
        buttons: engine_input::Buttons(cursor.u8()?),
    };
    input.validate().map_err(DecodeError::BadInput)?;
    let damage_before = cursor.u16()?;
    let damage_after = cursor.u16()?;
    if damage_after < damage_before {
        return Err(DecodeError::DamageRegressed);
    }
    let killed = match cursor.u8()? {
        0 => false,
        1 => true,
        other => return Err(DecodeError::BadKilled(other)),
    };
    Ok(Row {
        observation: ForkObservation {
            tick,
            fighters: FighterEntries { count, entries },
        },
        action: RecordedAction { fighter, input },
        outcome: OutcomeDelta {
            damage_before,
            damage_after,
            killed,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::{Buttons, ATTACK};

    fn entry(id: u8) -> FighterEntry {
        FighterEntry {
            id,
            pos: [1.0, 0.0, -2.0],
            yaw: 0.5,
            hp: 1000,
        }
    }

    fn input(tick: u32, fighter: u8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(fighter).expect("sample uses slots 0..2"),
            mx: 10,
            mz: -20,
            facing: 9_000,
            buttons: Buttons::NONE,
        }
    }

    fn sample_row(tick: u32, fighter: u8) -> Row {
        Row {
            observation: ForkObservation {
                tick,
                fighters: FighterEntries::copy_from_slice(&[entry(0), entry(1)])
                    .expect("sample holds two entries"),
            },
            action: RecordedAction {
                fighter,
                input: input(tick, fighter),
            },
            outcome: OutcomeDelta {
                damage_before: 0,
                damage_after: u16::from(fighter) * 120,
                killed: false,
            },
        }
    }

    fn sample_dataset() -> Dataset {
        Dataset {
            version: DATASET_VERSION,
            rows: vec![sample_row(7, 0), sample_row(7, 1), sample_row(8, 0)],
        }
    }

    #[test]
    fn encode_decode_roundtrip_is_byte_stable() {
        let dataset = sample_dataset();
        let bytes = dataset.encode();
        assert_eq!(Dataset::decode(&bytes).expect("own bytes decode"), dataset);
        assert_eq!(dataset.encode(), bytes);
    }

    #[test]
    fn empty_dataset_roundtrips() {
        let empty = Dataset::empty();
        assert_eq!(empty.version, DATASET_VERSION);
        assert!(empty.rows.is_empty());
        assert_eq!(
            Dataset::decode(&empty.encode()).expect("empty decodes"),
            empty
        );
    }

    #[test]
    fn future_version_rejected_by_decode_and_migrate() {
        let mut bytes = sample_dataset().encode();
        bytes[MAGIC.len()] = DATASET_VERSION.wrapping_add(1) as u8;
        assert_eq!(
            Dataset::decode(&bytes),
            Err(DecodeError::FutureVersion(DATASET_VERSION + 1))
        );
        assert_eq!(
            migrate(&bytes),
            Err(DecodeError::FutureVersion(DATASET_VERSION + 1))
        );
    }

    #[test]
    fn migrate_v1_is_a_checked_noop() {
        for dataset in [Dataset::empty(), sample_dataset()] {
            let bytes = dataset.encode();
            assert_eq!(migrate(&bytes).expect("v1 migrates"), bytes);
        }
    }

    #[test]
    fn truncated_suffixes_rejected() {
        let bytes = sample_dataset().encode();
        // Every strict prefix keeps the magic but loses declared rows.
        for cut in [0, 1, 3, 4, 5, 6, 9, 10, bytes.len() / 2, bytes.len() - 1] {
            assert_eq!(
                Dataset::decode(&bytes[..cut]),
                Err(DecodeError::Truncated),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn bad_magic_and_trailing_rejected() {
        let mut bytes = sample_dataset().encode();
        bytes[0] ^= 0xFF;
        assert_eq!(Dataset::decode(&bytes), Err(DecodeError::BadMagic));
        assert_eq!(migrate(&bytes), Err(DecodeError::BadMagic));

        let mut trailed = sample_dataset().encode();
        trailed.push(0);
        assert_eq!(Dataset::decode(&trailed), Err(DecodeError::Trailing(1)));
    }

    #[test]
    fn declared_row_overflow_rejected_before_reading() {
        // Header only: magic + v1 + MAX_ROWS + 1 declares an illegal table.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&DATASET_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(MAX_ROWS + 1).to_le_bytes());
        assert_eq!(
            Dataset::decode(&bytes),
            Err(DecodeError::TooManyRows(MAX_ROWS + 1))
        );
    }

    /// Single 2-fighter row layout: header 10 B, then tick@10..14, count@14,
    /// entry0@15..34 (id@15, pos@16..28, yaw@28..32, hp@32..34),
    /// entry1@34..53, action@53..64 (fighter@53, tick@54..58, slot@58,
    /// mx@59, mz@60, facing@61..63, buttons@63), outcome@64..69
    /// (before@64..66, after@66..68, killed@68). 69 B total.
    fn single_row_bytes() -> Vec<u8> {
        let dataset = Dataset {
            version: DATASET_VERSION,
            rows: vec![sample_row(7, 0)],
        };
        let bytes = dataset.encode();
        assert_eq!(bytes.len(), 69);
        bytes
    }

    #[test]
    fn corrupt_payloads_rejected_with_cause() {
        let mut bad_buttons = single_row_bytes();
        bad_buttons[63] = 0x08;
        assert_eq!(
            Dataset::decode(&bad_buttons),
            Err(DecodeError::BadInput(FighterError::UnknownButtons(0x08)))
        );

        let mut bad_hp = single_row_bytes();
        bad_hp[32..34].copy_from_slice(&1001u16.to_le_bytes());
        assert_eq!(Dataset::decode(&bad_hp), Err(DecodeError::BadHp(1001)));

        let mut bad_killed = single_row_bytes();
        bad_killed[68] = 2;
        assert_eq!(Dataset::decode(&bad_killed), Err(DecodeError::BadKilled(2)));

        let mut regressed = single_row_bytes();
        regressed[64..66].copy_from_slice(&50u16.to_le_bytes());
        regressed[66..68].copy_from_slice(&49u16.to_le_bytes());
        assert_eq!(
            Dataset::decode(&regressed),
            Err(DecodeError::DamageRegressed)
        );

        let mut bad_pos = single_row_bytes();
        bad_pos[16..20].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert_eq!(Dataset::decode(&bad_pos), Err(DecodeError::NonFinite));
    }

    #[test]
    fn ledger_folds_hits_chips_and_knockdowns() {
        let mut ledger = DamageLedger::new(2);
        assert!(!ledger.apply(&[]));
        assert_eq!(ledger.damage(0), 0);
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        let chip = FightEvent::Blocked {
            attacker: 1,
            victim: 0,
            chip: 24,
        };
        assert!(!ledger.apply(&[hit, chip]));
        assert_eq!(ledger.damage(0), 120);
        assert_eq!(ledger.damage(1), 24);
        let down = FightEvent::Knockdown {
            attacker: 0,
            victim: 1,
        };
        assert!(ledger.apply(&[down]));
        assert_eq!(ledger.snapshot(), vec![120, 24]);
    }

    #[test]
    fn on_fork_step_rejects_structural_mismatch() {
        use engine_combat::FightState;

        let states = vec![
            FighterState {
                id: FighterId::new(0).expect("slot 0 legal"),
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterState {
                id: FighterId::new(1).expect("slot 1 legal"),
                pos: [1.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ];
        let good_input = input(3, 0);
        fn step<'a>(
            states: &'a [FighterState],
            fight: &'a FightState,
            step_input: FighterInput,
        ) -> StepRecord<'a> {
            StepRecord {
                tick: 3,
                states,
                fight,
                input: step_input,
                damage_before: 0,
                damage_after: 0,
                killed: false,
            }
        }
        let fight2 = FightState::new(2).expect("2 fighters legal");
        let row = on_fork_step(&step(&states, &fight2, good_input)).expect("good step records");
        assert_eq!(row.observation.tick, 3);
        assert_eq!(row.observation.fighters.len(), 2);
        assert_eq!(row.action.input, good_input);

        // Combat count must match the pose slice.
        let fight3 = FightState::new(3).expect("3 fighters legal");
        assert!(on_fork_step(&step(&states, &fight3, good_input)).is_none());
        // Unknown slots and invalid inputs record nothing.
        let mut unknown_slot = good_input;
        unknown_slot.fighter = FighterId(MAX_PARTICIPANTS);
        assert!(on_fork_step(&step(&states, &fight2, unknown_slot)).is_none());
        let mut bad = good_input;
        bad.buttons = Buttons(ATTACK | 0x08);
        assert!(on_fork_step(&step(&states, &fight2, bad)).is_none());
        // Non-finite poses record nothing.
        let mut nasty = states.clone();
        nasty[0].pos[0] = f32::NAN;
        let record = StepRecord {
            states: &nasty,
            ..step(&states, &fight2, good_input)
        };
        assert!(on_fork_step(&record).is_none());
    }

    #[test]
    fn fighter_entries_copy_validates_and_reads_as_slice() {
        assert!(FighterEntries::copy_from_slice(&[]).is_none());
        assert!(FighterEntries::copy_from_slice(&[entry(0)]).is_none());
        // One past the contract can never store.
        let overfull = vec![entry(0); usize::from(MAX_PARTICIPANTS) + 1];
        assert!(FighterEntries::copy_from_slice(&overfull).is_none());
        // The capture fallback is the only empty value: zero length, an
        // empty slice view, and no live entries to iterate.
        assert!(FighterEntries::empty().is_empty());
        assert_eq!(FighterEntries::empty().len(), 0);
        assert!(FighterEntries::empty().as_slice().is_empty());
        let live = [entry(0), entry(1)];
        let stored = FighterEntries::copy_from_slice(&live).expect("two entries store");
        assert_eq!(stored.len(), 2);
        assert!(!stored.is_empty());
        assert_eq!(stored.as_slice(), live);
        // The slice views external consumers rely on agree: deref length,
        // indexing and the shared reference iterator.
        assert_eq!(stored.len(), stored.as_slice().len());
        assert_eq!(stored[1], entry(1));
        let iterated: Vec<FighterEntry> = (&stored).into_iter().copied().collect();
        assert_eq!(iterated, live);
    }

    #[test]
    fn on_fork_step_allocates_nothing() {
        use engine_combat::FightState;

        let states = [
            FighterState {
                id: FighterId::new(0).expect("slot 0 legal"),
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterState {
                id: FighterId::new(1).expect("slot 1 legal"),
                pos: [1.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ];
        let fight = FightState::new(2).expect("2 fighters legal");
        let step = StepRecord {
            tick: 3,
            states: &states,
            fight: &fight,
            input: input(3, 0),
            damage_before: 0,
            damage_after: 0,
            killed: false,
        };
        let first = on_fork_step(&step).expect("good step records");
        assert_eq!(first.observation.fighters.len(), 2);
        // Live guard: the shared counting allocator is the test binary's
        // global allocator, so any heap use inside row construction shows
        // up here and fails the bound.
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            for _ in 0..32 {
                let row = on_fork_step(&step).expect("good step records");
                std::hint::black_box(row);
            }
        });
        assert_eq!(allocs.calls, 0, "row construction must not touch the heap");
    }

    #[test]
    fn ledger_snapshot_into_reuses_scratch_without_heap() {
        let mut ledger = DamageLedger::new(2);
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        assert!(!ledger.apply(&[hit]));
        assert_eq!(ledger.as_slice(), &[120, 0]);
        // Owned snapshot and scratch agree, so the reused path records the
        // same before/after counters as the cloning path.
        let mut scratch = Vec::new();
        ledger.snapshot_into(&mut scratch);
        assert_eq!(scratch, ledger.snapshot());
        // Warmup reserves capacity once; the steady snapshot then moves no heap.
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            for _ in 0..32 {
                ledger.snapshot_into(&mut scratch);
                std::hint::black_box(&scratch);
            }
        });
        assert_eq!(
            allocs.calls, 0,
            "damage snapshot reuse must not touch the heap"
        );
        assert_eq!(scratch, vec![120, 0]);
    }

    #[test]
    fn recorded_tick_scratch_reuses_states_and_damage_without_heap() {
        use engine_combat::FightState;

        let states = [
            FighterState {
                id: FighterId::new(0).expect("slot 0 legal"),
                pos: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterState {
                id: FighterId::new(1).expect("slot 1 legal"),
                pos: [1.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ];
        let fight = FightState::new(2).expect("2 fighters legal");
        let ledger = DamageLedger::new(2);
        // Hoisted scratch exactly like `run_dataset`: poses plus damage
        // counters reserved once outside the tick loop.
        let mut before_states = Vec::with_capacity(states.len());
        let mut before_damage = Vec::with_capacity(states.len());
        before_states.extend_from_slice(&states);
        ledger.snapshot_into(&mut before_damage);
        let step = StepRecord {
            tick: 3,
            states: &before_states,
            fight: &fight,
            input: input(3, 0),
            damage_before: before_damage[0],
            damage_after: ledger.damage(0),
            killed: false,
        };
        assert!(on_fork_step(&step).is_some());
        // Steady ticks refill the same buffers: no heap after warmup.
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            for _ in 0..32 {
                before_states.clear();
                before_states.extend_from_slice(&states);
                ledger.snapshot_into(&mut before_damage);
                let tick_step = StepRecord {
                    tick: 3,
                    states: &before_states,
                    fight: &fight,
                    input: input(3, 0),
                    damage_before: before_damage[0],
                    damage_after: ledger.damage(0),
                    killed: false,
                };
                std::hint::black_box(on_fork_step(&tick_step));
            }
        });
        assert_eq!(
            allocs.calls, 0,
            "recorder tick scratch must not touch the heap"
        );
    }

    fn arena_config() -> crate::ArenaConfig {
        crate::ArenaConfig {
            seed: 2040,
            ticks: 120,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    /// Relentless script-teacher input for one fighter over a window.
    fn attack_spam(fighter: u8, base: u32, steps: u32) -> Vec<FighterInput> {
        let id = FighterId::new(fighter).expect("sample uses slots 0..2");
        (0..steps)
            .map(|index| FighterInput {
                tick: base + index,
                fighter: id,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            })
            .collect()
    }

    #[test]
    fn script_and_tape_teachers_share_one_schema() {
        use crate::fork::{Fork, ForkProposal};
        use crate::{run_dataset, run_prefix};

        let config = arena_config();
        // Script teacher: a fork whose fighter 0 runs attack spam.
        let (snapshot, _) =
            run_prefix(&config, &crate::FighterTape::default(), 10).expect("valid prefix");
        let mut fork = Fork::from_snapshot(snapshot, 10).expect("tick matches");
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: 0,
            inputs: attack_spam(0, 10, 60),
        })
        .expect("spam validates");
        let (_, script_ds) = fork.run_recorded(60);
        assert_eq!(script_ds.rows.len(), 60 * 2);
        // Human-tape teacher: a live match over an explicit idle tape.
        let mut tape_inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                tape_inputs.push(input(tick, fighter));
            }
        }
        tape_inputs.sort_by_key(|entry| (entry.tick, entry.fighter.0));
        let tape = crate::FighterTape(tape_inputs);
        let tape_ds = run_dataset(&config, &tape).expect("valid tape records");
        assert_eq!(
            tape_ds.rows.len(),
            config.ticks as usize * usize::from(config.fighters)
        );
        // Live recording is deterministic.
        assert_eq!(
            run_dataset(&config, &tape).expect("valid tape records"),
            tape_ds
        );
        // Both teachers share the versioned schema: same version, same
        // shape, same codec round-trip through the migration gate.
        for dataset in [&script_ds, &tape_ds] {
            assert_eq!(dataset.version, DATASET_VERSION);
            for row in &dataset.rows {
                assert_eq!(row.observation.fighters.len(), 2);
                row.action.input.validate().expect("actions validate");
                assert_eq!(row.action.fighter, row.action.input.fighter.0);
                assert!(row.outcome.damage_after >= row.outcome.damage_before);
            }
            let bytes = dataset.encode();
            assert_eq!(migrate(&bytes).expect("v1 migrates"), bytes);
            assert_eq!(Dataset::decode(&bytes).expect("v1 decodes"), *dataset);
        }
    }
}
