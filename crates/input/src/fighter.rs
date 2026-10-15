//! fighter: acyclic fighter contract. `engine-input` owns `FighterId`
//! and `FighterInput`; `arena` owns match state and maps snapshots into the
//! combat/AI observation types those lanes define. No lower crate imports
//! arena, combat, AI or provider types.
//!
//! Units: wishes are quantized camera-relative integers in
//! `-WISH_RANGE..=WISH_RANGE`; facing is centidegrees
//! (`-18000..=18000`); ticks are zero-based fixed steps. All types are
//! `Copy` and hash through [`FighterInput::append_bytes`] (little-endian).

/// Canonical match slots in one headless match: `2..=MAX_PARTICIPANTS`
/// (2–16 today; siege scales this tomorrow). Single source: arena, combat,
/// net, ui and game read this const and declare no private cap. Server
/// connection capacity (`engine-net` `server::MAX_PLAYERS = 64`) is a
/// separate transport bound, not match slots; the provider observation
/// window (`MAX_FIGHTERS_PER_OBSERVATION = 8`) is a separate view bound.
pub const MAX_PARTICIPANTS: u8 = 16;
/// Wish axis range: `-100..=100` (percent of top speed).
pub const WISH_RANGE: i8 = 100;
/// Facing range in centidegrees: `-18000..=18000`.
pub const FACING_RANGE: i16 = 18_000;

/// Attack button bit in [`Buttons`].
pub const ATTACK: u8 = 0x01;
/// Block button bit in [`Buttons`].
pub const BLOCK: u8 = 0x02;
/// Dodge button bit in [`Buttons`].
pub const DODGE: u8 = 0x04;
/// Button bits the contract accepts; anything else is rejected.
pub const KNOWN_BUTTONS: u8 = ATTACK | BLOCK | DODGE;

/// Stable fighter slot in `0..MAX_PARTICIPANTS`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FighterId(pub u8);

impl FighterId {
    /// Slot constructor; `None` when `id >= MAX_PARTICIPANTS`.
    pub const fn new(id: u8) -> Option<Self> {
        if id < MAX_PARTICIPANTS {
            Some(Self(id))
        } else {
            None
        }
    }

    /// Raw slot value.
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Button bitset; only [`KNOWN_BUTTONS`] bits are legal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Buttons(pub u8);

impl Buttons {
    /// Empty bitset.
    pub const NONE: Self = Self(0);

    /// True when no unknown bits are set.
    pub const fn is_legal(self) -> bool {
        self.0 & !KNOWN_BUTTONS == 0
    }
}

/// One validated fighter command for one fixed tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FighterInput {
    /// Zero-based fixed tick this input applies before.
    pub tick: u32,
    /// Which fighter moves.
    pub fighter: FighterId,
    /// Forward wish, `-100..=100` percent of top speed.
    pub mx: i8,
    /// Right wish, `-100..=100` percent of top speed.
    pub mz: i8,
    /// Absolute facing in centidegrees, `-18000..=18000`.
    pub facing: i16,
    /// Attack/block/dodge bits.
    pub buttons: Buttons,
}

impl FighterInput {
    /// Canonical little-endian bytes for tick hashing.
    pub fn append_bytes(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.tick.to_le_bytes());
        bytes.push(self.fighter.0);
        bytes.extend_from_slice(&self.mx.to_le_bytes());
        bytes.extend_from_slice(&self.mz.to_le_bytes());
        bytes.extend_from_slice(&self.facing.to_le_bytes());
        bytes.push(self.buttons.0);
    }

    /// Range-check every field; unknown button bits fail.
    /// The [`FighterInput::append_bytes`] layout is frozen: a per-fighter
    /// record is 10 bytes whatever the participant cap is.
    pub fn validate(self) -> Result<(), FighterError> {
        if self.fighter.0 >= MAX_PARTICIPANTS {
            return Err(FighterError::UnknownFighter(self.fighter.0));
        }
        if self.mx < -WISH_RANGE || self.mx > WISH_RANGE {
            return Err(FighterError::WishOutOfRange(self.mx));
        }
        if self.mz < -WISH_RANGE || self.mz > WISH_RANGE {
            return Err(FighterError::WishOutOfRange(self.mz));
        }
        if self.facing < -FACING_RANGE || self.facing > FACING_RANGE {
            return Err(FighterError::FacingOutOfRange(self.facing));
        }
        if !self.buttons.is_legal() {
            return Err(FighterError::UnknownButtons(self.buttons.0));
        }
        Ok(())
    }
}

