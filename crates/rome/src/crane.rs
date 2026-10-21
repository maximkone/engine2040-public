//! crane: the Roman lifting machines, the trispastos and the pentaspastos.
//!
//! A crane is data: `content/rome/tech/crane.toml` says how many pulleys each
//! machine has, how many gangs it is rated for, how hard a gang pulls and how
//! much it lifts. The reader refuses a number with no `ref`, so no number enters
//! a Rome rule unsourced.
//!
//! The rule the game plays by: the pull on the ropes times the purchase (the
//! pulleys) is the force the load feels, and the load rises at the rate of that
//! force over its own weight, in millimetres a tick. A crane short of a gang is
//! out of balance: it works at a fraction of its rate, and the balance divisor
//! in the table says how much. The crew count therefore sets the lift speed.
//!
//! Integers only: no clock, no IO, no float. The same table, the same crew and
//! the same load give the same lift and the same digest twice.

use core::fmt;

use crate::table::{Entry, Table};
use crate::{int, only, LoadError};
use engine_core::Fnv64;

/// The drive that turns the drums of a crane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    /// The wheel the crew walks in (Vitruvius 10.2).
    Treadwheel,
}

impl Drive {
    /// The word this drive is written as in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Treadwheel => "treadwheel",
        }
    }

    /// Read the word of a drive, or `None` when it is not one we know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "treadwheel" => Some(Self::Treadwheel),
            _ => None,
        }
    }
}

/// The lifting machines the sources give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Machine {
    /// The trispastos: three pulleys (Vitruvius 10.2).
    Trispastos,
    /// The pentaspastos: five pulleys (Vitruvius 10.2).
    Pentaspastos,
}

impl Machine {
    /// Every machine, in table order.
    pub const ALL: [Self; 2] = [Self::Trispastos, Self::Pentaspastos];

    /// The table entry this machine reads, for example `crane.trispastos`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::Trispastos => "crane.trispastos",
            Self::Pentaspastos => "crane.pentaspastos",
        }
    }

    /// Position in the crane table's machine list.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Trispastos => 0,
            Self::Pentaspastos => 1,
        }
    }
}

/// One lifting machine as the table gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rig<'a> {
    /// Which machine this is.
    pub machine: Machine,
    /// The pulleys: the purchase of the machine.
    pub pulleys: u32,
    /// The gangs the machine is rated for.
    pub gangs: u32,
    /// The men in one gang.
    pub men_per_gang: u32,
    /// The pull of one gang, in kilograms.
    pub pull_kg_per_gang: u32,
    /// The most the machine lifts, in kilograms.
    pub capacity_kg: u32,
    /// The drive that turns the drums.
    pub drive: Drive,
    /// The source of these numbers.
    pub source: &'a str,
}

impl Rig<'_> {
    /// The force the machine puts on the load: the rated crew's pull times the
    /// purchase.
    #[must_use]
    pub fn force_kg(&self) -> u32 {
        self.pull_kg_per_gang
            .saturating_mul(self.gangs)
            .saturating_mul(self.pulleys)
    }
}

/// The crane table: the machines a site can work and the rules they share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cranes<'a> {
    rigs: [Rig<'a>; Machine::ALL.len()],
    wheel_men: u32,
    short_gang_divisor: u32,
}

