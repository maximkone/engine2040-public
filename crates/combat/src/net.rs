//! net: the retiarius' weighted net and its wrist cord (row SG12), a 3 m
//! cast in an arc that entangles a man, then hauls him off his feet.
//!
//! Row: "a 3 m weighted net cast in an arc; a hit entangles (stagger, no
//! attack) and holding the cord hauls the victim off their feet (knockdown)
//! after 30 ticks; a miss hauls the net back to the hand (recovery). Counter:
//! dodge out of the arc, or cut the cord (SG13)."
//!
//! Pure and integer: no tick, no world, no heap, no trig call. The caller arms
//! a cast with the thrower's feet and facing, then hands this module one
//! victim capsule per tick; it answers with the phase and the pull the cord
//! applied, and the caller moves the bodies. The fan is fixed at the release,
//! so a man who steps out of the arc during the flight is missed. Units:
//! millimetres, ticks at 60 Hz, facing any non-zero x/z vector (never
//! normalized, so its scale cancels in every test). Determinism: integer
//! compares and `u64::isqrt` only; the same cast plus the same victim path
//! gives the same state and the same trace. Errors: none; a bad cast is
//! refused (`false`) and a malformed body is a miss, never a panic.

use engine_core::checked::saturating_i64_to_i32;

/// Reach of the cast, mm (row: "a 3 m weighted net").
pub const NET_REACH_MM: i32 = 3_000;
/// Half-width of the cast fan, degrees. The row gives no width: a 3 m net
/// opens about 4 m across at the end of the cast, so 35 degrees a side.
pub const NET_ARC_HALF_DEG: u16 = 35;
/// Ticks the net is in the air at 60 Hz. The row gives no flight time: a
/// weighted net covers its 3 m in a quarter second.
pub const CAST_TICKS: u32 = 15;
/// Ticks a netted man stays staggered before the haul (row: "after 30 ticks").
pub const ENTANGLE_TICKS: u32 = 30;
/// Ticks the net takes to come back to the hand after a miss or a haul. The
/// row gives no count: the same quarter second as the cast.
pub const RECOVER_TICKS: u32 = 15;
/// Wrist slack past the cast reach, mm. The row gives no cord length: the cord
/// is as long as the cast plus a wrist's give.
pub const WRIST_SLACK_MM: i32 = 300;
/// Longest the wrist cord pays out, mm: the cast plus the wrist's slack.
pub const CORD_MAX_MM: i32 = NET_REACH_MM + WRIST_SLACK_MM;
/// Distance the haul drags the netted man toward the thrower, mm: off his
/// feet and a body's width in. The row gives no distance.
pub const HAUL_MM: i32 = 600;
/// Closest the cord brings a man, mm: at the thrower's feet, never through
/// him. The row gives no floor.
pub const MIN_GAP_MM: i32 = 300;
/// Most the cord drags a straining man in one tick, mm. The row gives no rate.
pub const SNUB_MAX_MM: i32 = 120;
/// Height of the net's weighted rim as it leaves the wrist, mm: a body whose
/// capsule top sits under it passes beneath the net (a prone man, no catch).
/// The row gives no rim height.
pub const NET_RIM_MM: i32 = 500;
/// No victim slot: the net is in the hand or in the air.
pub const NO_VICTIM: u8 = 0xFF;

/// Q16 one, the scale of [`TAN_HALF_Q16`].
const Q16: i64 = 65_536;
/// `tan(NET_ARC_HALF_DEG)` in Q16, rounded to nearest: the cone test compares
/// a cross product against a dot product, so no trig runs in the tick.
const TAN_HALF_Q16: i64 = 45_889;
/// Widest position component accepted, mm (+/-200 m arena scale).
const POS_BOUND_MM: i32 = 200_000;
/// Widest facing component accepted before the solver refuses the input.
const FACING_BOUND: i32 = 1_000_000;
/// Widest capsule radius or half height accepted, mm.
const BODY_MAX_MM: u32 = 10_000;

/// One body the net can fall on: its ground point and hurt capsule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetBody {
    /// Feet on the ground, mm.
    pub pos_mm: [i32; 3],
    /// Hurt capsule radius, mm.
    pub radius_mm: u32,
    /// Hurt capsule half height, mm.
    pub half_height_mm: u32,
}

