//! world_snapshot: byte-exact snapshot and restore of the archetype [`World`].
//!
//! Owner: engine-ecs. Child module of `archetype` (reads its private tables).
//! Format: version, clocks, every id slot with its generation, each table's
//! rows in row order with change stamps and spine columns, the free list and
//! the staged kills, all little-endian. Sparse markers are not captured.

use super::*;

/// World snapshot format version. Bump only with a hash-law receipt.
pub const WORLD_SNAPSHOT_VERSION: u16 = 1;

/// Most tables a world holds: one per mask, masks below 128.
const MAX_TABLES: usize = 128;

/// [`World::restore`] failure. Loud `Err`, never panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldSnapshotError {
    /// Fewer bytes than a header, count or field needs.
    TooShort,
    /// Unknown version (holds the found version).
    BadVersion(u16),
    /// A flag byte was not 0 or 1 (holds the found byte).
    BadFlag(u8),
    /// Ids, rows or lists disagree with each other.
    BadLink,
    /// Bytes left over after a full decode (holds the count).
    Trailing(usize),
}

impl core::fmt::Display for WorldSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "world snapshot bytes too short"),
            Self::BadVersion(v) => write!(f, "world snapshot bad version {v}"),
            Self::BadFlag(b) => write!(f, "world snapshot bad flag {b}"),
            Self::BadLink => write!(f, "world snapshot ids, rows or lists disagree"),
            Self::Trailing(n) => write!(f, "world snapshot trailing {n} bytes"),
        }
    }
}

impl std::error::Error for WorldSnapshotError {}

/// Bounds-checked little-endian reader over snapshot bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], WorldSnapshotError> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or(WorldSnapshotError::TooShort)?;
        let chunk = self
            .bytes
            .get(self.pos..end)
            .ok_or(WorldSnapshotError::TooShort)?;
        self.pos = end;
        let mut out = [0u8; N];
        out.copy_from_slice(chunk);
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, WorldSnapshotError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, WorldSnapshotError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, WorldSnapshotError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, WorldSnapshotError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn fixed(&mut self) -> Result<Fixed, WorldSnapshotError> {
        Ok(Fixed::from_le_bytes(self.take()?))
    }
    fn flag(&mut self) -> Result<bool, WorldSnapshotError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(WorldSnapshotError::BadFlag(other)),
        }
    }
    /// A `u32` item count whose items each need at least `min_bytes` more
    /// bytes, so a lying count cannot ask for a huge allocation.
    fn count(&mut self, min_bytes: usize) -> Result<usize, WorldSnapshotError> {
        let n = usize::try_from(self.u32()?).map_err(|_| WorldSnapshotError::TooShort)?;
        let need = n
            .checked_mul(min_bytes)
            .ok_or(WorldSnapshotError::TooShort)?;
        if self.bytes.len() - self.pos < need {
            return Err(WorldSnapshotError::TooShort);
        }
        Ok(n)
    }
    /// The spine columns of `mask`, in bit order.
    fn spine(&mut self, mask: u8) -> Result<SpineRow, WorldSnapshotError> {
        let mut row = SpineRow::default();
        if has_bit(mask, TRANSFORM_BIT) {
            row.transform = Some(FixedTransform {
                pos: [self.fixed()?, self.fixed()?, self.fixed()?],
                yaw: self.fixed()?,
            });
        }
        if has_bit(mask, BODY_BIT) {
            row.body = Some(Body {
                vel: [self.fixed()?, self.fixed()?],
                radius: self.fixed()?,
                height: self.fixed()?,
            });
        }
        if has_bit(mask, VITALS_BIT) {
            row.vitals = Some(Vitals {
                hp_permille: self.u16()?,
                stamina: self.u16()?,
                poise: self.u16()?,
                state: self.u8()?,
            });
        }
        if has_bit(mask, AI_BIT) {
            row.ai = Some(AiState {
                intent: Intent {
                    mx: i8::from_le_bytes(self.take()?),
                    mz: i8::from_le_bytes(self.take()?),
                    facing: i16::from_le_bytes(self.take()?),
                    buttons: self.u8()?,
                },
                clock: self.u32()?,
                source: self.u8()?,
            });
        }
        if has_bit(mask, ANIM_BIT) {
            row.anim = Some(FixedAnim {
                loco: self.u8()?,
                phase: self.fixed()?,
                facing: [self.fixed()?, self.fixed()?],
                airborne: self.flag()?,
                interrupt: self.flag()?,
                pose_ref: self.u32()?,
            });
        }
        Ok(row)
    }
}

