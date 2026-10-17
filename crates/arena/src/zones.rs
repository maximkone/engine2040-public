//! Arena zones (GP36): where a fight happens, as a pure classifier.
//!
//! Consumer: the balance tournament (`crates/playtest/src/tournament.rs`)
//! tallies every recorded pose through [`zone_at`] and reads the gate anchor
//! from [`shipped_gate_anchor`]; `crates/game/src/fight_ring.rs` re-exports
//! these types so the ring's own zone paths stay valid.
//! Data owner: this module owns the bands, the gate-mouth tie break and the
//! share rounding; the hand-placed layout lives in
//! `content/design/arena-zones.rows` and only the gate anchor is read.
//! Units: meters; shares are permille. Determinism: same inputs give the
//! same zone and the same shares, integer rounding only, no heap, no IO.
//! Errors: [`ZoneError`] names the broken gate row with its 1-based line;
//! an absent or malformed row falls back to [`GATE_ANCHOR_FALLBACK_M`].
//! Budget: classification and tallying are a few float/integer ops each.

/// Centre radius in meters: inside this is [`ArenaZone::Centre`].
pub const CENTRE_R_M: f32 = 3.0;
/// Ring outer radius in meters: inside this (past centre) is [`ArenaZone::Ring`].
pub const RING_R_M: f32 = 8.0;

/// Arena zones (GP36): where a fight happens, as a pure classifier.
///
/// Centre is `r < 3 m`, the ring is `3..=8 m`, the wall band is over 8 m,
/// the gate mouth is a square around the gate anchor and wins over the
/// radial band. The tournament metric (time share per zone,
/// `entrance_share`, kills per zone) reads this; the rows file
/// `content/design/arena-zones.rows` holds the hand-placed layout.
/// Units: meters. Determinism: same inputs, same zone, no heap, no IO.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArenaZone {
    /// Within 3 m of the centre.
    Centre,
    /// Between 3 m and 8 m out.
    Ring,
    /// Past 8 m, against the wall.
    Wall,
    /// Inside the gate mouth square.
    Gate,
}

impl ArenaZone {
    /// Slot of this zone in [`ZoneTally::shares`] order (`0..=3`).
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Centre => 0,
            Self::Ring => 1,
            Self::Wall => 2,
            Self::Gate => 3,
        }
    }
}

/// Classify a floor point into its zone. `gate` is the gate anchor
/// `(x, z)` in meters, `gate_half_m` is the mouth half-edge. The gate
/// mouth wins ties so camping the entrance always counts as entrance.
#[must_use]
pub fn zone_at(x_m: f32, z_m: f32, gate: [f32; 2], gate_half_m: f32) -> ArenaZone {
    if (x_m - gate[0]).abs() <= gate_half_m && (z_m - gate[1]).abs() <= gate_half_m {
        return ArenaZone::Gate;
    }
    let r = (x_m * x_m + z_m * z_m).sqrt();
    if r < CENTRE_R_M {
        ArenaZone::Centre
    } else if r <= RING_R_M {
        ArenaZone::Ring
    } else {
        ArenaZone::Wall
    }
}

/// Running zone counts for one tournament: feeds the time-share and
/// `entrance_share` metrics. Plain counters, no heap. Determinism: same
/// add order gives the same shares.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ZoneTally {
    /// Ticks with a fighter in the centre.
    pub centre: u64,
    /// Ticks with a fighter in the ring.
    pub ring: u64,
    /// Ticks with a fighter at the wall.
    pub wall: u64,
    /// Ticks with a fighter holding the gate mouth.
    pub gate: u64,
}

impl ZoneTally {
    /// Add one tick in `zone`.
    pub fn add(&mut self, zone: ArenaZone) {
        match zone {
            ArenaZone::Centre => self.centre += 1,
            ArenaZone::Ring => self.ring += 1,
            ArenaZone::Wall => self.wall += 1,
            ArenaZone::Gate => self.gate += 1,
        }
    }