impl NetBody {
    /// A standing man: 300 mm radius, 900 mm half height.
    #[must_use]
    pub const fn standing(pos_mm: [i32; 3]) -> Self {
        Self {
            pos_mm,
            radius_mm: 300,
            half_height_mm: 900,
        }
    }
}

/// Where the net is in its one life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetPhase {
    /// In the hand, ready to cast.
    #[default]
    Idle,
    /// In the air, fan fixed at the release.
    Flying,
    /// Over a man: he is staggered, cannot attack, and the cord is taut.
    Entangled,
    /// Coming back to the hand after a miss or a haul.
    Recover,
}

/// What one tick of the net did. The caller moves the bodies by `pull_mm`
/// toward the release point (`pulled` does the arithmetic).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetTick {
    /// The cast finished this tick (hit or miss).
    pub landed: bool,
    /// The net fell over the victim.
    pub hit: bool,
    /// The row's 30-tick haul fired: a knockdown on the victim.
    pub hauled: bool,
    /// The net is back in the hand.
    pub recovered: bool,
    /// Distance the cord drags the victim toward the thrower, mm.
    pub pull_mm: i32,
}

/// The net and its wrist cord, as state the caller owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetState {
    phase: NetPhase,
    age: u32,
    origin_mm: [i32; 3],
    dir: [i32; 2],
    cord_mm: i32,
    victim: u8,
}

impl Default for NetState {
    fn default() -> Self {
        Self::new()
    }
}

impl NetState {
    /// A net in the hand.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: NetPhase::Idle,
            age: 0,
            origin_mm: [0; 3],
            dir: [0, 1],
            cord_mm: 0,
            victim: NO_VICTIM,
        }
    }

    /// Current phase.
    #[must_use]
    pub const fn phase(&self) -> NetPhase {
        self.phase
    }

    /// Ticks spent in the current phase.
    #[must_use]
    pub const fn age(&self) -> u32 {
        self.age
    }

    /// Cord paid out at the catch, mm (`0` while the net is in the hand).
    #[must_use]
    pub const fn cord_mm(&self) -> i32 {
        self.cord_mm
    }

    /// The netted man's slot, `None` when no man is netted.
    #[must_use]
    pub const fn victim(&self) -> Option<u8> {
        if self.victim == NO_VICTIM {
            None
        } else {
            Some(self.victim)
        }
    }

    /// True while the net is out of the hand.
    #[must_use]
    pub const fn busy(&self) -> bool {
        !matches!(self.phase, NetPhase::Idle)
    }

    /// True while the netted man is staggered and his attack is refused: the
    /// row's whole entangle tick count.
    #[must_use]
    pub const fn staggered(&self) -> bool {
        matches!(self.phase, NetPhase::Entangled)
    }

    /// Arm a cast from the thrower's feet along `dir`, netting `slot` if it
    /// lands on him. Returns `false` when the net is not in the hand or the
    /// facing is zero: one net, one cast.
    pub fn cast(&mut self, thrower_mm: [i32; 3], dir: [i32; 2], slot: u8) -> bool {
        if self.busy() || (dir[0] == 0 && dir[1] == 0) {
            return false;
        }
        self.phase = NetPhase::Flying;
        self.age = 0;
        self.origin_mm = thrower_mm;
        self.dir = dir;
        self.cord_mm = 0;
        self.victim = slot;
        true
    }

    /// Advance one tick with the victim where he is and the cord in the
    /// thrower's hand (or dropped). Pure: the same state plus the same body
    /// gives the same tick and the same next state.
    pub fn step(&mut self, victim: &NetBody, holding: bool) -> NetTick {
        let mut tick = NetTick::default();
        match self.phase {
            NetPhase::Idle => {}
            NetPhase::Flying => {
                self.age = self.age.saturating_add(1);
                if self.age >= CAST_TICKS {
                    tick.landed = true;
                    if in_arc(self.origin_mm, self.dir, victim) && over_rim(victim) {
                        tick.hit = true;
                        self.phase = NetPhase::Entangled;
                        self.age = 0;
                        self.cord_mm = cord_for(self.origin_mm, victim.pos_mm);
                    } else {
                        self.phase = NetPhase::Recover;
                        self.age = 0;
                        self.victim = NO_VICTIM;
                    }
                }
            }
            NetPhase::Entangled => {
                self.age = self.age.saturating_add(1);
                let gap = ground_gap_mm(self.origin_mm, victim.pos_mm);
                // The wrist cord only tightens: a man who strains beyond the
                // paid-out length is dragged back by the excess this tick.
                let slack = gap - i64::from(self.cord_mm);
                if slack > 0 {
                    tick.pull_mm = clamp_i32(slack, SNUB_MAX_MM);
                }
                if self.age >= ENTANGLE_TICKS {
                    if holding {
                        // The row's haul: off his feet, a body's width in.
                        tick.hauled = true;
                        let haul = (gap - i64::from(MIN_GAP_MM)).clamp(0, i64::from(HAUL_MM));
                        tick.pull_mm = clamp_i32(haul, HAUL_MM).max(tick.pull_mm);
                    }
                    self.phase = NetPhase::Recover;
                    self.age = 0;
                    self.victim = NO_VICTIM;
                }
            }
            NetPhase::Recover => {
                self.age = self.age.saturating_add(1);
                if self.age >= RECOVER_TICKS {
                    self.phase = NetPhase::Idle;
                    self.age = 0;
                    self.cord_mm = 0;
                    tick.recovered = true;
                }
            }
        }
        tick
    }
}

