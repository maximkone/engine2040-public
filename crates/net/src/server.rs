//! server: headless authoritative match tick + quantized snapshots + grid AOI + bandwidth budget.
use engine_core::Fnv64;

/// Fixed server tick rate in Hz (the arena tick via [`crate::scale::TICK_HZ`]).
pub use crate::scale::TICK_HZ;
/// Server connection capacity: transport slots per host (chaos gate).
/// Distinct from match slots (`engine_input::MAX_PARTICIPANTS = 16`):
/// one host can carry several matches plus spectators/AI legs up to this
/// transport bound, while one match fields `2..=MAX_PARTICIPANTS` fighters.
pub const MAX_PLAYERS: usize = 64;
/// Bytes per entity entry (id 4 + xq 2 + yq 2).
pub const BYTES_PER_ENT: usize = 8;
/// Bytes per snapshot header (tick 4 + viewer 2 + count 2).
pub const HEADER_BYTES: usize = 8;
/// Default per-client per-tick budget (63 ents max).
pub const DEFAULT_BUDGET: usize = 512;
/// Default AOI cell size in cm.
pub const DEFAULT_CELL_CM: i32 = 1600;
/// Default AOI view radius in cm.
pub const DEFAULT_VIEW_CM: i32 = 4800;
/// Max authoritative step per tick in cm (~9.6 m/s).
pub const MAX_STEP_CM: i32 = 16;
/// Square world extent in cm.
pub const WORLD_CM: i32 = 25600;
/// Match config.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Deterministic seed.
    pub seed: u64,
    /// Per-client per-tick byte budget.
    pub budget: usize,
    /// AOI cell size in cm.
    pub cell: i32,
    /// AOI view radius in cm.
    pub view: i32,
    /// World extent in cm.
    pub world: i32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 2040,
            budget: DEFAULT_BUDGET,
            cell: DEFAULT_CELL_CM,
            view: DEFAULT_VIEW_CM,
            world: WORLD_CM,
        }
    }
}
impl Config {
    /// Config with seed.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }
}
/// Quantized entity entry in cm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapEnt {
    /// Entity id.
    pub id: u32,
    /// X in cm.
    pub x: i32,
    /// Y in cm.
    pub y: i32,
}
/// Per-viewer snapshot.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Tick id.
    pub tick: u64,
    /// Viewer id.
    pub viewer: u32,
    /// Visible ents, sorted by (dist2,id).
    pub ents: Vec<SnapEnt>,
}
impl Snapshot {
    /// Wire size in bytes.
    pub fn byte_len(&self) -> usize {
        HEADER_BYTES + self.ents.len() * BYTES_PER_ENT
    }
}
/// Headless authoritative server. SoA slots, id==slot.
pub struct Server {
    cfg: Config,
    tick: u64,
    hash: u64,
    rng: u64,
    live: bool,
    next_spawn: u32,
    alive: [bool; MAX_PLAYERS],
    xs: [i32; MAX_PLAYERS],
    ys: [i32; MAX_PLAYERS],
    pend: [u32; MAX_PLAYERS],
    cells: Vec<Vec<u32>>,
    gw: i32,
    gh: i32,
}
impl Server {
    /// Create headless match (agones Allocated+Ready).
    pub fn new(cfg: Config) -> Self {
        let gw = ((cfg.world + cfg.cell - 1) / cfg.cell.max(1)).max(1);
        let n = (gw * gw) as usize;
        let mut s = Self {
            cfg,
            tick: 0,
            hash: Fnv64::new().finish(),
            rng: cfg.seed | 1,
            live: true,
            next_spawn: 0,
            alive: [false; MAX_PLAYERS],
            xs: [0; MAX_PLAYERS],
            ys: [0; MAX_PLAYERS],
            pend: [0; MAX_PLAYERS],
            cells: vec![Vec::new(); n],
            gw,
            gh: gw,
        };
        s.rehash();
        s
    }
    /// Create with seed.
    pub fn with_seed(seed: u64) -> Self {
        Self::new(Config::with_seed(seed))
    }
    /// Tick id.
    pub fn tick_id(&self) -> u64 {
        self.tick
    }
    /// Authoritative state hash.
    pub fn state_hash(&self) -> u64 {
        self.hash
    }
    /// Alias for state_hash.
    pub fn hash(&self) -> u64 {
        self.hash
    }
    /// Headless liveness (agones Ready).
    pub fn is_live(&self) -> bool {
        self.live
    }
    /// Health: live and within cap.
    pub fn health(&self) -> bool {
        self.live && self.count() <= MAX_PLAYERS
    }
    /// Shut down match (agones Shutdown).
    pub fn shutdown(&mut self) {
        self.live = false;
    }
    /// Alive player count.
    pub fn count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }
    /// Join first free slot (nakama presence). None when full/shut.
    pub fn join(&mut self) -> Option<u32> {
        if !self.live {
            return None;
        }
        for i in 0..MAX_PLAYERS {
            if !self.alive[i] {
                self.alive[i] = true;
                self.pend[i] = 0;
                let w = self.cfg.world;
                self.xs[i] = (self.next_rand() % w as u64) as i32;
                self.ys[i] = (self.next_rand() % w as u64) as i32;
                self.next_spawn += 1;
                self.rebuild();
                self.rehash();
                return Some(i as u32);
            }
        }
        None
    }
    /// Alias for join.
    pub fn add_player(&mut self) -> Option<u32> {
        self.join()
    }
    /// Leave slot.
    pub fn leave(&mut self, id: u32) -> bool {
        let i = id as usize;
        if i < MAX_PLAYERS && self.alive[i] {
            self.alive[i] = false;
            self.pend[i] = 0;
            self.rebuild();
            self.rehash();
            return true;
        }
        false
    }
    /// Alias for leave.
    pub fn remove_player(&mut self, id: u32) -> bool {
        self.leave(id)
    }
    /// Queue input for next tick; last wins (rate clamp). False on bad id.
    pub fn submit(&mut self, id: u32, input: u32) -> bool {
        let i = id as usize;
        if i < MAX_PLAYERS && self.alive[i] && self.live {
            self.pend[i] = input;
            return true;
        }
        false
    }
    /// Advance one fixed tick, consuming pending inputs authoritatively.
    pub fn tick(&mut self) {
        if !self.live {
            return;
        }
        for i in 0..MAX_PLAYERS {
            if !self.alive[i] {
                continue;
            }
            let (dx, dy) = decode(self.pend[i]);
            self.pend[i] = 0;
            self.xs[i] = (self.xs[i] + dx * MAX_STEP_CM).clamp(0, self.cfg.world - 1);
            self.ys[i] = (self.ys[i] + dy * MAX_STEP_CM).clamp(0, self.cfg.world - 1);
        }
        self.tick += 1;
        self.rebuild();
        self.rehash();
    }
    /// Alias for tick.
    pub fn step(&mut self) {
        self.tick();
    }
    /// Submit many then tick once.
    pub fn tick_with(&mut self, inputs: &[(u32, u32)]) {
        for &(id, inp) in inputs {
            self.submit(id, inp);
        }
        self.tick();
    }
    /// Position of id in cm.
    pub fn pos(&self, id: u32) -> Option<(i32, i32)> {
        let i = id as usize;
        if i < MAX_PLAYERS && self.alive[i] {
            Some((self.xs[i], self.ys[i]))
        } else {
            None
        }
    }
    /// AOI snapshot for viewer, culled to budget (nearest-first, id tiebreak).
    pub fn snapshot_for(&self, viewer: u32) -> Snapshot {
        let vi = viewer as usize;
        let (vx, vy) = if vi < MAX_PLAYERS && self.alive[vi] {
            (self.xs[vi], self.ys[vi])
        } else {
            (0, 0)
        };
        let r2 = i64::from(self.cfg.view) * i64::from(self.cfg.view);
        let mut tmp: Vec<(i64, u32)> = Vec::new();
        let cell_r = (self.cfg.view + self.cfg.cell - 1) / self.cfg.cell.max(1);
        let ccx = vx / self.cfg.cell.max(1);
        let ccy = vy / self.cfg.cell.max(1);
        for dy in -cell_r..=cell_r {
            for dx in -cell_r..=cell_r {
                let (gx, gy) = (ccx + dx, ccy + dy);
                if gx < 0 || gy < 0 || gx >= self.gw || gy >= self.gh {
                    continue;
                }
                let idx = (gy * self.gw + gx) as usize;
                for &id in &self.cells[idx] {
                    let i = id as usize;
                    let ddx = i64::from(self.xs[i]) - i64::from(vx);
                    let ddy = i64::from(self.ys[i]) - i64::from(vy);
                    let d2 = ddx * ddx + ddy * ddy;
                    if d2 <= r2 {
                        tmp.push((d2, id));
                    }
                }
            }
        }
        tmp.sort_unstable();
        tmp.dedup_by(|a, b| a.1 == b.1);
        let max_ents = self.cfg.budget.saturating_sub(HEADER_BYTES) / BYTES_PER_ENT;
        tmp.truncate(max_ents);
        let ents = tmp
            .into_iter()
            .map(|(_, id)| {
                let i = id as usize;
                SnapEnt {
                    id,
                    x: self.xs[i],
                    y: self.ys[i],
                }
            })
            .collect();
        Snapshot {
            tick: self.tick,
            viewer,
            ents,
        }
    }
    /// Alias for snapshot_for.
    pub fn snapshot(&self, viewer: u32) -> Snapshot {
        self.snapshot_for(viewer)
    }
    /// Snapshots for all alive viewers, sorted by id.
    pub fn snapshot_all(&self) -> Vec<Snapshot> {
        let mut ids: Vec<u32> = (0..MAX_PLAYERS as u32)
            .filter(|&id| self.alive[id as usize])
            .collect();
        ids.sort_unstable();
        ids.into_iter().map(|id| self.snapshot_for(id)).collect()
    }
    /// Wire bytes for viewer.
    pub fn bytes_for(&self, viewer: u32) -> usize {
        self.snapshot_for(viewer).byte_len()
    }
    /// Total wire bytes across all viewers this tick.
    pub fn total_bytes(&self) -> usize {
        (0..MAX_PLAYERS as u32)
            .filter(|&id| self.alive[id as usize])
            .map(|id| self.bytes_for(id))
            .sum()
    }
    /// Max per-client bytes this tick.
    pub fn max_bytes(&self) -> usize {
        (0..MAX_PLAYERS as u32)
            .filter(|&id| self.alive[id as usize])
            .map(|id| self.bytes_for(id))
            .max()
            .unwrap_or(HEADER_BYTES)
    }
    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
    fn cell_idx(&self, x: i32, y: i32) -> usize {
        let gx = (x / self.cfg.cell.max(1)).clamp(0, self.gw - 1);
        let gy = (y / self.cfg.cell.max(1)).clamp(0, self.gh - 1);
        (gy * self.gw + gx) as usize
    }
    fn rebuild(&mut self) {
        for c in self.cells.iter_mut() {
            c.clear();
        }
        for i in 0..MAX_PLAYERS {
            if self.alive[i] {
                let idx = self.cell_idx(self.xs[i], self.ys[i]);
                self.cells[idx].push(i as u32);
            }
        }
        for c in self.cells.iter_mut() {
            c.sort_unstable();
        }
    }
    fn rehash(&mut self) {
        let mut fold = Fnv64::new();
        fold.write_u64(self.tick);
        for i in 0..MAX_PLAYERS {
            if self.alive[i] {
                fold.write_u32(i as u32);
                fold.write_u32(self.xs[i].cast_unsigned());
                fold.write_u32(self.ys[i].cast_unsigned());
            }
        }
        self.hash = fold.finish();
    }
}
/// Decode input bits to (dx,dy) in {-1,0,1}; high bits masked (authoritative).
pub fn decode(input: u32) -> (i32, i32) {
    let dx = ((input & 1) as i32) - (((input >> 1) & 1) as i32);
    let dy = (((input >> 2) & 1) as i32) - (((input >> 3) & 1) as i32);
    (dx, dy)
}
/// True iff b within radius r of a (cm, integer dist2).
pub fn interest(ax: i32, ay: i32, bx: i32, by: i32, r: i32) -> bool {
    let dx = i64::from(bx) - i64::from(ax);
    let dy = i64::from(by) - i64::from(ay);
    dx * dx + dy * dy <= i64::from(r) * i64::from(r)
}
/// Deterministic lobby draft: first n of pool rotated by seed.
pub fn draft(pool: &[u32], n: usize, seed: u64) -> Vec<u32> {
    if pool.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut v: Vec<u32> = pool.to_vec();
    v.sort_unstable();
    let k = (seed % v.len() as u64) as usize;
    v.rotate_left(k);
    v.truncate(n.min(v.len()));
    v
}
/// Headless chaos harness: 64p join + seeded ticks. Returns (hash, max_bytes).
pub fn chaos(seed: u64, ticks: u64) -> (u64, usize) {
    let mut s = Server::with_seed(seed);
    for _ in 0..MAX_PLAYERS {
        s.join();
    }
    let mut rng = seed | 1;
    let mut peak = 0usize;
    for _ in 0..ticks {
        for id in 0..MAX_PLAYERS as u32 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            s.submit(id, (rng & 0xF) as u32);
        }
        s.tick();
        peak = peak.max(s.max_bytes());
    }
    (s.state_hash(), peak)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chaos_64_under_budget() {
        let (h, peak) = chaos(19, 120);
        assert!(peak <= DEFAULT_BUDGET, "peak {peak}");
        assert_ne!(h, 0xcbf29ce484222325);
    }
    #[test]
    fn determinism_x2() {
        assert_eq!(chaos(7, 60), chaos(7, 60));
        assert_ne!(chaos(7, 60).0, chaos(8, 60).0);
    }
    #[test]
    fn authoritative_clamp() {
        let mut s = Server::with_seed(1);
        let id = s.join().unwrap();
        let (x0, y0) = s.pos(id).unwrap();
        s.submit(id, u32::MAX);
        s.tick();
        let (x1, y1) = s.pos(id).unwrap();
        assert!((x1 - x0).abs() <= MAX_STEP_CM && (y1 - y0).abs() <= MAX_STEP_CM);
        assert!(!s.submit(999, 1) && s.pos(999).is_none());
    }
    #[test]
    fn aoi_sorted_and_excludes_far() {
        let mut s = Server::new(Config {
            seed: 3,
            budget: DEFAULT_BUDGET,
            cell: 1600,
            view: 4800,
            world: WORLD_CM,
        });
        let a = s.join().unwrap();
        let b = s.join().unwrap();
        s.xs[a as usize] = 1000;
        s.ys[a as usize] = 1000;
        s.xs[b as usize] = WORLD_CM - 1;
        s.ys[b as usize] = WORLD_CM - 1;
        s.rebuild();
        let snap = s.snapshot_for(a);
        assert!(snap.ents.iter().any(|e| e.id == a));
        assert!(!snap.ents.iter().any(|e| e.id == b));
        assert!(interest(0, 0, 10, 10, 4800) && !interest(0, 0, WORLD_CM - 1, WORLD_CM - 1, 4800));
        assert_eq!(
            snap.byte_len(),
            HEADER_BYTES + snap.ents.len() * BYTES_PER_ENT
        );
        assert!(snap.byte_len() <= DEFAULT_BUDGET);
    }
    #[test]
    fn clustered_never_exceeds_budget() {
        let mut s = Server::with_seed(5);
        for _ in 0..MAX_PLAYERS {
            s.join();
        }
        for i in 0..MAX_PLAYERS {
            s.xs[i] = 8000;
            s.ys[i] = 8000;
        }
        s.rebuild();
        for id in 0..MAX_PLAYERS as u32 {
            assert!(s.bytes_for(id) <= DEFAULT_BUDGET, "{}", s.bytes_for(id));
        }
        assert_eq!(
            s.snapshot_for(0).ents.len(),
            (DEFAULT_BUDGET - HEADER_BYTES) / BYTES_PER_ENT
        );
    }
    #[test]
    fn lifecycle_presence_draft() {
        let mut s = Server::with_seed(9);
        assert!(s.is_live() && s.health());
        let mut ids = Vec::new();
        for _ in 0..MAX_PLAYERS {
            ids.push(s.join().unwrap());
        }
        assert_eq!(s.count(), MAX_PLAYERS);
        assert!(s.join().is_none());
        assert!(s.leave(ids[0]));
        assert_eq!(s.count(), MAX_PLAYERS - 1);
        assert_eq!(draft(&ids, 4, 9).len(), 4);
        assert_eq!(draft(&ids, 4, 9), draft(&ids, 4, 9));
        s.shutdown();
        assert!(!s.is_live() && s.join().is_none());
    }
}
