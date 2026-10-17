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
    /// Capture the world as versioned bytes: clocks, every id slot with its
    /// generation, every table row in row order with its stamps and spine
    /// columns, the free list and the staged kills. Sparse markers are
    /// transient presentation flags (never hashed) and are not captured.
    /// Boundary path: allocates the byte vector.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&WORLD_SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.last_scan.to_le_bytes());
        put_len(&mut out, self.locs.len());
        for l in &self.locs {
            out.extend_from_slice(&l.gen.to_le_bytes());
            out.push(u8::from(l.alive));
        }
        put_len(&mut out, self.tables.len());
        for (mask, t) in (0u8..).zip(&self.tables) {
            put_len(&mut out, t.entities.len());
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
                t.spine_row(mask, i).append_canonical(&mut out);
            }
        }
        put_len(&mut out, self.free.len());
        for id in &self.free {
            out.extend_from_slice(&id.to_le_bytes());
        }
        put_len(&mut out, self.pending.len());
        for e in &self.pending {
            out.extend_from_slice(&e.id.to_le_bytes());
            out.extend_from_slice(&e.gen.to_le_bytes());
        }
        out
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