/// True when a body's ground point lies in the fan the net covers: inside the
/// 3 m reach plus the body's radius, and inside the arc plus the radius, both
/// widened by the capsule so a graze counts. Behind the thrower never hits.
#[must_use]
pub fn in_arc(origin_mm: [i32; 3], dir: [i32; 2], body: &NetBody) -> bool {
    if !body_fits(body) || !pos_fits(origin_mm) || !pos_fits(body.pos_mm) {
        return false;
    }
    let fx = i64::from(dir[0]);
    let fz = i64::from(dir[1]);
    if fx.abs() > i64::from(FACING_BOUND) || fz.abs() > i64::from(FACING_BOUND) {
        return false;
    }
    let flen = isqrt_u64(u64::try_from(fx * fx + fz * fz).unwrap_or(u64::MAX));
    if flen <= 0 {
        return false;
    }
    let dx = i64::from(body.pos_mm[0]) - i64::from(origin_mm[0]);
    let dz = i64::from(body.pos_mm[2]) - i64::from(origin_mm[2]);
    let along = fx * dx + fz * dz;
    if along <= 0 {
        return false;
    }
    let radius = i64::from(body.radius_mm);
    // Reach: the net's far edge, widened by the capsule.
    if along > (i64::from(NET_REACH_MM) + radius) * flen {
        return false;
    }
    // Arc: |cross| <= along*tan + radius*|f|, in Q16 throughout.
    let cross = (fx * dz - fz * dx).abs();
    cross * Q16 <= along * TAN_HALF_Q16 + radius * flen * Q16
}

/// The victim's position after the cord drags him `pull_mm` toward the release
/// point along the ground; height is kept and the pull never overshoots.
#[must_use]
pub fn pulled(origin_mm: [i32; 3], from_mm: [i32; 3], pull_mm: i32) -> [i32; 3] {
    if pull_mm <= 0 {
        return from_mm;
    }
    let gap = ground_gap_mm(origin_mm, from_mm);
    if gap <= 0 {
        return from_mm;
    }
    let step = i64::from(pull_mm).min(gap);
    let dx = i64::from(from_mm[0]) - i64::from(origin_mm[0]);
    let dz = i64::from(from_mm[2]) - i64::from(origin_mm[2]);
    [
        saturating_i64_to_i32(i64::from(from_mm[0]) - dx * step / gap),
        from_mm[1],
        saturating_i64_to_i32(i64::from(from_mm[2]) - dz * step / gap),
    ]
}

/// True when the net's weighted rim clears the body: a capsule whose top sits
/// under the rim passes beneath the net and is not caught.
fn over_rim(body: &NetBody) -> bool {
    let top = i64::from(body.pos_mm[1]) + 2 * i64::from(body.half_height_mm);
    top >= i64::from(NET_RIM_MM)
}