/// Append a count as `u32` LE (a world never holds 2^32 of anything).
fn put_len(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
}

impl World {
    /// Exact byte length [`World::snapshot`] writes for this world: the
    /// header, one `4`-byte count plus `5` bytes per id slot, one `4`-byte
    /// count per table plus a `4`-byte entity id and the present columns of
    /// every row, then the free list and the staged kills. The writer appends
    /// fixed-width fields only, so the sum is exact and reserving it makes the
    /// boundary path one allocation. The bytes themselves never depend on it.
    /// Budget: O(tables) time, zero allocation.
    fn snapshot_len(&self) -> usize {
        let mut len = 2 + 4 + 4 + 4;
        len += 5 * self.locs.len();
        len += 4;
        for (mask, t) in (0u8..).zip(&self.tables) {
            let mut row = 4;
            if has_bit(mask, P) {
                row += 12;
            }
            if has_bit(mask, V) {
                row += 12;
            }
            row += SpineRow::canonical_len(mask);
            len += 4 + t.entities.len() * row;
        }
        len += 4 + 4 * self.free.len();
        len += 4 + 8 * self.pending.len();
        len
    }

    /// Capture the world as versioned bytes: clocks, every id slot with its
    /// generation, every table row in row order with its stamps and spine
    /// columns, the free list and the staged kills. Sparse markers are
    /// transient presentation flags (never hashed) and are not captured.
    /// Boundary path: allocates the byte vector once, at exactly
    /// [`World::snapshot_len`] bytes.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.snapshot_len());
        self.append_snapshot(&mut out);
        out
    }

    /// Append the bytes [`World::snapshot`] promises to `out`: the same field
    /// sequence whether `out` was reserved or empty, so the capacity hint
    /// never reaches a byte.
    fn append_snapshot(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&WORLD_SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.last_scan.to_le_bytes());
        put_len(out, self.locs.len());
        for l in &self.locs {
            out.extend_from_slice(&l.gen.to_le_bytes());
            out.push(u8::from(l.alive));
        }
        put_len(out, self.tables.len());
        for (mask, t) in (0u8..).zip(&self.tables) {
            put_len(out, t.entities.len());
            for (i, e) in t.entities.iter().enumerate() {
                out.extend_from_slice(&e.id.to_le_bytes());
                if has_bit(mask, P) {
                    out.extend_from_slice(&t.pos[i].x.to_bits().to_le_bytes());
                    out.extend_from_slice(&t.pos[i].y.to_bits().to_le_bytes());
                    out.extend_from_slice(&t.pos_changed[i].to_le_bytes());
                }
                if has_bit(mask, V) {
                    out.extend_from_slice(&t.vel[i].x.to_bits().to_le_bytes());
                    out.extend_from_slice(&t.vel[i].y.to_bits().to_le_bytes());
                    out.extend_from_slice(&t.vel_changed[i].to_le_bytes());
                }
                t.spine_row(mask, i).append_canonical(out);
            }
        }
        put_len(out, self.free.len());
        for id in &self.free {
            out.extend_from_slice(&id.to_le_bytes());
        }
        put_len(out, self.pending.len());
        for e in &self.pending {
            out.extend_from_slice(&e.id.to_le_bytes());
            out.extend_from_slice(&e.gen.to_le_bytes());
        }
    }

    /// Rebuild the exact world [`World::snapshot`] captured: same ids,
    /// generations, row order, stamps, free-list order and staged kills, so
    /// the next spawn and every iteration match the original. Sparse markers
    /// start empty. Errors: [`WorldSnapshotError`] for short, mislabeled or
    /// inconsistent bytes; nothing is returned half built.
    pub fn restore(bytes: &[u8]) -> Result<Self, WorldSnapshotError> {
        let mut c = Cursor { bytes, pos: 0 };
        let version = c.u16()?;
        if version != WORLD_SNAPSHOT_VERSION {
            return Err(WorldSnapshotError::BadVersion(version));
        }
        let mut w = Self::new();
        w.tick = c.u32()?;
        w.last_scan = c.u32()?;
        let slots = c.count(5)?;
        w.locs.reserve_exact(slots);
        for _ in 0..slots {
            let gen = c.u32()?;
            let alive = c.flag()?;
            w.locs.push(Loc {
                mask: 0,
                row: 0,
                gen,
                alive,
            });
        }
        let table_count = c.count(4)?;
        if table_count > MAX_TABLES {
            return Err(WorldSnapshotError::BadLink);
        }
        let mut placed = vec![false; slots];
        for mask in (0u8..).take(table_count) {
            w.ensure_table(mask);
            for _ in 0..c.count(4)? {
                let id = c.u32()?;
                let idx = usize::try_from(id).map_err(|_| WorldSnapshotError::BadLink)?;
                let gen = match w.locs.get(idx) {
                    Some(l) if l.alive && !placed[idx] => l.gen,
                    _ => return Err(WorldSnapshotError::BadLink),
                };
                let p = if has_bit(mask, P) {
                    let pos = Position {
                        x: c.f32()?,
                        y: c.f32()?,
                    };
                    (Some(pos), c.u32()?)
                } else {
                    (None, 0)
                };
                let v = if has_bit(mask, V) {
                    let vel = Velocity {
                        x: c.f32()?,
                        y: c.f32()?,
                    };
                    (Some(vel), c.u32()?)
                } else {
                    (None, 0)
                };
                let spine = c.spine(mask)?;
                let row = w.push_row(mask, Entity { id, gen }, p, v, &spine);
                w.locs[idx].mask = mask;
                w.locs[idx].row = row;
                placed[idx] = true;
            }
        }
        if w.locs.iter().zip(&placed).any(|(l, done)| l.alive && !done) {
            return Err(WorldSnapshotError::BadLink);
        }
        let dead = w.locs.iter().filter(|l| !l.alive).count();
        let free = c.count(4)?;
        if free != dead {
            return Err(WorldSnapshotError::BadLink);
        }
        let mut listed = vec![false; slots];
        for _ in 0..free {
            let id = c.u32()?;
            let idx = usize::try_from(id).map_err(|_| WorldSnapshotError::BadLink)?;
            if !matches!(w.locs.get(idx), Some(l) if !l.alive) || listed[idx] {
                return Err(WorldSnapshotError::BadLink);
            }
            listed[idx] = true;
            w.free.push(id);
        }
        for _ in 0..c.count(8)? {
            let (id, gen) = (c.u32()?, c.u32()?);
            let idx = usize::try_from(id).map_err(|_| WorldSnapshotError::BadLink)?;
            if idx >= slots {
                return Err(WorldSnapshotError::BadLink);
            }
            w.pending.push(Entity { id, gen });
        }
        if c.pos != bytes.len() {
            return Err(WorldSnapshotError::Trailing(bytes.len() - c.pos));
        }
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_core::checked::{f32_from_u64, f64_from_u64};

    /// Timing fixture size: 4,000 P+V slots, a realistic net checkpoint.
    const BENCH_ROWS: usize = 4_000;
    /// Snapshots timed per run; the p50 of these is the receipt.
    const BENCH_ITERS: usize = 200;
    /// Rows of the frozen-byte oracle fixture.
    const ORACLE_ROWS: usize = 64;
    /// Whole byte stream of the oracle fixture, written before the reserve
    /// change; the capacity hint must not move one byte of it.
    const SNAPSHOT_ORACLE: u64 = 6_287_169_743_782_133_372;

    /// `i` as an `f32` through the checked helper, so this fixture adds no cast.
    fn coord(i: usize) -> f32 {
        f32_from_u64(u64::try_from(i).unwrap_or(0))
    }

    /// `ns` in microseconds for the receipt line (no cast, no float method).
    fn micros(ns: u128) -> f64 {
        f64_from_u64(u64::try_from(ns).unwrap_or(0)) / 1000.0
    }

    /// How many times slower `slow` is than `fast`, as a receipt figure.
    fn ratio(slow: u128, fast: u128) -> f64 {
        if fast == 0 {
            return 0.0;
        }
        f64_from_u64(u64::try_from(slow).unwrap_or(0))
            / f64_from_u64(u64::try_from(fast).unwrap_or(0))
    }

    /// One spine row with every column set from `k`.
    fn spine_row(k: i64) -> SpineRow {
        SpineRow {
            transform: Some(FixedTransform {
                pos: [Fixed::from_int(k); 3],
                yaw: Fixed::from_int(k),
            }),
            body: Some(Body {
                vel: [Fixed::ZERO; 2],
                radius: Fixed::from_bits(26_214),
                height: Fixed::from_bits(117_965),
            }),
            vitals: Some(Vitals {
                hp_permille: 1000,
                stamina: 100,
                poise: u16::try_from(k).unwrap_or(0),
                state: Vitals::ATTACKING,
            }),
            ai: Some(AiState {
                intent: Intent {
                    mx: i8::try_from(k).unwrap_or(0),
                    mz: -1,
                    facing: 900,
                    buttons: 1,
                },
                clock: u32::try_from(k).unwrap_or(0),
                source: AiState::FROM_BRAIN,
            }),
            anim: Some(FixedAnim {
                loco: 1,
                phase: Fixed::from_bits(k * 11),
                facing: [Fixed::ZERO; 2],
                airborne: false,
                interrupt: false,
                pose_ref: u32::try_from(k).unwrap_or(0),
            }),
        }
    }

    /// `rows` P+V slots with every 7th despawned (free list) and every 11th
    /// staged for kill (pending list), plus a full and a transform-only spine
    /// row, so the bytes cover every section and every column kind.
    fn seeded(rows: usize) -> World {
        let mut w = World::new();
        w.reserve(rows + 8);
        for i in 0..rows {
            let f = coord(i);
            let e = w.spawn(
                Some(Position { x: f * 0.25, y: -f }),
                Some(Velocity { x: 0.5, y: -0.25 }),
            );
            if i % 7 == 3 {
                assert!(w.despawn(e), "row {i} despawns");
            } else if i % 11 == 5 {
                assert!(w.kill(e), "row {i} stages");
            }
        }
        w.spawn_with(None, None, spine_row(7));
        w.spawn_with(
            Some(Position { x: 1.5, y: -1.5 }),
            None,
            SpineRow {
                transform: Some(FixedTransform {
                    pos: [Fixed::from_int(2); 3],
                    yaw: Fixed::from_int(1),
                }),
                ..SpineRow::default()
            },
        );
        w
    }

    /// The frozen byte stream still holds and the whole write is one
    /// allocation, so the reserve covers every byte the writer emits.
    #[test]
    fn world_snapshot_bytes_match_the_frozen_oracle() {
        let w = seeded(ORACLE_ROWS);
        let (_, heap) = measure(|| std::hint::black_box(w.snapshot()));
        assert_eq!(heap.calls, 1, "one allocation covers the whole write");
        let bytes = w.snapshot();
        assert_eq!(
            hash_bytes(&bytes),
            SNAPSHOT_ORACLE,
            "the snapshot byte stream moved"
        );
        let back = World::restore(&bytes).expect("the fixture decodes");
        assert_eq!(back.snapshot(), bytes, "re-snapshot is byte-identical");
        // Not vacuous: one moved position moves the bytes.
        let mut moved = seeded(ORACLE_ROWS);
        let mut target = None;
        moved.for_each_pv(|e, _, _| target = target.or(Some(e)));
        let e = target.expect("the fixture holds P+V rows");
        assert!(moved.set_pos(e, Position { x: -1.0, y: 1.0 }));
        assert_ne!(
            hash_bytes(&moved.snapshot()),
            SNAPSHOT_ORACLE,
            "moved state must move the bytes"
        );
    }

    /// Snapshot write receipt: p50 microseconds for one `World::snapshot` of a
    /// 4,000-row world against the same write into an unreserved vector, on
    /// interleaved rounds so the same machine load sits under both sides.
    /// Run: cargo test -p engine-ecs --lib --release
    /// world_snapshot_bench_receipt -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn world_snapshot_bench_receipt() {
        let w = seeded(BENCH_ROWS);
        // Old path (no capacity promise) against the new path, one fixture:
        // the reserve must not move a byte of the stream.
        let mut unreserved = Vec::new();
        w.append_snapshot(&mut unreserved);
        assert_eq!(unreserved, w.snapshot(), "the reserve moved bytes");
        assert!(
            unreserved.len() > BENCH_ROWS * 20,
            "the fixture writes a real byte stream, got {}",
            unreserved.len()
        );
        let mut old_runs = Vec::with_capacity(BENCH_ITERS);
        let mut new_runs = Vec::with_capacity(BENCH_ITERS);
        for _ in 0..BENCH_ITERS {
            let mut out = Vec::new();
            let t = std::time::Instant::now();
            w.append_snapshot(&mut out);
            old_runs.push(t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            let fresh = w.snapshot();
            new_runs.push(t.elapsed().as_nanos());
            std::hint::black_box((out.len(), fresh.len()));
        }
        old_runs.sort_unstable();
        new_runs.sort_unstable();
        let (old, new) = (old_runs[BENCH_ITERS / 2], new_runs[BENCH_ITERS / 2]);
        eprintln!(
            "world_snapshot_p50_us={:.3} unreserved_p50_us={:.3} speedup={:.2}x rows={BENCH_ROWS} bytes={}",
            micros(new),
            micros(old),
            ratio(old, new),
            unreserved.len()
        );
        assert!(new < old, "the reserve must not be the slower write");
    }
}
