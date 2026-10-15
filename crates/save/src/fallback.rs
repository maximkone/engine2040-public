//! fallback: previous-generation load with observe notices.
//!
//! Consumer: game progression. Data owner: slot files `saveNN.e2040`
//! (current) plus `saveNN.e2040.prev` (previous); the caller owns the
//! [`RingBuffer`]. Units: generations (slot files), ticks (`u64`).
//! Determinism: messages come from slot digits and [`SaveErr`] labels only,
//! no clock; the same corrupt bytes give the same fallback and notices.
//! Errors: [`FallbackErr`]; total loss (no valid generation) fails loudly.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use engine_observe::{Event, EventCode, RingBuffer};

use super::{open, slot_name, verify, Envelope, SaveErr, MAX_TOTAL_BYTES, SLOTS};

/// Which stored generation a load returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadSource {
    /// The current slot file opened cleanly; no notice was emitted.
    Current,
    /// The current file failed and the previous generation served instead.
    Previous,
}

/// One loaded save plus the generation it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The opened and migrated envelope.
    pub envelope: Envelope,
    /// Which generation served this load.
    pub source: LoadSource,
}

/// Typed fallback failure. Every variant is loud: nothing falls back
/// silently and nothing forges state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackErr {
    /// Slot id out of range (`>= SLOTS`).
    BadSlot,
    /// The filesystem itself failed for both generations.
    Io {
        /// OS error kind for the failed read.
        kind: std::io::ErrorKind,
        /// Human detail (`save.io slot=NN ...`), diagnostics only.
        message: String,
    },
    /// No valid generation: current failed and the previous generation is
    /// missing or failed too. The caller must surface this, never start
    /// from a guess.
    TotalLoss {
        /// Slot that lost every generation.
        slot: u8,
        /// Why the current generation failed.
        current: SaveErr,
        /// Why the previous generation failed (`Truncated` when absent).
        previous: Option<SaveErr>,
    },
}

impl std::fmt::Display for FallbackErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FallbackErr::BadSlot => f.write_str("save slot out of range"),
            FallbackErr::Io { kind, message } => {
                write!(f, "save io failed ({kind:?}): {message}")
            }
            FallbackErr::TotalLoss {
                slot,
                current,
                previous,
            } => match previous {
                Some(prev) => write!(
                    f,
                    "save total loss slot={slot:02}: current={current:?} previous={prev:?}"
                ),
                None => write!(f, "save total loss slot={slot:02}: current={current:?}"),
            },
        }
    }
}

impl std::error::Error for FallbackErr {}

/// Previous-generation file name (`save03.e2040.prev`). Err on bad slot.
pub fn prev_name(slot: u8) -> Result<String, SaveErr> {
    if slot >= SLOTS {
        return Err(SaveErr::BadSlot);
    }
    Ok(format!("save{slot:02}.e2040.prev"))
}

/// True when the slot holds a current file (readable or not). Lets the game tell
/// "never saved" (fresh start) apart from "saved but unreadable" (fallback
/// or [`FallbackErr::TotalLoss`]).
#[must_use]
pub fn has_slot(dir: &Path, slot: u8) -> bool {
    slot_name(slot)
        .map(|name| dir.join(name).is_file())
        .unwrap_or(false)
}