impl Cranes<'static> {
    /// The crane table shipped in `content/rome/tech/crane.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/tech/crane.toml");

    /// Read the shipped crane table.
    ///
    /// # Errors
    ///
    /// Same as [`Cranes::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Cranes<'a> {
    /// Read one crane table.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or a
    /// section holds a number and no `ref`, and the typed variants when a
    /// section or key is missing, unknown, zero or too large for its number.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let rigs = [
            rig(&table, Machine::Trispastos)?,
            rig(&table, Machine::Pentaspastos)?,
        ];
        let wheel = table.entry("treadwheel").ok_or(LoadError::MissingEntry {
            entry: "treadwheel",
        })?;
        only(wheel, &["men"])?;
        let balance = table
            .entry("balance")
            .ok_or(LoadError::MissingEntry { entry: "balance" })?;
        only(balance, &["short_gang_divisor"])?;
        Ok(Self {
            rigs,
            wheel_men: positive(wheel, "men")?,
            short_gang_divisor: positive(balance, "short_gang_divisor")?,
        })
    }

    /// The machine of this kind the table holds.
    #[must_use]
    pub fn rig(&self, machine: Machine) -> &Rig<'a> {
        &self.rigs[machine.index()]
    }

    /// The men in the wheel that turns the drums.
    #[must_use]
    pub fn wheel_men(&self) -> u32 {
        self.wheel_men
    }

    /// Lift a load with one of the machines.
    ///
    /// The load rises at the rate of the machine's force over the load's own
    /// weight. A crew short of the machine's rating is out of balance: the
    /// balance divisor divides the rate, so a missing gang halves the speed.
    ///
    /// # Errors
    ///
    /// [`LiftError::NoCrew`] when no gang is on the ropes, [`LiftError::NoLoad`]
    /// when the load weighs nothing, and [`LiftError::TooHeavy`] when the load
    /// is over the machine's capacity.
    pub fn lift(
        &self,
        machine: Machine,
        crew: Crew,
        load_kg: u32,
        height_mm: u32,
    ) -> Result<Lift, LiftError> {
        let rig = self.rig(machine);
        if crew.gangs == 0 {
            return Err(LiftError::NoCrew);
        }
        if load_kg == 0 {
            return Err(LiftError::NoLoad);
        }
        if load_kg > rig.capacity_kg {
            return Err(LiftError::TooHeavy {
                load_kg,
                capacity_kg: rig.capacity_kg,
            });
        }
        let gangs = crew.gangs.min(rig.gangs);
        let missing = rig.gangs - gangs;
        let balance = self.short_gang_divisor.saturating_pow(missing);
        let rate = Rate {
            num: rig.force_kg(),
            den: load_kg.saturating_mul(balance),
        };
        let crew_men = gangs
            .saturating_mul(rig.men_per_gang)
            .saturating_add(self.wheel_men);
        let lift = Lift {
            machine,
            load_kg,
            height_mm,
            ticks: rate.ticks_for(height_mm),
            rate,
            gangs,
            crew_men,
            hash: 0,
        };
        let hash = lift.digest();
        Ok(Lift { hash, ..lift })
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/crane/1");
        for rig in &self.rigs {
            hasher.write_len(rig.machine.entry().len());
            hasher.write_str(rig.machine.entry());
            hasher.write_u32(rig.pulleys);
            hasher.write_u32(rig.gangs);
            hasher.write_u32(rig.men_per_gang);
            hasher.write_u32(rig.pull_kg_per_gang);
            hasher.write_u32(rig.capacity_kg);
            hasher.write_len(rig.drive.as_str().len());
            hasher.write_str(rig.drive.as_str());
        }
        hasher.write_u32(self.wheel_men);
        hasher.write_u32(self.short_gang_divisor);
        hasher.finish()
    }
}

/// The crew on the ropes of a crane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crew {
    /// How many gangs are on the ropes.
    pub gangs: u32,
}

impl Crew {
    /// A crew of so many gangs.
    #[must_use]
    pub const fn new(gangs: u32) -> Self {
        Self { gangs }
    }
}

/// The rate a load rises at: the force on the load over the load's own weight.
///
/// The rate is kept as a fraction of two whole numbers, so no rounding creeps
/// into a Rome rule: the load rises at `num / den` millimetres a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// The force on the load: the pull times the purchase.
    pub num: u32,
    /// The load the rate is against, times the balance divisor.
    pub den: u32,
}

impl Rate {
    /// Millimetres a tick, rounded down.
    #[must_use]
    pub fn mm_per_tick(self) -> u32 {
        if self.den == 0 {
            return 0;
        }
        self.num / self.den
    }

    /// The ticks a lift of `height_mm` takes, rounded up.
    #[must_use]
    pub fn ticks_for(self, height_mm: u32) -> u32 {
        if self.num == 0 {
            return 0;
        }
        let work = u64::from(height_mm).saturating_mul(u64::from(self.den));
        let ticks = work.div_ceil(u64::from(self.num));
        u32::try_from(ticks).unwrap_or(u32::MAX)
    }

    /// True when this rate is faster than `other`.
    #[must_use]
    pub fn faster_than(self, other: Self) -> bool {
        u64::from(self.num).saturating_mul(u64::from(other.den))
            > u64::from(other.num).saturating_mul(u64::from(self.den))
    }
}

/// One lift: what the crane raised, how long it took and who did it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lift {
    /// The machine that did the work.
    pub machine: Machine,
    /// The load that was raised, in kilograms.
    pub load_kg: u32,
    /// The millimetres the load rose.
    pub height_mm: u32,
    /// The ticks the lift took.
    pub ticks: u32,
    /// The rate the load rose at.
    pub rate: Rate,
    /// The gangs on the ropes.
    pub gangs: u32,
    /// The men on the machine: the gangs on the ropes and the men in the wheel.
    pub crew_men: u32,
    /// A digest of the lift: the same lift always digests the same.
    pub hash: u64,
}

