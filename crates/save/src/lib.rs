//! save: versioned envelope + 16 slots + atomic write. Payload hashes before seal, seal after.
//!
//! Storage and envelope only: the game owns its versioned progression payload
//! bytes (seed/tick/payload are opaque here) and `save` never imports game or
//! arena code. Generations are bounded ([`MAX_PAYLOAD_BYTES`] per payload,
//! [`MAX_TOTAL_BYTES`] per sealed file) and only [`SUPPORTED_VERSIONS`] open.

/// Own magic (4 bytes). Never reuse another engine's magic.
pub const MAGIC: [u8; 4] = *b"E204";
/// Current envelope version.
pub const VERSION: u8 = 1;
/// Slot count (0..16).
pub const SLOTS: u8 = 16;
/// Envelope versions this crate opens and migrates.
pub const SUPPORTED_VERSIONS: &[u8] = &[1];
/// Maximum opaque payload bytes per generation (1 MiB; game progression
/// state is kilobytes, so this cap keeps 16 slots bounded on disk).
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576;
/// Maximum sealed file bytes: 29 bytes of envelope (4 magic + 1 version +
/// 8 seed + 8 tick + 4 length + 4 CRC32) plus [`MAX_PAYLOAD_BYTES`].
pub const MAX_TOTAL_BYTES: usize = 29 + MAX_PAYLOAD_BYTES;

/// Previous-generation fallback with observe notices.
pub mod fallback;
/// Versioned save payload for spine bytes (ARC5).
pub mod persist;
/// Bounded spine-byte checkpoints for net resimulation (ARC5).
pub mod rollback;

/// Open failures (deterministic, no IO inside).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveErr {
    /// Magic mismatch.
    BadMagic,
    /// Version newer than we read / unknown.
    BadVersion,
    /// Truncated header or payload.
    Truncated,
    /// CRC32 mismatch (corrupt or tampered).
    CrcMismatch,
    /// Slot id out of range.
    BadSlot,
    /// Declared or actual size exceeds MAX_PAYLOAD_BYTES / MAX_TOTAL_BYTES.
    TooLarge,
}

/// Sealed envelope fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// Envelope version (opened only if <= VERSION).
    pub version: u8,
    /// Seed the payload was baked under.
    pub seed: u64,
    /// Tick the payload was baked at.
    pub tick: u64,
    /// Opaque payload bytes.
    pub payload: Vec<u8>,
}

/// Bitwise IEEE CRC32 (table-free, deterministic, std-only).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Length prefix (`u32` LE bytes) to `usize`, shared by the three envelope
/// decoders ([`verify`], [`persist::open_spine`], [`rollback::decode_frame`]).
///
/// Splits the word into two `u16` halves so the only conversions are
/// `usize::from(u16)` widenings: exact on every target, no `as` cast, no
/// truncation, no platform-dependent `From<u32>`. Decodes the full `u32`
/// range; callers still refuse lengths past their own caps. Units: bytes in,
/// count out. Determinism: pure byte math, no alloc.
/// Caller: [`verify`] (and through it the spine decoders, which call this
/// same helper instead of their own `as` cast).
pub fn len_usize(len_b: [u8; 4]) -> usize {
    let lo = u16::from_le_bytes([len_b[0], len_b[1]]);
    let hi = u16::from_le_bytes([len_b[2], len_b[3]]);
    usize::from(lo) | (usize::from(hi) << 16)
}

/// Seal: MAGIC(4) + ver(1) + seed LE(8) + tick LE(8) + len LE(4) + payload + crc LE(4).
///
/// Fallible: payloads over [`MAX_PAYLOAD_BYTES`] return
/// [`SaveErr::TooLarge`] before allocating, so a huge caller slice never
/// doubles into a second huge `Vec`. Units: bytes in and out. Errors:
/// `TooLarge` on oversize. Determinism: same inputs give same bytes.
pub fn seal(version: u8, seed: u64, tick: u64, payload: &[u8]) -> Result<Vec<u8>, SaveErr> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(SaveErr::TooLarge);
    }
    let mut out = Vec::with_capacity(29 + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(version);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    let len = u32::try_from(payload.len()).map_err(|_| SaveErr::TooLarge)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc32(payload).to_le_bytes());
    Ok(out)
}