/// Reads one slot file, at most one byte past [`MAX_TOTAL_BYTES`]. A longer
/// file comes back one byte over the cap, so callers refuse it without ever
/// heaping the rest, even when the file grows during the read.
fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let cap = u64::try_from(MAX_TOTAL_BYTES).map_or(u64::MAX, |max| max.saturating_add(1));
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(cap)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Back up the current file to its `.prev` generation before overwriting.
/// Called by [`super::save_generation`]; missing current means nothing to
/// keep, and a corrupt or oversized current never becomes the previous
/// generation. A failed backup aborts loudly with the current file untouched.
pub(crate) fn backup_current(dir: &Path, slot: u8) -> std::io::Result<()> {
    let current: PathBuf = dir.join(
        slot_name(slot)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad slot"))?,
    );
    let bytes = match read_capped(&current) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(bytes) => bytes,
    };
    // Allocation-free probe: `verify` runs the same gates as `open` without
    // heaping a throwaway payload copy (previously `open` allocated up to
    // 1 MiB here just to answer a bool).
    if verify(&bytes).is_err() {
        return Ok(());
    }
    let prev: PathBuf = dir.join(
        prev_name(slot)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad slot"))?,
    );
    let tmp = dir.join(format!(
        "{}.tmp",
        prev.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("save.prev")
    ));
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        // Flush before the rename publishes the backup; the handle closes
        // at the end of this block, before the rename.
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &prev)?;
    super::sync_dir(dir);
    Ok(())
}

/// Stable label for one open failure, used in notice messages.
fn cause_label(error: SaveErr) -> &'static str {
    match error {
        SaveErr::BadMagic => "bad-magic",
        SaveErr::BadVersion => "bad-version",
        SaveErr::Truncated => "truncated",
        SaveErr::CrcMismatch => "crc-mismatch",
        SaveErr::BadSlot => "bad-slot",
        SaveErr::TooLarge => "too-large",
    }
}

/// One open failure as a notice cause. Absent files read as truncated so
/// total loss stays uniformly typed.
enum OpenFault {
    Corrupt(SaveErr),
    Io(std::io::Error),
}

fn read_open(path: &Path) -> Result<Envelope, OpenFault> {
    match read_capped(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(OpenFault::Corrupt(SaveErr::Truncated))
        }
        Err(error) => Err(OpenFault::Io(error)),
        // Named here so garbage over the cap reports its size, not its magic.
        Ok(bytes) if bytes.len() > MAX_TOTAL_BYTES => Err(OpenFault::Corrupt(SaveErr::TooLarge)),
        Ok(bytes) => open(&bytes).map_err(OpenFault::Corrupt),
    }
}

fn fault_label(fault: &OpenFault) -> String {
    match fault {
        OpenFault::Corrupt(error) => cause_label(*error).to_owned(),
        OpenFault::Io(error) => format!("io-{:?}", error.kind()),
    }
}

/// Load one slot: current when it opens, else the previous generation with
/// `save.corrupt` plus `heal.applied` notices in `ring`. Both generations
/// failed means [`FallbackErr::TotalLoss`] plus a `heal.failed` notice.
pub fn load_slot(
    dir: &Path,
    slot: u8,
    tick: u64,
    ring: &mut RingBuffer,
) -> Result<Loaded, FallbackErr> {
    if slot >= SLOTS {
        return Err(FallbackErr::BadSlot);
    }
    let current_path: PathBuf = dir.join(slot_name(slot).map_err(|_| FallbackErr::BadSlot)?);
    let prev_path: PathBuf = dir.join(prev_name(slot).map_err(|_| FallbackErr::BadSlot)?);
    let current_fault = match read_open(&current_path) {
        Ok(envelope) => {
            return Ok(Loaded {
                envelope,
                source: LoadSource::Current,
            });
        }
        Err(fault) => fault,
    };
    ring.push(Event::new(
        EventCode::SAVE_CORRUPT,
        tick,
        &format!(
            "save.corrupt slot={slot:02} gen=current cause={}",
            fault_label(&current_fault)
        ),
    ));
    match read_open(&prev_path) {
        Ok(envelope) => {
            ring.push(Event::new(
                EventCode::HEAL_APPLIED,
                tick,
                &format!("heal.applied cause=save.corrupt slot={slot:02} gen=previous"),
            ));
            Ok(Loaded {
                envelope,
                source: LoadSource::Previous,
            })
        }
        Err(prev_fault) => {
            ring.push(Event::new(
                EventCode::HEAL_FAILED,
                tick,
                &format!("heal.failed cause=save.corrupt slot={slot:02} quarantined"),
            ));
            Err(combine(slot, current_fault, prev_fault))
        }
    }
}