impl Lift {
    /// A digest of the lift: the machine, the load, the crew and the time.
    fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/crane/lift/1");
        hasher.write_len(self.machine.entry().len());
        hasher.write_str(self.machine.entry());
        hasher.write_u32(self.load_kg);
        hasher.write_u32(self.height_mm);
        hasher.write_u32(self.ticks);
        hasher.write_u32(self.rate.num);
        hasher.write_u32(self.rate.den);
        hasher.write_u32(self.gangs);
        hasher.write_u32(self.crew_men);
        hasher.finish()
    }
}

/// Why a lift did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiftError {
    /// No gang is on the ropes.
    NoCrew,
    /// The load weighs nothing.
    NoLoad,
    /// The load is over the machine's capacity.
    TooHeavy {
        /// The load that was asked for, in kilograms.
        load_kg: u32,
        /// The most the machine lifts, in kilograms.
        capacity_kg: u32,
    },
}

impl fmt::Display for LiftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCrew => write!(f, "no gang is on the ropes"),
            Self::NoLoad => write!(f, "the load weighs nothing"),
            Self::TooHeavy {
                load_kg,
                capacity_kg,
            } => write!(
                f,
                "the load of {load_kg} kg is over the capacity of {capacity_kg} kg"
            ),
        }
    }
}

impl std::error::Error for LiftError {}

/// One machine's entry in the crane table.
fn rig<'a>(table: &Table<'a>, machine: Machine) -> Result<Rig<'a>, LoadError<'a>> {
    let entry = table
        .entry(machine.entry())
        .ok_or_else(|| LoadError::MissingEntry {
            entry: machine.entry(),
        })?;
    only(
        entry,
        &[
            "pulleys",
            "gangs",
            "men_per_gang",
            "pull_kg_per_gang",
            "capacity_kg",
            "drive",
        ],
    )?;
    let drive = match entry.text("drive").map(str::trim) {
        Some(word) => Drive::parse(word).ok_or_else(|| LoadError::BadValue {
            entry: entry.key(),
            key: "drive",
            value: word,
        })?,
        None => {
            return Err(LoadError::MissingKey {
                entry: entry.key(),
                key: "drive",
            })
        }
    };
    Ok(Rig {
        machine,
        pulleys: positive(entry, "pulleys")?,
        gangs: positive(entry, "gangs")?,
        men_per_gang: positive(entry, "men_per_gang")?,
        pull_kg_per_gang: positive(entry, "pull_kg_per_gang")?,
        capacity_kg: positive(entry, "capacity_kg")?,
        drive,
        source: entry.source(),
    })
}

/// One whole number from a section.
fn number<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LoadError<'a>> {
    let value = int(entry, key)?;
    u32::try_from(value).map_err(|_| LoadError::NumberOverflow {
        entry: entry.key(),
        key,
    })
}