/// Fighter contract failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FighterError {
    /// Fighter slot `>= MAX_PARTICIPANTS`, culprit carried.
    UnknownFighter(u8),
    /// Wish axis outside `-100..=100`, culprit carried.
    WishOutOfRange(i8),
    /// Facing outside `-18000..=18000`, culprit carried.
    FacingOutOfRange(i16),
    /// Button byte with unknown bits, raw byte carried.
    UnknownButtons(u8),
    /// Tape not sorted by `(tick, fighter)`, index carried.
    UnsortedTape(usize),
    /// Same `(tick, fighter)` twice, index carried.
    DuplicateInput(usize),
}

impl FighterError {
    /// Fixed-width digest payload for this failure, for refusal logs.
    ///
    /// Owner: input crate; consumers fold the value into deterministic
    /// digests without naming contract variants. Units: unitless `u64`
    /// fold. Threading: `Copy`, no shared state. Determinism: stable
    /// across platforms (little-endian bytes, `u64::MAX` fallback when
    /// the tape index exceeds 64 bits). Errors: never fails.
    pub fn range_detail(self) -> u64 {
        match self {
            Self::UnknownFighter(slot) => u64::from(slot),
            Self::WishOutOfRange(axis) => u64::from(u8::from_le_bytes(axis.to_le_bytes())),
            Self::FacingOutOfRange(face) => u64::from(u16::from_le_bytes(face.to_le_bytes())),
            Self::UnknownButtons(byte) => u64::from(byte),
            Self::UnsortedTape(index) => u64::try_from(index).unwrap_or(u64::MAX),
            Self::DuplicateInput(index) => u64::try_from(index).unwrap_or(u64::MAX),
        }
    }
}

/// Per-fighter input tape v2: validated, `(tick, fighter)`-sorted inputs.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FighterTape(pub Vec<FighterInput>);