/// Open + verify. Ok only on magic + version + length + crc all green.
///
/// Zero-copy order: the CRC runs over the borrowed payload slice before the
/// single payload `Vec` is allocated, so a corrupt file costs no payload
/// allocation (previously the copy happened first and was dropped on
/// mismatch). Happy path keeps exactly one payload copy plus one CRC pass.
pub fn open(bytes: &[u8]) -> Result<Envelope, SaveErr> {
    let (version, seed, tick, len) = verify(bytes)?;
    let payload = bytes[25..25 + len].to_vec();
    Ok(Envelope {
        version,
        seed,
        tick,
        payload,
    })
}

/// Verify magic + version + length + crc without allocating the payload.
///
/// Same gates as [`open`] (same errors in the same order) but returns the
/// header fields and payload length instead of an owned [`Envelope`], so
/// validity probes (e.g. the pre-rotation backup check) never heap a
/// throwaway payload copy. Units: bytes in; header fields out.
pub fn verify(bytes: &[u8]) -> Result<(u8, u64, u64, usize), SaveErr> {
    if bytes.len() < 29 {
        return Err(SaveErr::Truncated);
    }
    if bytes[0..4] != MAGIC {
        return Err(SaveErr::BadMagic);
    }
    let version = bytes[4];
    if version > VERSION || !SUPPORTED_VERSIONS.contains(&version) {
        return Err(SaveErr::BadVersion);
    }
    let mut seed_b = [0u8; 8];
    let mut tick_b = [0u8; 8];
    let mut len_b = [0u8; 4];
    seed_b.copy_from_slice(&bytes[5..13]);
    tick_b.copy_from_slice(&bytes[13..21]);
    len_b.copy_from_slice(&bytes[21..25]);
    let len = len_usize(len_b);
    if len > MAX_PAYLOAD_BYTES || bytes.len() > MAX_TOTAL_BYTES {
        return Err(SaveErr::TooLarge);
    }
    if bytes.len() != 29 + len {
        return Err(SaveErr::Truncated);
    }
    let mut crc_b = [0u8; 4];
    crc_b.copy_from_slice(&bytes[25 + len..29 + len]);
    // CRC over the borrowed slice: no allocation on either outcome.
    if u32::from_le_bytes(crc_b) != crc32(&bytes[25..25 + len]) {
        return Err(SaveErr::CrcMismatch);
    }
    Ok((
        version,
        u64::from_le_bytes(seed_b),
        u64::from_le_bytes(tick_b),
        len,
    ))
}

/// Migrate an opened envelope to current version (v1: passthrough).
/// Rejects any version outside SUPPORTED_VERSIONS.
pub fn migrate(env: &Envelope) -> Result<Envelope, SaveErr> {
    migrate_owned(env.clone())
}

/// Migrate an owned envelope without cloning the payload (zero-copy path).
///
/// Same result as [`migrate`], but the caller moves the envelope in and gets
/// it back, so no second payload allocation exists. Prefer this on save/load
/// paths; [`migrate`] stays for borrowed call sites.
pub fn migrate_owned(env: Envelope) -> Result<Envelope, SaveErr> {
    if !SUPPORTED_VERSIONS.contains(&env.version) {
        return Err(SaveErr::BadVersion);
    }
    Ok(env)
}

/// Slot file name (`save03.e2040`). Err on slot >= SLOTS.
pub fn slot_name(slot: u8) -> Result<String, SaveErr> {
    if slot >= SLOTS {
        return Err(SaveErr::BadSlot);
    }
    Ok(format!("save{slot:02}.e2040"))
}

/// Atomic write: tmp synced to disk, then rename, then the directory synced
/// (a crash never leaves half a save). Rejects oversize buffers and bad slots
/// before touching the target file, so a failed write leaves the prior valid
/// generation intact.
pub fn write_atomic(dir: &std::path::Path, slot: u8, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_TOTAL_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "save exceeds MAX_TOTAL_BYTES",
        ));
    }
    let name = slot_name(slot)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad slot"))?;
    let tmp = dir.join(format!("{name}.tmp"));
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // Flush before the rename publishes the bytes. The handle closes at
        // the end of this block, which Windows needs before the rename.
        file.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(name))?;
    sync_dir(dir);
    Ok(())
}