/// One whole number from a section that must be above zero.
fn positive<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LoadError<'a>> {
    let value = number(entry, key)?;
    if value == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key,
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crew the machines are rated for: three gangs on the ropes.
    fn full_crew() -> Crew {
        Crew::new(3)
    }

    /// The shipped crane table.
    fn cranes() -> Cranes<'static> {
        Cranes::shipped().expect("the shipped crane table reads")
    }

    #[test]
    fn the_crane_table_reads_the_two_machines() {
        let cranes = cranes();
        let trispastos = cranes.rig(Machine::Trispastos);
        assert_eq!(trispastos.pulleys, 3, "the trispastos has three pulleys");
        assert_eq!(trispastos.gangs, 3, "three gangs on the ropes");
        assert_eq!(trispastos.drive, Drive::Treadwheel);
        assert!(
            trispastos.source.contains("Vitruvius 10.2"),
            "{}",
            trispastos.source
        );
        let pentaspastos = cranes.rig(Machine::Pentaspastos);
        assert_eq!(pentaspastos.pulleys, 5, "the pentaspastos has five pulleys");
        assert_eq!(pentaspastos.gangs, 3, "three gangs on the ropes");
        assert_eq!(pentaspastos.drive, Drive::Treadwheel);
        assert!(
            pentaspastos.source.contains("Vitruvius 10.2"),
            "{}",
            pentaspastos.source
        );
    }

    #[test]
    fn the_pentaspastos_lifts_the_same_load_faster_than_the_trispastos() {
        let cranes = cranes();
        let trispastos = cranes
            .lift(Machine::Trispastos, full_crew(), 600, 3000)
            .expect("the trispastos lifts the load");
        let pentaspastos = cranes
            .lift(Machine::Pentaspastos, full_crew(), 600, 3000)
            .expect("the pentaspastos lifts the load");
        assert!(
            pentaspastos.rate.faster_than(trispastos.rate),
            "the pentaspastos has the faster rate"
        );
        assert!(
            pentaspastos.ticks < trispastos.ticks,
            "the pentaspastos lifts in fewer ticks"
        );
        assert_eq!(trispastos.rate.mm_per_tick(), 6, "the pull over the load");
        assert_eq!(
            pentaspastos.rate.mm_per_tick(),
            10,
            "the pull over the load"
        );
    }

    #[test]
    fn a_missing_gang_halves_the_speed() {
        let cranes = cranes();
        let full = cranes
            .lift(Machine::Pentaspastos, full_crew(), 600, 3000)
            .expect("the full crew lifts the load");
        let short = cranes
            .lift(Machine::Pentaspastos, Crew::new(2), 600, 3000)
            .expect("two gangs lift the load");
        assert_eq!(short.rate.num, full.rate.num, "the same force on the load");
        assert_eq!(
            u64::from(short.rate.den),
            2 * u64::from(full.rate.den),
            "the balance divisor doubles"
        );
        assert_eq!(
            short.rate.mm_per_tick() * 2,
            full.rate.mm_per_tick(),
            "the speed halves"
        );
        assert_eq!(short.ticks, full.ticks * 2, "the lift takes twice as long");
    }

    #[test]
    fn the_same_lift_gives_the_same_digest_twice() {
        let cranes = cranes();
        let first = cranes
            .lift(Machine::Trispastos, full_crew(), 600, 3000)
            .expect("the first lift");
        let second = cranes
            .lift(Machine::Trispastos, full_crew(), 600, 3000)
            .expect("the second lift");
        assert_eq!(first, second, "the same lift twice");
        assert_eq!(first.hash, second.hash, "and the same digest");
        assert_ne!(first.hash, 0, "the digest is not empty");
    }

    #[test]
    fn two_loads_of_the_crane_table_hash_the_same() {
        assert_eq!(
            cranes().hash(),
            cranes().hash(),
            "two loads of one table agree"
        );
    }

    #[test]
    fn a_load_over_the_capacity_is_refused() {
        let cranes = cranes();
        let error = cranes
            .lift(Machine::Trispastos, full_crew(), 2000, 3000)
            .expect_err("too heavy for the trispastos");
        assert_eq!(
            error,
            LiftError::TooHeavy {
                load_kg: 2000,
                capacity_kg: 1800
            }
        );
        assert!(error.to_string().contains("2000"), "{error}");
        let lift = cranes
            .lift(Machine::Pentaspastos, full_crew(), 2000, 3000)
            .expect("the pentaspastos lifts it");
        assert_eq!(lift.rate.mm_per_tick(), 3, "a heavier load rises slower");
    }

    #[test]
    fn a_crane_with_no_gang_on_the_ropes_cannot_lift() {
        let error = cranes()
            .lift(Machine::Trispastos, Crew::new(0), 600, 3000)
            .expect_err("no gang on the ropes");
        assert_eq!(error, LiftError::NoCrew);
    }

    #[test]
    fn the_lift_counts_the_men_on_the_machine() {
        let lift = cranes()
            .lift(Machine::Trispastos, full_crew(), 600, 3000)
            .expect("the lift");
        assert_eq!(
            lift.crew_men, 36,
            "three gangs of eight and twelve in the wheel"
        );
    }

    #[test]
    fn a_section_with_numbers_and_no_ref_is_refused() {
        let text = "[crane.trispastos]\npulleys = 3\ngangs = 3\n";
        let error = Cranes::parse(text).expect_err("a machine with no source");
        assert!(error.to_string().contains("crane.trispastos"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = Cranes::SHIPPED.replace("pulleys = 3", "pulle = 3");
        let error = Cranes::parse(&text).expect_err("a typo in a key");
        assert!(error.to_string().contains("pulle"), "{error}");
    }

    #[test]
    fn a_drive_we_do_not_know_is_refused() {
        let text = Cranes::SHIPPED.replace("\"treadwheel\"", "\"windlass\"");
        let error = Cranes::parse(&text).expect_err("a drive we do not know");
        assert!(error.to_string().contains("windlass"), "{error}");
    }
}