/// Merge two generation faults: corrupt pairs are total loss, any real IO
/// failure is a loud IO error naming the slot.
fn combine(slot: u8, current: OpenFault, previous: OpenFault) -> FallbackErr {
    match (current, previous) {
        (OpenFault::Corrupt(current), OpenFault::Corrupt(previous)) => FallbackErr::TotalLoss {
            slot,
            current,
            previous: Some(previous),
        },
        (OpenFault::Io(error), _) | (_, OpenFault::Io(error)) => FallbackErr::Io {
            kind: error.kind(),
            message: format!("save.io slot={slot:02} {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::{save_generation, MAX_PAYLOAD_BYTES};
    use super::*;
    use engine_observe::EventCode;

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-save-ah5-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir builds");
        dir
    }

    fn two_generations(dir: &Path, slot: u8) {
        save_generation(dir, slot, 7, 70, b"gen-one").expect("gen one saves");
        save_generation(dir, slot, 7, 71, b"gen-two").expect("gen two saves");
    }

    fn current_path(dir: &Path, slot: u8) -> PathBuf {
        dir.join(slot_name(slot).expect("test slot valid"))
    }

    fn ring(cap: usize) -> RingBuffer {
        RingBuffer::new(cap)
    }

    #[test]
    fn clean_load_serves_current_with_no_notice() {
        let dir = test_dir("clean");
        save_generation(&dir, 0, 7, 70, b"fresh").expect("save works");
        assert!(has_slot(&dir, 0));
        assert!(!has_slot(&dir, 1));
        assert!(!has_slot(&dir, 16));
        let mut events = ring(8);
        let loaded = load_slot(&dir, 0, 11, &mut events).expect("clean slot loads");
        assert_eq!(loaded.source, LoadSource::Current);
        assert_eq!(loaded.envelope.payload, b"fresh");
        assert!(events.is_empty(), "clean load emits no notice");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_slot_fails_loudly_without_touching_the_ring() {
        let dir = test_dir("badslot");
        let mut events = ring(8);
        assert_eq!(
            load_slot(&dir, 16, 0, &mut events),
            Err(FallbackErr::BadSlot)
        );
        assert_eq!(prev_name(16), Err(SaveErr::BadSlot));
        assert!(events.is_empty());
        assert_eq!(FallbackErr::BadSlot.to_string(), "save slot out of range");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One chaos row: name, the fault it crafts, and how to craft it.
    type Corruption = (&'static str, SaveErr, Box<dyn Fn(&mut Vec<u8>)>);

    /// Chaos sweep: every open-failure shape on the current file falls back
    /// to the previous generation with a `save.corrupt` plus `heal.applied`
    /// notice naming the fault.
    #[test]
    fn every_corrupt_current_falls_back_to_previous() {
        let corruptions: Vec<Corruption> = vec![
            (
                "bad-magic",
                SaveErr::BadMagic,
                Box::new(|bytes: &mut Vec<u8>| {
                    bytes[0] = b'Z';
                }),
            ),
            (
                "bad-version",
                SaveErr::BadVersion,
                Box::new(|bytes: &mut Vec<u8>| {
                    bytes[4] = super::super::VERSION + 1;
                }),
            ),
            (
                "truncated",
                SaveErr::Truncated,
                Box::new(|bytes: &mut Vec<u8>| {
                    bytes.truncate(10);
                }),
            ),
            (
                "crc-mismatch",
                SaveErr::CrcMismatch,
                Box::new(|bytes: &mut Vec<u8>| {
                    let last = 25 + bytes.len() - 29 - 1;
                    bytes[last] ^= 0xFF;
                }),
            ),
            (
                "too-large",
                SaveErr::TooLarge,
                Box::new(|bytes: &mut Vec<u8>| {
                    let len = (MAX_PAYLOAD_BYTES + 1) as u32;
                    bytes[21..25].copy_from_slice(&len.to_le_bytes());
                }),
            ),
        ];
        for (name, expected, corrupt) in corruptions {
            let dir = test_dir(&format!("chaos-{name}"));
            two_generations(&dir, 4);
            let mut bytes = std::fs::read(current_path(&dir, 4)).expect("current exists");
            corrupt(&mut bytes);
            // The crafted bytes really fail with the fault the row names.
            assert_eq!(
                open(&bytes),
                Err(expected),
                "row {name} crafts {expected:?}"
            );
            std::fs::write(current_path(&dir, 4), &bytes).expect("corrupt current");

            let mut events = ring(8);
            let loaded = load_slot(&dir, 4, 12, &mut events).expect("previous serves");
            assert_eq!(loaded.source, LoadSource::Previous, "row {name}");
            assert_eq!(loaded.envelope.payload, b"gen-one", "row {name}");
            assert_eq!(loaded.envelope.tick, 70, "row {name}");
            let corrupt_rows = events.query(Some(EventCode::SAVE_CORRUPT));
            assert_eq!(corrupt_rows.len(), 1, "row {name}");
            assert!(
                corrupt_rows[0].message.contains(name),
                "row {name} names its fault: {}",
                corrupt_rows[0].message
            );
            assert_eq!(
                events.query(Some(EventCode::HEAL_APPLIED)).len(),
                1,
                "row {name}"
            );
            assert!(
                events.query(Some(EventCode::HEAL_FAILED)).is_empty(),
                "row {name}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn rotation_keeps_previous_and_never_backs_up_corruption() {
        let dir = test_dir("rotation");
        save_generation(&dir, 2, 7, 70, b"gen-one").expect("gen one saves");
        // One generation only: no previous file yet.
        assert!(!dir.join(prev_name(2).expect("prev name")).exists());
        save_generation(&dir, 2, 7, 71, b"gen-two").expect("gen two saves");
        let prev_bytes =
            std::fs::read(dir.join(prev_name(2).expect("prev name"))).expect("prev exists");
        assert_eq!(open(&prev_bytes).expect("prev opens").payload, b"gen-one");

        // Corrupt the current file, then save again: the corrupt bytes must
        // not become the previous generation.
        let mut broken = std::fs::read(current_path(&dir, 2)).expect("current exists");
        broken[0] = b'Z';
        std::fs::write(current_path(&dir, 2), &broken).expect("corrupt current");
        save_generation(&dir, 2, 7, 72, b"gen-three").expect("save over corruption works");
        let mut events = ring(8);
        let loaded = load_slot(&dir, 2, 13, &mut events).expect("current loads");
        assert_eq!(loaded.source, LoadSource::Current);
        assert_eq!(loaded.envelope.payload, b"gen-three");
        // Previous still holds a valid older generation, never the corruption.
        let prev_bytes =
            std::fs::read(dir.join(prev_name(2).expect("prev name"))).expect("prev kept");
        open(&prev_bytes).expect("previous stays openable");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn total_loss_when_every_generation_fails() {
        let dir = test_dir("totalloss");
        two_generations(&dir, 6);
        // Corrupt both generations differently: both faults stay visible.
        let mut current = std::fs::read(current_path(&dir, 6)).expect("current exists");
        current[0] = b'Z';
        std::fs::write(current_path(&dir, 6), &current).expect("corrupt current");
        let prev = dir.join(prev_name(6).expect("prev name"));
        let mut older = std::fs::read(&prev).expect("prev exists");
        older.truncate(5);
        std::fs::write(&prev, &older).expect("corrupt previous");

        let mut events = ring(8);
        let error = load_slot(&dir, 6, 14, &mut events).expect_err("no valid generation");
        assert_eq!(
            error,
            FallbackErr::TotalLoss {
                slot: 6,
                current: SaveErr::BadMagic,
                previous: Some(SaveErr::Truncated),
            }
        );
        assert!(
            error.to_string().contains("total loss"),
            "total loss says so loudly: {error}"
        );
        assert_eq!(events.query(Some(EventCode::SAVE_CORRUPT)).len(), 1);
        assert!(events.query(Some(EventCode::HEAL_APPLIED)).is_empty());
        assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_slot_is_total_loss_not_a_guess() {
        let dir = test_dir("missing");
        assert!(!has_slot(&dir, 9));
        let mut events = ring(8);
        let error = load_slot(&dir, 9, 15, &mut events).expect_err("absent slot is loud");
        assert_eq!(
            error,
            FallbackErr::TotalLoss {
                slot: 9,
                current: SaveErr::Truncated,
                previous: Some(SaveErr::Truncated),
            }
        );
        assert_eq!(events.query(Some(EventCode::SAVE_CORRUPT)).len(), 1);
        assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn same_corrupt_bytes_twice_give_same_fallback_and_notices() {
        fn attempt(tag: &str) -> (Loaded, Vec<String>) {
            let dir = test_dir(&format!("replay-{tag}"));
            two_generations(&dir, 1);
            let mut bytes = std::fs::read(current_path(&dir, 1)).expect("current exists");
            bytes[0] = b'Z';
            std::fs::write(current_path(&dir, 1), &bytes).expect("corrupt current");
            let mut events = ring(8);
            let loaded = load_slot(&dir, 1, 21, &mut events).expect("previous serves");
            let messages: Vec<String> = events.iter().map(|event| event.message.clone()).collect();
            std::fs::remove_dir_all(&dir).unwrap();
            (loaded, messages)
        }
        assert_eq!(attempt("a"), attempt("b"));
    }

    #[test]
    fn read_capped_stops_one_byte_past_the_cap() {
        let dir = test_dir("capped");
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; MAX_TOTAL_BYTES + 4096]).expect("plant big file");
        let capped = read_capped(&path).expect("reads");
        assert_eq!(capped.len(), MAX_TOTAL_BYTES + 1, "one byte over the cap");
        std::fs::write(&path, vec![7u8; MAX_TOTAL_BYTES]).expect("plant exact file");
        assert_eq!(read_capped(&path).expect("reads").len(), MAX_TOTAL_BYTES);
        assert_eq!(
            read_capped(&dir.join("missing")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_current_falls_back_naming_its_size() {
        let dir = test_dir("oversize-gate");
        two_generations(&dir, 3);
        std::fs::write(current_path(&dir, 3), vec![0u8; MAX_TOTAL_BYTES + 1])
            .expect("oversized current");
        let mut events = ring(8);
        let loaded = load_slot(&dir, 3, 31, &mut events).expect("previous serves");
        assert_eq!(loaded.source, LoadSource::Previous);
        assert_eq!(loaded.envelope.payload, b"gen-one");
        let corrupt = events.query(Some(EventCode::SAVE_CORRUPT));
        assert_eq!(corrupt.len(), 1);
        assert!(
            corrupt[0].message.contains("too-large"),
            "oversize names its fault: {}",
            corrupt[0].message
        );
        assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_skips_oversized_current_without_copying_it() {
        let dir = test_dir("oversize-backup");
        save_generation(&dir, 8, 7, 70, b"gen-one").expect("gen one saves");
        save_generation(&dir, 8, 7, 71, b"gen-two").expect("gen two saves");
        let prev_path = dir.join(prev_name(8).expect("prev name"));
        let kept = std::fs::read(&prev_path).expect("prev exists");
        std::fs::write(current_path(&dir, 8), vec![0u8; MAX_TOTAL_BYTES + 16])
            .expect("plant oversize");
        save_generation(&dir, 8, 7, 72, b"gen-three").expect("save over oversize works");
        let mut events = ring(8);
        let loaded = load_slot(&dir, 8, 32, &mut events).expect("current loads");
        assert_eq!(loaded.envelope.payload, b"gen-three");
        assert_eq!(
            std::fs::read(&prev_path).expect("prev kept"),
            kept,
            "oversize never becomes the previous generation"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