impl FighterTape {
    /// Validate every input plus tape order; duplicates fail.
    pub fn validate(&self) -> Result<(), FighterError> {
        let mut prev: Option<(u32, u8)> = None;
        for (index, input) in self.0.iter().enumerate() {
            input.validate()?;
            let key = (input.tick, input.fighter.0);
            if let Some(p) = prev {
                if key < p {
                    return Err(FighterError::UnsortedTape(index));
                }
                if key == p {
                    return Err(FighterError::DuplicateInput(index));
                }
            }
            prev = Some(key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> FighterInput {
        FighterInput {
            tick: 7,
            fighter: FighterId::new(1).unwrap(),
            mx: 100,
            mz: -100,
            facing: 9_000,
            buttons: Buttons(ATTACK | BLOCK),
        }
    }

    #[test]
    fn valid_input_passes_and_hashes_stably() {
        let input = valid();
        input.validate().unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        input.append_bytes(&mut a);
        input.append_bytes(&mut b);
        assert_eq!(a, b);
        assert_eq!(a.len(), 4 + 1 + 1 + 1 + 2 + 1);
    }

    #[test]
    fn changed_input_changes_bytes() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        valid().append_bytes(&mut a);
        let mut other = valid();
        other.mx = 99;
        other.append_bytes(&mut b);
        assert_ne!(a, b);
    }

    #[test]
    fn bad_fighter_rejected() {
        assert_eq!(FighterId::new(15).map(|id| id.get()), Some(15));
        assert_eq!(FighterId::new(16), None);
        assert_eq!(FighterId::new(u8::MAX), None);
        let mut input = valid();
        input.fighter = FighterId(16);
        assert_eq!(input.validate(), Err(FighterError::UnknownFighter(16)));
        let mut wide = valid();
        wide.fighter = FighterId(15);
        wide.validate().expect("slot 15 is inside the 16-slot cap");
    }

    #[test]
    fn canonical_cap_single_sourced() {
        // One match-slot cap, no legacy alias.
        assert_eq!(MAX_PARTICIPANTS, 16);
    }

    #[test]
    fn sixteen_slot_boundary_validates() {
        for slot in [0u8, 1, 4, 15] {
            let mut input = valid();
            input.fighter = FighterId(slot);
            assert!(input.validate().is_ok(), "slot {slot} must validate");
        }
        for slot in [16u8, 17, u8::MAX] {
            let mut input = valid();
            input.fighter = FighterId(slot);
            assert_eq!(
                input.validate(),
                Err(FighterError::UnknownFighter(slot)),
                "slot {slot} must refuse"
            );
        }
    }

    #[test]
    fn c1_bytes_frozen_across_cap_change() {
        // Byte contract: per-fighter record stays 10 bytes, little-endian,
        // independent of the cap value (4-fighter tapes hash identically).
        let input = valid();
        let mut bytes = Vec::new();
        input.append_bytes(&mut bytes);
        assert_eq!(bytes.len(), 10);
        assert_eq!(&bytes[0..4], &7u32.to_le_bytes());
        assert_eq!(bytes[4], 1);
        assert_eq!(&bytes[7..9], &9_000i16.to_le_bytes());
    }

    #[test]
    fn signed_wishes_hash_as_twos_complement_bytes() {
        let mut input = valid();
        input.mx = -100;
        input.mz = -1;
        let mut bytes = Vec::new();
        input.append_bytes(&mut bytes);
        assert_eq!(bytes.len(), 10);
        assert_eq!(&bytes[5..7], &[0x9C, 0xFF]);
    }

    #[test]
    fn wish_and_facing_ranges_rejected() {
        let mut input = valid();
        input.mx = 101;
        assert_eq!(input.validate(), Err(FighterError::WishOutOfRange(101)));
        let mut input = valid();
        input.mz = -101;
        assert_eq!(input.validate(), Err(FighterError::WishOutOfRange(-101)));
        let mut input = valid();
        input.facing = 18_001;
        assert_eq!(
            input.validate(),
            Err(FighterError::FacingOutOfRange(18_001))
        );
    }

    #[test]
    fn unknown_buttons_rejected() {
        let mut input = valid();
        input.buttons = Buttons(0x08);
        assert_eq!(input.validate(), Err(FighterError::UnknownButtons(0x08)));
        assert!(!Buttons(0x08).is_legal());
        assert!(Buttons(ATTACK | BLOCK | DODGE).is_legal());
    }

    #[test]
    fn tape_order_and_duplicates_rejected() {
        let (a, b) = (
            FighterInput {
                tick: 0,
                fighter: FighterId::new(0).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 1,
                fighter: FighterId::new(0).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            },
        );
        FighterTape(vec![a, b]).validate().unwrap();
        assert_eq!(
            FighterTape(vec![b, a]).validate(),
            Err(FighterError::UnsortedTape(1))
        );
        assert_eq!(
            FighterTape(vec![a, a]).validate(),
            Err(FighterError::DuplicateInput(1))
        );
    }

    #[test]
    fn range_detail_folds_without_naming_variants_downstream() {
        assert_eq!(FighterError::UnknownFighter(16).range_detail(), 16);
        assert_eq!(FighterError::UnknownButtons(0x08).range_detail(), 8);
        assert_eq!(FighterError::UnsortedTape(1).range_detail(), 1);
        assert_eq!(FighterError::DuplicateInput(2).range_detail(), 2);
        // Signed payloads fold by little-endian byte, not sign extension:
        // -101i8 is 0x9B (155), -101 as u64 would be huge and fail here.
        assert_eq!(FighterError::WishOutOfRange(-101).range_detail(), 155);
        assert_eq!(FighterError::WishOutOfRange(101).range_detail(), 101);
        assert_eq!(
            FighterError::FacingOutOfRange(18_001).range_detail(),
            18_001
        );
        assert_eq!(
            FighterError::FacingOutOfRange(-18_001).range_detail(),
            u64::from((-18_001i16).to_le_bytes()[0])
                | (u64::from((-18_001i16).to_le_bytes()[1]) << 8)
        );
    }
}