/// Cord paid out at a catch: the gap, capped by the wrist's length.
fn cord_for(origin_mm: [i32; 3], victim_mm: [i32; 3]) -> i32 {
    let gap = ground_gap_mm(origin_mm, victim_mm);
    clamp_i32(gap, CORD_MAX_MM)
}

/// Ground distance between two points, mm.
fn ground_gap_mm(a: [i32; 3], b: [i32; 3]) -> i64 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dz = i64::from(a[2]) - i64::from(b[2]);
    isqrt_u64(u64::try_from(dx * dx + dz * dz).unwrap_or(u64::MAX))
}

/// Integer square root of the squared gap, saturating for garbage input.
fn isqrt_u64(sq: u64) -> i64 {
    i64::try_from(sq.isqrt()).unwrap_or(i64::MAX)
}

/// A non-negative `i64` as an `i32` distance, capped at `cap`.
fn clamp_i32(v: i64, cap: i32) -> i32 {
    if v <= 0 {
        0
    } else {
        i32::try_from(v.min(i64::from(cap))).unwrap_or(cap)
    }
}

/// True when the position is inside the arena's mm envelope.
fn pos_fits(pos: [i32; 3]) -> bool {
    let mut i = 0;
    while i < 3 {
        if pos[i].abs() > POS_BOUND_MM {
            return false;
        }
        i += 1;
    }
    true
}