/// Flushes the directory entry that a rename just created. Best effort:
/// Windows refuses to open a directory for sync, and a save whose file is
/// already in place must not fail over it, so every error is ignored here.
fn sync_dir(dir: &std::path::Path) {
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Seal one game generation (current VERSION + seed + tick + opaque payload)
/// and atomically store it in `slot`. The prior valid current file rotates
/// to `saveNN.e2040.prev` first (see [`fallback`]), so a later corrupt read
/// falls back one generation; payloads over MAX_PAYLOAD_BYTES and backup
/// failures are rejected before touching the current file, leaving the
/// prior valid generation intact.
pub fn save_generation(
    dir: &std::path::Path,
    slot: u8,
    seed: u64,
    tick: u64,
    payload: &[u8],
) -> std::io::Result<()> {
    // Seal first: an oversize payload is refused before the backup rotates.
    let bytes = seal(VERSION, seed, tick, payload).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "payload exceeds MAX_PAYLOAD_BYTES",
        )
    })?;
    fallback::backup_current(dir, slot)?;
    write_atomic(dir, slot, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Length prefixes decode exactly over the full u32 range. Ratchet pin:
    /// the three envelope decoders (`verify`, `open_spine`, `decode_frame`)
    /// share one `From`-based conversion, so no `as` cast may reappear on
    /// their decode paths.
    #[test]
    fn len_prefix_decodes_full_u32_range() {
        assert_eq!(len_usize([0, 0, 0, 0]), 0);
        assert_eq!(len_usize(29u32.to_le_bytes()), 29);
        assert_eq!(
            len_usize(u32::MAX.to_le_bytes()),
            usize::try_from(u64::from(u32::MAX)).expect("u32 fits in usize on 32/64-bit"),
        );
    }

    #[test]
    fn seal_open_roundtrip_x2() {
        let (a, b) = (
            seal(1, 2040, 600, b"payload").expect("valid payload seals"),
            seal(1, 2040, 600, b"payload").expect("valid payload seals"),
        );
        assert_eq!(a, b);
        let env = open(&a).expect("roundtrip opens");
        assert_eq!((env.version, env.seed, env.tick), (1, 2040, 600));
        assert_eq!(env.payload, b"payload");
        assert_eq!(migrate(&env).unwrap(), env);
    }

    #[test]
    fn tamper_fails_crc() {
        let mut bytes = seal(1, 7, 8, b"abc").expect("valid payload seals");
        let last_payload = 25 + 3 - 1;
        bytes[last_payload] ^= 0xFF;
        assert_eq!(open(&bytes), Err(SaveErr::CrcMismatch));
    }

    #[test]
    fn bad_magic_version_truncated() {
        assert_eq!(open(b""), Err(SaveErr::Truncated));
        let mut bad = seal(1, 0, 0, b"x").expect("valid payload seals");
        bad[0] = b'Z';
        assert_eq!(open(&bad), Err(SaveErr::BadMagic));
        let future = seal(VERSION + 1, 0, 0, b"x").expect("valid payload seals");
        assert_eq!(open(&future), Err(SaveErr::BadVersion));
        assert_eq!(
            open(&seal(1, 0, 0, b"xy").expect("valid payload seals")[..30]),
            Err(SaveErr::Truncated)
        );
    }

    #[test]
    fn slots_bounds() {
        assert_eq!(slot_name(0).unwrap(), "save00.e2040");
        assert_eq!(slot_name(15).unwrap(), "save15.e2040");
        assert_eq!(slot_name(16), Err(SaveErr::BadSlot));
    }

    #[test]
    fn atomic_roundtrip_tmp() {
        // Per-process dir: parallel lanes run this suite at the same time.
        let dir = std::env::temp_dir().join(format!("e2040-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = seal(1, 2040, 600, b"hi").expect("valid payload seals");
        write_atomic(&dir, 3, &bytes).unwrap();
        let back = std::fs::read(dir.join("save03.e2040")).unwrap();
        assert_eq!(open(&back).unwrap().payload, b"hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn sv1_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-save-sv1-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_slot(dir: &std::path::Path, slot: u8) -> Envelope {
        let back = std::fs::read(dir.join(slot_name(slot).unwrap())).unwrap();
        open(&back).expect("slot holds a valid generation")
    }

    #[test]
    fn bounds_and_versions_are_explicit() {
        assert_eq!(MAX_TOTAL_BYTES, 29 + MAX_PAYLOAD_BYTES);
        assert!(SUPPORTED_VERSIONS.contains(&VERSION));
        // v0 predates the supported list: opens nothing, migrates nothing.
        assert_eq!(
            open(&seal(0, 0, 0, b"x").expect("valid payload seals")),
            Err(SaveErr::BadVersion)
        );
        assert_eq!(
            migrate(&Envelope {
                version: 0,
                seed: 0,
                tick: 0,
                payload: vec![],
            }),
            Err(SaveErr::BadVersion)
        );
    }

    #[test]
    fn save_generation_roundtrip() {
        let dir = sv1_dir("roundtrip");
        save_generation(&dir, 0, 99, 1234, b"gen-payload").unwrap();
        let env = read_slot(&dir, 0);
        assert_eq!((env.version, env.seed, env.tick), (VERSION, 99, 1234));
        assert_eq!(env.payload, b"gen-payload");
        assert_eq!(migrate(&env).unwrap(), env);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_generation_reloads_identical_bytes() {
        let dir = sv1_dir("durable-bytes");
        save_generation(&dir, 4, 5, 60, b"first").unwrap();
        save_generation(&dir, 4, 5, 61, b"second").unwrap();
        // The current file holds exactly the sealed second generation ...
        let current = std::fs::read(dir.join(slot_name(4).unwrap())).unwrap();
        assert_eq!(current, seal(VERSION, 5, 61, b"second").unwrap());
        // ... and the rotated backup holds exactly the sealed first one.
        let prev = std::fs::read(dir.join(fallback::prev_name(4).unwrap())).unwrap();
        assert_eq!(prev, seal(VERSION, 5, 60, b"first").unwrap());
        // Reloading through the open path returns the second payload.
        let env = read_slot(&dir, 4);
        assert_eq!((env.seed, env.tick), (5, 61));
        assert_eq!(env.payload, b"second");
        // The tmp files are renamed away, never left behind.
        assert!(!dir.join("save04.e2040.tmp").exists());
        assert!(!dir.join("save04.e2040.prev.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversize_payload_rejected_without_panic() {
        let big = vec![0xABu8; MAX_PAYLOAD_BYTES + 1];
        // Seal refuses before allocating the output Vec, so no second
        // 1 MiB copy exists; `open` never sees the envelope ...
        assert_eq!(seal(VERSION, 1, 2, &big), Err(SaveErr::TooLarge));
        // ... raw oversize buffers are refused before touching disk ...
        let dir = sv1_dir("oversize");
        let raw = vec![0u8; MAX_TOTAL_BYTES + 1];
        let err = write_atomic(&dir, 1, &raw).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.join("save01.e2040").exists());
        // ... and the generation API rejects the payload itself.
        let err = save_generation(&dir, 1, 1, 2, &big).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.join("save01.e2040").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversize_save_leaves_both_generations_intact() {
        let dir = sv1_dir("oversize-keeps");
        save_generation(&dir, 2, 1, 10, b"gen-one").unwrap();
        save_generation(&dir, 2, 1, 20, b"gen-two").unwrap();
        let prev = dir.join(fallback::prev_name(2).unwrap());
        let (cur_before, prev_before) = (
            std::fs::read(dir.join("save02.e2040")).unwrap(),
            std::fs::read(&prev).unwrap(),
        );
        let big = vec![0u8; MAX_PAYLOAD_BYTES + 1];
        assert!(save_generation(&dir, 2, 1, 30, &big).is_err());
        assert_eq!(std::fs::read(dir.join("save02.e2040")).unwrap(), cur_before);
        assert_eq!(
            std::fs::read(&prev).unwrap(),
            prev_before,
            "a refused save never rotates the backup"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn seal_boundary_and_empty() {
        assert!(seal(VERSION, 1, 2, &[]).is_ok());
        let exact = vec![0x5Au8; MAX_PAYLOAD_BYTES];
        let sealed = seal(VERSION, 9, 10, &exact).expect("exact cap seals");
        assert_eq!(sealed.len(), MAX_TOTAL_BYTES);
        assert_eq!(open(&sealed).expect("exact cap opens").payload, exact);
        let over = vec![0x5Au8; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(seal(VERSION, 9, 10, &over), Err(SaveErr::TooLarge));
    }

    #[test]
    fn repeat_overwrite_preserves_latest_valid() {
        let dir = sv1_dir("overwrite");
        save_generation(&dir, 5, 1, 10, b"gen-one").unwrap();
        save_generation(&dir, 5, 1, 20, b"gen-two").unwrap();
        let env = read_slot(&dir, 5);
        assert_eq!(env.tick, 20);
        assert_eq!(env.payload, b"gen-two");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_write_preserves_prior_valid_generation() {
        let dir = sv1_dir("recover");
        save_generation(&dir, 2, 7, 70, b"prior-good").unwrap();
        let before = read_slot(&dir, 2);

        // Oversize payload: rejected before the filesystem is touched.
        let big = vec![0xCDu8; MAX_PAYLOAD_BYTES + 1];
        let err = save_generation(&dir, 2, 7, 71, &big).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(read_slot(&dir, 2), before);

        // IO error (missing directory): target generation untouched.
        let missing = dir.join("no-such-dir");
        let err = save_generation(&missing, 2, 7, 72, b"lost-write").unwrap_err();
        assert_ne!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(read_slot(&dir, 2), before);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Published envelope pins the sealed layout: the JSON forge reads holds
    /// the same version, slot count and byte caps the seal enforces, so a
    /// moved constant or a stale file fails loudly.
    #[test]
    fn envelope_drift() {
        assert_eq!(VERSION, 1);
        assert_eq!(SLOTS, 16);
        assert_eq!(MAX_PAYLOAD_BYTES, 1_048_576);
        assert_eq!(MAX_TOTAL_BYTES, 29 + MAX_PAYLOAD_BYTES);
        assert_eq!(MAX_TOTAL_BYTES, 1_048_605);
        let text = include_str!("../../../content/arena/save-envelope.json");
        assert_eq!(
            json_u64(text, "\"version\""),
            u64::from(VERSION),
            "published version tracks the seal"
        );
        assert_eq!(
            json_u64(text, "\"slots\""),
            u64::from(SLOTS),
            "published slot count tracks the seal"
        );
        assert_eq!(
            json_u64(text, "\"max_payload_bytes\""),
            u64::try_from(MAX_PAYLOAD_BYTES).expect("payload cap fits in u64"),
            "published payload cap tracks the seal"
        );
        assert_eq!(
            json_u64(text, "\"max_total_bytes\""),
            u64::try_from(MAX_TOTAL_BYTES).expect("total cap fits in u64"),
            "published total tracks header plus payload"
        );
        assert_eq!(slot_name(SLOTS), Err(SaveErr::BadSlot));
        assert!(slot_name(SLOTS - 1).is_ok());
    }

    /// Integer after `"key":` in the published envelope JSON (std-only, no
    /// new dependency for one small file).
    fn json_u64(text: &str, needle: &str) -> u64 {
        let at = text.find(needle).expect("envelope json holds key");
        let rest = text
            .get(at.saturating_add(needle.len())..)
            .expect("key has tail");
        let colon = rest.find(':').expect("key has colon");
        let value = rest
            .get(colon.saturating_add(1)..)
            .expect("colon has value");
        let start = value
            .find(|c: char| c.is_ascii_digit())
            .expect("value holds digits");
        let digits = value.get(start..).expect("digits have tail");
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        digits
            .get(..end)
            .expect("digits slice")
            .parse()
            .expect("digits parse")
    }
}