    /// Ticks counted so far, saturating.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.centre
            .saturating_add(self.ring)
            .saturating_add(self.wall)
            .saturating_add(self.gate)
    }

    /// Share of one zone in permille (0 on empty), floored.
    #[must_use]
    pub fn permille(&self, zone: ArenaZone) -> u32 {
        let total = self.total();
        if total == 0 {
            return 0;
        }
        let count = match zone {
            ArenaZone::Centre => self.centre,
            ArenaZone::Ring => self.ring,
            ArenaZone::Wall => self.wall,
            ArenaZone::Gate => self.gate,
        };
        u32::try_from(count.saturating_mul(1000) / total).unwrap_or(u32::MAX)
    }

    /// Gate-mouth time share in permille: the GP36 bar keeps this under 250.
    #[must_use]
    pub fn entrance_share(&self) -> u32 {
        self.permille(ArenaZone::Gate)
    }

    /// Four zone shares in permille as `[centre, ring, wall, gate]`, rounded
    /// so they always sum to exactly 1000 (`[0; 4]` on an empty tally). A
    /// plain floor drops up to three points; the largest fractional
    /// remainders among centre/ring/wall take them, so the gate share keeps
    /// its exact floor and equals [`ZoneTally::entrance_share`].
    #[must_use]
    pub fn shares(&self) -> [u32; 4] {
        let total = self.total();
        if total == 0 {
            return [0; 4];
        }
        let counts = [self.centre, self.ring, self.wall, self.gate];
        let mut shares = [0u32; 4];
        let mut remainders = [0u64; 4];
        let mut assigned = 0u64;
        for (index, count) in counts.iter().enumerate() {
            let scaled = count.saturating_mul(1000);
            shares[index] = u32::try_from(scaled / total).unwrap_or(u32::MAX);
            remainders[index] = scaled % total;
            assigned = assigned.saturating_add(u64::from(shares[index]));
        }
        let mut leftover = 1000u64.saturating_sub(assigned);
        while leftover > 0 {
            // Largest remainder among the non-gate zones wins; ties keep the
            // earlier zone, so the same counts always give the same shares.
            let mut best = 0usize;
            for index in 1..3 {
                if remainders[index] > remainders[best] {
                    best = index;
                }
            }
            if remainders[best] == 0 {
                break;
            }
            shares[best] = shares[best].saturating_add(1);
            remainders[best] = 0;
            leftover -= 1;
        }
        shares
    }
}

/// Shipped zones layout, compiled in: the gate anchor is data, never a
/// constant (`content/design/arena-zones.rows`).
pub const ZONES_ROWS: &str = include_str!("../../../content/design/arena-zones.rows");

/// Row id of the entrance tunnel in the zones layout.
const GATE_ROW_ID: &str = "gate_tunnel";
/// Whole-number cm fields the gate anchor reads, in output order.
const GATE_KEYS: [&str; 3] = ["cx_cm", "cz_cm", "radius_cm"];

/// Fallback gate mouth anchor `[x_m, z_m, half_m]` used when the shipped
/// zones layout has no readable gate row. The entrance tunnel sits on the
/// centre lane 2 m outside the 10 m duel floor (12 m out, 2 m half-edge);
/// the constant keeps the metric live if a designer drops the row.
pub const GATE_ANCHOR_FALLBACK_M: [f32; 3] = [12.0, 0.0, 2.0];

/// Gate-anchor read failure: which row broke and where.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoneError {
    /// The text carries no `gate_tunnel` row.
    MissingRow,
    /// A gate field is missing, repeated or not a whole number; the 1-based
    /// `line` and the effective `field` name the culprit.
    BadField {
        /// 1-based line of the gate row.
        line: usize,
        /// Effective field name (`pair` for a token without `=`).
        field: &'static str,
    },
}

impl core::fmt::Display for ZoneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingRow => write!(f, "arena zones: no gate_tunnel row"),
            Self::BadField { line, field } => {
                write!(f, "arena zones line {line}: bad gate field {field}")
            }
        }
    }
}

impl std::error::Error for ZoneError {}