/// True when the capsule is a physical size.
fn body_fits(body: &NetBody) -> bool {
    body.radius_mm <= BODY_MAX_MM && body.half_height_mm <= BODY_MAX_MM
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The thrower stands at the origin.
    const THROWER: [i32; 3] = [0, 0, 0];
    /// Facing +z, the arena's forward axis at yaw zero.
    const FORWARD: [i32; 2] = [0, 1_000];
    /// The one victim slot these tests net.
    const SLOT: u8 = 1;

    fn man(at: [i32; 3]) -> NetBody {
        NetBody::standing(at)
    }

    /// One cast, stepped until it lands; returns the landing tick.
    fn cast_until_landed(net: &mut NetState, victim: &NetBody) -> NetTick {
        assert!(net.cast(THROWER, FORWARD, SLOT), "a fresh net casts");
        let mut tick = NetTick::default();
        for _ in 0..CAST_TICKS {
            tick = net.step(victim, true);
        }
        assert!(tick.landed, "the cast lands after {CAST_TICKS} ticks");
        tick
    }

    #[test]
    fn net_cast_entangles_and_hauls() {
        // Row SG12: a 3 m net cast in an arc entangles a man (stagger, no
        // attack) for 30 ticks, then the held cord hauls him off his feet.
        let mut net = NetState::new();
        assert!(!net.busy(), "a new net is in the hand");
        let victim = man([0, 0, 2_400]);
        let landed = cast_until_landed(&mut net, &victim);
        assert!(landed.hit, "a man 2.4 m ahead is in the arc");
        assert_eq!(net.phase(), NetPhase::Entangled);
        assert!(net.staggered(), "entangled: staggered, no attack");
        assert_eq!(net.victim(), Some(SLOT), "the net names its man");
        assert_eq!(net.cord_mm(), 2_400, "the cord pays out to the catch");

        // The 30 entangle ticks hold him; the haul fires on the last one.
        for _ in 0..ENTANGLE_TICKS - 1 {
            let tick = net.step(&victim, true);
            assert!(!tick.hauled, "the haul waits its 30 ticks");
            assert!(net.staggered(), "still netted, still no attack");
        }
        let haul = net.step(&victim, true);
        assert!(haul.hauled, "holding the cord hauls him down");
        assert_eq!(haul.pull_mm, HAUL_MM, "the haul drags a body's width in");
        assert!(!net.staggered(), "a downed man is not netted");
        assert_eq!(net.victim(), None, "the net comes off him");
        assert_eq!(net.phase(), NetPhase::Recover);
        let down = pulled(THROWER, victim.pos_mm, haul.pull_mm);
        assert_eq!(
            down,
            [0, 0, 2_400 - HAUL_MM],
            "off his feet, toward the thrower"
        );

        // Recovery: the net is hauled back to the hand, ready again.
        let mut recovered = false;
        for _ in 0..RECOVER_TICKS {
            recovered |= net.step(&victim, true).recovered;
        }
        assert!(recovered, "the net comes back to the hand");
        assert_eq!(net.phase(), NetPhase::Idle);
        assert!(!net.busy());
        assert_eq!(net.cord_mm(), 0, "the cord is gathered with the net");
        assert!(net.cast(THROWER, FORWARD, SLOT), "and it can cast again");
    }

    #[test]
    fn a_dodge_out_of_the_arc_is_a_miss_that_hauls_back() {
        // Counter from the row: dodge out of the arc. The fan is fixed at the
        // release, so stepping sideways during the flight misses.
        let mut net = NetState::new();
        let at_cast = man([0, 0, 2_400]);
        assert!(net.cast(THROWER, FORWARD, SLOT));
        let dodged = man([2_000, 0, 1_000]);
        let mut tick = NetTick::default();
        for _ in 0..CAST_TICKS {
            tick = net.step(&dodged, true);
        }
        assert!(tick.landed, "the cast still lands");
        assert!(!tick.hit, "a man 2 m off the facing is outside the arc");
        assert!(!tick.hauled);
        assert_eq!(net.victim(), None);
        assert!(!net.staggered(), "a miss never staggers him");
        assert_eq!(net.phase(), NetPhase::Recover);
        // The same cast over the man who stood still would have caught him.
        assert!(in_arc(THROWER, FORWARD, &at_cast));

        let mut recovered = false;
        for _ in 0..RECOVER_TICKS {
            recovered |= net.step(&dodged, true).recovered;
        }
        assert!(recovered, "a miss hauls the net back to the hand");
        assert_eq!(net.phase(), NetPhase::Idle);
    }

    #[test]
    fn the_arc_covers_the_cast_reach_and_nothing_behind() {
        // 3 m ahead catches; the same man 0.5 m further is past the net.
        assert!(in_arc(THROWER, FORWARD, &man([0, 0, NET_REACH_MM])));
        assert!(!in_arc(THROWER, FORWARD, &man([0, 0, 3_500])));
        // Behind the thrower never hits, however close.
        assert!(!in_arc(THROWER, FORWARD, &man([0, 0, -500])));
        // The fan is wider than a line and narrower than a half circle: at
        // 2.4 m along the rim is about 1.68 m either side (tan 35 deg).
        assert!(in_arc(THROWER, FORWARD, &man([1_500, 0, 2_400])));
        assert!(!in_arc(THROWER, FORWARD, &man([2_100, 0, 2_400])));
        // A zero facing or a non-physical body is a miss, never a panic.
        assert!(!in_arc(THROWER, [0, 0], &man([0, 0, 1_000])));
        assert!(!in_arc(THROWER, FORWARD, &man([POS_BOUND_MM + 1, 0, 0])));
        let fat = NetBody {
            pos_mm: [0, 0, 1_000],
            radius_mm: BODY_MAX_MM + 1,
            half_height_mm: 900,
        };
        assert!(!in_arc(THROWER, FORWARD, &fat));
    }

    #[test]
    fn a_body_under_the_rim_passes_under_the_net() {
        // A prone man's capsule top sits under the weight's rim: no catch.
        let prone = NetBody {
            pos_mm: [0, 0, 2_400],
            radius_mm: 300,
            half_height_mm: 200,
        };
        let mut net = NetState::new();
        let landed = cast_until_landed(&mut net, &prone);
        assert!(!landed.hit, "a body under the rim is under the net");
        assert_eq!(net.phase(), NetPhase::Recover);
    }

    #[test]
    fn the_wrist_cord_snubs_a_man_who_runs() {
        // The cord paid out at the catch (2.4 m) is the length limit; a man
        // who pulls away is dragged back by the excess, capped per tick.
        let mut net = NetState::new();
        let caught = man([0, 0, 2_400]);
        assert!(cast_until_landed(&mut net, &caught).hit);
        assert_eq!(net.cord_mm(), 2_400);

        let inside = man([0, 0, 2_300]);
        assert_eq!(
            net.step(&inside, true).pull_mm,
            0,
            "slack cord pulls nobody"
        );

        // A little past the paid-out length: the excess alone, under the cap.
        let drifting = man([0, 0, 2_500]);
        assert_eq!(
            net.step(&drifting, true).pull_mm,
            100,
            "the cord takes up its slack"
        );

        let straining = man([0, 0, 3_000]);
        let tick = net.step(&straining, true);
        assert_eq!(tick.pull_mm, SNUB_MAX_MM, "600 mm of excess, capped at 120");
        let reeled = pulled(THROWER, straining.pos_mm, tick.pull_mm);
        assert_eq!(reeled, [0, 0, 2_880], "the cord drags him back in");
        assert!(net.staggered(), "the snub does not free him");

        // A man already at the thrower's feet is never pulled past him.
        let at_feet = man([0, 0, MIN_GAP_MM]);
        assert_eq!(net.step(&at_feet, true).pull_mm, 0);
        assert_eq!(pulled(THROWER, [0, 0, 100], HAUL_MM), THROWER);
    }

    #[test]
    fn a_dropped_cord_ends_the_entangle_without_a_haul() {
        // The row hauls only while the cord is held; let go and the net
        // slips off at the end of the 30 ticks, no knockdown.
        let mut net = NetState::new();
        let victim = man([0, 0, 2_400]);
        assert!(cast_until_landed(&mut net, &victim).hit);
        let mut tick = NetTick::default();
        for _ in 0..ENTANGLE_TICKS {
            tick = net.step(&victim, false);
        }
        assert!(!tick.hauled, "an unheld cord never yanks a man down");
        assert_eq!(tick.pull_mm, 0);
        assert_eq!(net.phase(), NetPhase::Recover, "the net slips off");
        assert_eq!(net.victim(), None);
    }

    #[test]
    fn the_cord_pays_out_no_further_than_its_length() {
        // A fat man at 3.4 m is inside reach + radius but past the wrist cord:
        // the cord stops at 3 m plus the 300 mm slack, and the excess snubs.
        let mut net = NetState::new();
        let fat = NetBody {
            pos_mm: [0, 0, 3_400],
            radius_mm: 600,
            half_height_mm: 900,
        };
        assert!(in_arc(THROWER, FORWARD, &fat), "3.4 m is in reach + radius");
        assert!(cast_until_landed(&mut net, &fat).hit);
        assert_eq!(net.cord_mm(), CORD_MAX_MM, "the cord stops at its length");
        assert_eq!(
            net.step(&fat, true).pull_mm,
            100,
            "the catch is 100 mm past the cord"
        );
    }

    #[test]
    fn the_same_tape_twice_gives_the_same_net_state() {
        // Determinism: one scripted tape (cast, 20 flight and entangle ticks
        // with a walking victim, the haul, the recovery) run twice gives the
        // same state and the same trace, tick for tick.
        fn tape() -> (NetState, Vec<NetTick>) {
            let mut net = NetState::new();
            let mut trace = Vec::with_capacity(64);
            assert!(net.cast(THROWER, FORWARD, SLOT));
            let mut z = 2_000;
            let mut held = true;
            for i in 0..64 {
                if i == 20 {
                    held = false;
                }
                if i < 30 {
                    z += 10;
                }
                trace.push(net.step(&man([0, 0, z]), held));
            }
            (net, trace)
        }
        let (first, first_trace) = tape();
        let (second, second_trace) = tape();
        assert_eq!(first, second, "same tape, same state");
        assert_eq!(first_trace, second_trace, "same tape, same trace");
        assert!(first_trace.iter().any(|t| t.landed), "the tape lands");
        assert!(first_trace.iter().any(|t| t.recovered), "and recovers");
    }

    #[test]
    fn the_rows_numbers_and_the_arc_constant_hold() {
        // The row's two numbers, verbatim: 3 m and 30 ticks.
        assert_eq!(NET_REACH_MM, 3_000);
        assert_eq!(ENTANGLE_TICKS, 30);
        assert_eq!(CORD_MAX_MM, NET_REACH_MM + WRIST_SLACK_MM);
        // tan(35 deg) in Q16, rounded: 0.7002075382 * 65536.
        assert_eq!(TAN_HALF_Q16, 45_889);
        assert_eq!(
            TAN_HALF_Q16 * 2,
            91_778,
            "the arc constant is not silently rewritten"
        );
        let _ = NET_ARC_HALF_DEG;
    }
}