/// ASCII digits only, no sign, no dot, checked overflow; `None` refuses.
fn decimal(text: &str) -> Option<u32> {
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

/// Whole centimeters as meters (gate values stay far below `u16::MAX`).
fn cm_to_m(cm: u32) -> f32 {
    f32::from(u16::try_from(cm).unwrap_or(u16::MAX)) / 100.0
}

/// Read the gate mouth anchor `[x_m, z_m, half_m]` from `rows`
/// (`content/design/arena-zones.rows`: a `table <name> version <n>` header
/// then one `<id> <key>=<int> ...` row per actor, whole numbers in cm).
///
/// Only the `gate_tunnel` row's `cx_cm`, `cz_cm` and `radius_cm` are read
/// (cm -> m); other rows and keys are ignored. `#` starts a comment and
/// blank lines are skipped. A malformed gate row is a [`ZoneError`]; an
/// absent gate row is [`ZoneError::MissingRow`], which
/// [`shipped_gate_anchor`] turns into [`GATE_ANCHOR_FALLBACK_M`].
pub fn gate_anchor(rows: &str) -> Result<[f32; 3], ZoneError> {
    for (index, raw) in rows.lines().enumerate() {
        let line = index + 1;
        let body = raw.split('#').next().unwrap_or("").trim();
        let mut words = body.split_whitespace();
        if words.next() != Some(GATE_ROW_ID) {
            continue;
        }
        let mut fields: [Option<u32>; 3] = [None; 3];
        for word in words {
            let (key, value) = word.split_once('=').ok_or(ZoneError::BadField {
                line,
                field: "pair",
            })?;
            let Some(slot) = GATE_KEYS.iter().position(|known| *known == key) else {
                continue;
            };
            if fields[slot].is_some() {
                return Err(ZoneError::BadField {
                    line,
                    field: GATE_KEYS[slot],
                });
            }
            fields[slot] = Some(decimal(value).ok_or(ZoneError::BadField {
                line,
                field: GATE_KEYS[slot],
            })?);
        }
        let mut anchor = [0.0f32; 3];
        for (slot, key) in GATE_KEYS.iter().enumerate() {
            let value = fields[slot].ok_or(ZoneError::BadField { line, field: key })?;
            anchor[slot] = cm_to_m(value);
        }
        return Ok(anchor);
    }
    Err(ZoneError::MissingRow)
}

/// [`gate_anchor`] with the documented [`GATE_ANCHOR_FALLBACK_M`] on any
/// refusal, so a dropped or malformed row can never silence the metric.
#[must_use]
fn anchor_or_fallback(rows: &str) -> [f32; 3] {
    gate_anchor(rows).unwrap_or(GATE_ANCHOR_FALLBACK_M)
}

/// Gate mouth anchor `[x_m, z_m, half_m]` from the shipped layout
/// ([`ZONES_ROWS`]); the tournament metric reads this.
#[must_use]
pub fn shipped_gate_anchor() -> [f32; 3] {
    anchor_or_fallback(ZONES_ROWS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_sum_to_1000_over_a_count_sweep() {
        // Every count combination: the four shares sum to exactly 1000 and
        // the gate share keeps its floored value (0,0,0,0 stays empty).
        for centre in 0..7u64 {
            for ring in 0..7u64 {
                for wall in 0..7u64 {
                    for gate in 0..7u64 {
                        let tally = ZoneTally {
                            centre,
                            ring,
                            wall,
                            gate,
                        };
                        let shares = tally.shares();
                        assert_eq!(shares[3], tally.entrance_share(), "{tally:?}");
                        if tally.total() == 0 {
                            assert_eq!(shares, [0; 4]);
                        } else {
                            assert_eq!(shares.iter().sum::<u32>(), 1000, "{tally:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn shares_keep_the_earlier_zone_on_a_remainder_tie() {
        // One tick each of centre/ring/wall: the naive floors total 999, and
        // the single dropped point goes to the earliest zone.
        let tally = ZoneTally {
            centre: 1,
            ring: 1,
            wall: 1,
            gate: 0,
        };
        assert_eq!(tally.shares(), [334, 333, 333, 0]);
        assert_eq!(tally.shares().iter().sum::<u32>(), 1000);
    }

    #[test]
    fn shipped_rows_give_the_gate_anchor_twice_alike() {
        // The shipped gate row sits at 10 m on the centre lane with a 2 m
        // half-edge; parsed twice alike and equal to the shipped reader.
        let first = gate_anchor(ZONES_ROWS).expect("shipped gate row parses");
        let second = gate_anchor(ZONES_ROWS).expect("shipped gate row replays");
        assert_eq!(first, second);
        assert_eq!(first, [10.0, 0.0, 2.0]);
        assert_eq!(shipped_gate_anchor(), first);
    }

    #[test]
    fn absent_or_malformed_gate_rows_refuse_typed() {
        let head = "table arena-zones version 1\n";
        let absent = format!(
            "{head}sand_floor class=0 cx_cm=-6000 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n"
        );
        assert_eq!(gate_anchor(&absent), Err(ZoneError::MissingRow));
        let float = format!(
            "{head}gate_tunnel class=2 cx_cm=12.0 cz_cm=0 radius_cm=200 height_cm=500 advantage=0 dominance=0 risk=1\n"
        );
        assert_eq!(
            gate_anchor(&float),
            Err(ZoneError::BadField {
                line: 2,
                field: "cx_cm"
            })
        );
        let short = format!("{head}gate_tunnel class=2 cz_cm=0 radius_cm=200\n");
        assert_eq!(
            gate_anchor(&short),
            Err(ZoneError::BadField {
                line: 2,
                field: "cx_cm"
            })
        );
    }

    #[test]
    fn absent_row_falls_back_to_the_documented_constant() {
        // A file with no gate row still yields the documented entrance, so a
        // dropped row can never silence the metric.
        assert_eq!(
            anchor_or_fallback("table arena-zones version 1\n"),
            GATE_ANCHOR_FALLBACK_M
        );
        let bad =
            "table arena-zones version 1\ngate_tunnel class=2 cx_cm=x cz_cm=0 radius_cm=200\n";
        assert_eq!(anchor_or_fallback(bad), GATE_ANCHOR_FALLBACK_M);
    }
}
