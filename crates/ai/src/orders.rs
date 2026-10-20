//! orders: squad orders as validated recorded actions (move, hold, fall in,
//! stack, mirror, wedge, breach, restrain, collect, split red/blue).
//!
//! Owner: this module owns the [`SquadOrder`] vocabulary, its versioned tape
//! bytes and the per-squad replay book. Consumer: the squad brain issues
//! orders, the command menu records them, the headless CQB fixture replays
//! them.
//! Units: integer millimetres for points, ticks at 60 Hz, ids start at 1.
//! Determinism: record order, little-endian bytes and the core FNV-1a64 only;
//! no float, no clock, so the same orders encode and replay the same twice.
//! Errors: [`OrderError`] refuses a bad byte, id or order; a refused push or
//! decode records nothing.
//! Budget: replay and per-order coding stay on the stack (one fixed 9-byte
//! code buffer); only a tape's own record and byte buffers allocate, as cold
//! storage.

use core::fmt;

use crate::legion::MAX_SQUADS;
use engine_core::hash::{hash_bytes, Fnv64};

/// Tape format version; a recorded tape names the layout that wrote it.
pub const TAPE_VERSION: u16 = 1;
/// Largest tape, so one count and one replay stay bounded.
pub const MAX_TAPE_ORDERS: usize = 4096;
/// Bound on one point axis; 32 km covers the largest city tier.
pub const MAX_COORD_MM: i32 = 32_000_000;

/// Tape header: version then order count.
const HEADER_LEN: usize = 4;
/// Longest coded order: the opcode plus both point axes.
const MAX_ORDER_BYTES: usize = 9;

/// Opcodes, fixed for recorded tapes.
const OP_MOVE: u8 = 0;
const OP_HOLD: u8 = 1;
const OP_FALL_IN: u8 = 2;
const OP_STACK: u8 = 3;
const OP_MIRROR: u8 = 4;
const OP_WEDGE: u8 = 5;
const OP_BREACH: u8 = 6;
const OP_RESTRAIN: u8 = 7;
const OP_COLLECT: u8 = 8;
const OP_SPLIT: u8 = 9;

/// A point on the squad grid, in integer millimetres.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PointMm {
    /// World x in millimetres.
    pub x_mm: i32,
    /// World y in millimetres.
    pub y_mm: i32,
}

/// A door id from the world's door table; zero is not an id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DoorId(pub u16);

/// An agent id from the world roster; zero is not an id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AgentId(pub u16);

/// An item id from the world's item table; zero is not an id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemId(pub u16);

/// A device row a breach throws after the door opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceId(pub u8);

impl DeviceId {
    /// Flashbang: stuns and deafens a room.
    pub const BANG: Self = Self(1);
    /// Stinger round: pain and noise without a wound.
    pub const STINGER: Self = Self(2);
    /// Gas: area denial through the doorway, no blast.
    pub const GAS: Self = Self(3);
}

/// Which recorded id a refused order is missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdKind {
    /// A door id.
    Door,
    /// An agent id.
    Agent,
    /// An item id.
    Item,
}

/// Team split target, red or blue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    /// Red element.
    Red,
    /// Blue element.
    Blue,
}

impl Team {
    /// Wire byte (`0` red, `1` blue).
    pub const fn as_u8(self) -> u8 {
        match self {
            Team::Red => 0,
            Team::Blue => 1,
        }
    }

    /// Parse the wire byte; anything else refuses typed.
    ///
    /// Errors: [`OrderError::UnknownTeam`] past `0..=1`.
    pub const fn from_u8(value: u8) -> Result<Self, OrderError> {
        match value {
            0 => Ok(Team::Red),
            1 => Ok(Team::Blue),
            _ => Err(OrderError::UnknownTeam { value }),
        }
    }
}

/// How a breach opens a door.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BreachHow {
    /// Open it quietly by the handle.
    Open,
    /// Kick the latch.
    Kick,
    /// Ram it as a pair.
    Ram,
    /// Shotgun the latch from outside.
    Shotgun,
    /// Blow the frame with a charge.
    Charge,
}

impl BreachHow {
    /// Wire byte (`0` open, `1` kick, `2` ram, `3` shotgun, `4` charge).
    pub const fn as_u8(self) -> u8 {
        match self {
            BreachHow::Open => 0,
            BreachHow::Kick => 1,
            BreachHow::Ram => 2,
            BreachHow::Shotgun => 3,
            BreachHow::Charge => 4,
        }
    }

    /// Parse the wire byte; anything else refuses typed.
    ///
    /// Errors: [`OrderError::UnknownBreachHow`] past `0..=4`.
    pub const fn from_u8(value: u8) -> Result<Self, OrderError> {
        match value {
            0 => Ok(BreachHow::Open),
            1 => Ok(BreachHow::Kick),
            2 => Ok(BreachHow::Ram),
            3 => Ok(BreachHow::Shotgun),
            4 => Ok(BreachHow::Charge),
            _ => Err(OrderError::UnknownBreachHow { value }),
        }
    }
}

/// One order a squad leader issues; a tape records these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SquadOrder {
    /// Move to a point and hold it.
    Move(PointMm),
    /// Hold the current slots; a shield wall braces near the enemy.
    Hold,
    /// Close up on the leader's slot.
    FallIn,
    /// Stack tight on the door's hinge side.
    Stack(DoorId),
    /// Mirror the stack on the far side of the door.
    Mirror(DoorId),
    /// Wedge the squad on the door, tip at the frame.
    Wedge(DoorId),
    /// Breach a door, then optionally throw a device through.
    Breach {
        /// Door to open.
        door: DoorId,
        /// Tool or body used on it.
        how: BreachHow,
        /// Device thrown through afterwards; `None` leaves it empty.
        then: Option<DeviceId>,
    },
    /// Restrain one agent.
    Restrain(AgentId),
    /// Collect one item.
    Collect(ItemId),
    /// Split the squad onto the red or blue element.
    Split(Team),
}

impl SquadOrder {
    /// Check every carried id and point before the order is recorded.
    ///
    /// Units: millimetres for points. Determinism: field tests only, so the
    /// same order always gives the same verdict.
    /// Errors: [`OrderError::ZeroId`] for a missing door, agent or item,
    /// [`OrderError::PointOutOfRange`] past [`MAX_COORD_MM`], and
    /// [`OrderError::UnknownDevice`] for a device row this build never wrote.
    pub fn validate(self) -> Result<(), OrderError> {
        match self {
            SquadOrder::Move(point) => {
                if point.x_mm.unsigned_abs() > MAX_COORD_MM.unsigned_abs()
                    || point.y_mm.unsigned_abs() > MAX_COORD_MM.unsigned_abs()
                {
                    return Err(OrderError::PointOutOfRange {
                        x_mm: point.x_mm,
                        y_mm: point.y_mm,
                    });
                }
                Ok(())
            }
            SquadOrder::Hold | SquadOrder::FallIn | SquadOrder::Split(_) => Ok(()),
            SquadOrder::Stack(door) | SquadOrder::Mirror(door) | SquadOrder::Wedge(door) => {
                check_id(door.0, IdKind::Door)
            }
            SquadOrder::Breach { door, then, .. } => {
                check_id(door.0, IdKind::Door)?;
                match then {
                    None => Ok(()),
                    Some(device) if device_row(device) => Ok(()),
                    Some(device) => Err(OrderError::UnknownDevice { value: device.0 }),
                }
            }
            SquadOrder::Restrain(agent) => check_id(agent.0, IdKind::Agent),
            SquadOrder::Collect(item) => check_id(item.0, IdKind::Item),
        }
    }

    /// Wire opcode for tapes (`0`..=`9`).
    pub const fn as_u8(self) -> u8 {
        match self {
            SquadOrder::Move(_) => OP_MOVE,
            SquadOrder::Hold => OP_HOLD,
            SquadOrder::FallIn => OP_FALL_IN,
            SquadOrder::Stack(_) => OP_STACK,
            SquadOrder::Mirror(_) => OP_MIRROR,
            SquadOrder::Wedge(_) => OP_WEDGE,
            SquadOrder::Breach { .. } => OP_BREACH,
            SquadOrder::Restrain(_) => OP_RESTRAIN,
            SquadOrder::Collect(_) => OP_COLLECT,
            SquadOrder::Split(_) => OP_SPLIT,
        }
    }

    /// Bytes this order takes on the wire, opcode included.
    const fn wire_len(self) -> usize {
        match self {
            SquadOrder::Move(_) => 9,
            SquadOrder::Hold | SquadOrder::FallIn => 1,
            SquadOrder::Stack(_) | SquadOrder::Mirror(_) | SquadOrder::Wedge(_) => 3,
            SquadOrder::Breach { .. } => 5,
            SquadOrder::Restrain(_) | SquadOrder::Collect(_) => 3,
            SquadOrder::Split(_) => 2,
        }
    }

    /// Write the opcode and payload into `out`; returns the bytes used.
    fn encode_into(self, out: &mut [u8; MAX_ORDER_BYTES]) -> usize {
        out[0] = self.as_u8();
        match self {
            SquadOrder::Move(point) => {
                out[1..5].copy_from_slice(&point.x_mm.to_le_bytes());
                out[5..9].copy_from_slice(&point.y_mm.to_le_bytes());
            }
            SquadOrder::Hold | SquadOrder::FallIn => {}
            SquadOrder::Stack(door) | SquadOrder::Mirror(door) | SquadOrder::Wedge(door) => {
                out[1..3].copy_from_slice(&door.0.to_le_bytes());
            }
            SquadOrder::Breach { door, how, then } => {
                out[1..3].copy_from_slice(&door.0.to_le_bytes());
                out[3] = how.as_u8();
                out[4] = match then {
                    None => 0,
                    Some(device) => device.0,
                };
            }
            SquadOrder::Restrain(agent) => {
                out[1..3].copy_from_slice(&agent.0.to_le_bytes());
            }
            SquadOrder::Collect(item) => {
                out[1..3].copy_from_slice(&item.0.to_le_bytes());
            }
            SquadOrder::Split(team) => out[1] = team.as_u8(),
        }
        self.wire_len()
    }
}

/// A device row a breach may throw; [`DeviceId::BANG`] through
/// [`DeviceId::GAS`] are the rows this build writes.
fn device_row(device: DeviceId) -> bool {
    (DeviceId::BANG.0..=DeviceId::GAS.0).contains(&device.0)
}

/// Refuse a zero id; ids start at one.
fn check_id(raw: u16, what: IdKind) -> Result<(), OrderError> {
    if raw == 0 {
        return Err(OrderError::ZeroId { what });
    }
    Ok(())
}

/// One recorded order: tick, squad and the order itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OrderRecord {
    tick: u64,
    squad: u8,
    order: SquadOrder,
}

impl OrderRecord {
    /// Tick the order takes effect.
    pub const fn tick(self) -> u64 {
        self.tick
    }

    /// Squad that receives it.
    pub const fn squad(self) -> u8 {
        self.squad
    }

    /// The order.
    pub const fn order(self) -> SquadOrder {
        self.order
    }
}

/// Refusals from an order, a tape byte or a tape record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderError {
    /// Tape version this build never wrote; the version carried.
    UnknownVersion {
        /// The rejected version.
        got: u16,
    },
    /// Count past [`MAX_TAPE_ORDERS`]; the count carried.
    TooManyOrders {
        /// The rejected count.
        got: usize,
    },
    /// Tape ended inside a field; bytes needed and offered carried.
    Truncated {
        /// Bytes the field needed.
        need: usize,
        /// Bytes the tape held.
        got: usize,
    },
    /// Bytes after the last recorded order; the count carried.
    TrailingBytes {
        /// Bytes past the last order.
        extra: usize,
    },
    /// Opcode outside `0..=9`; the culprit carried.
    UnknownOrder {
        /// The rejected opcode.
        value: u8,
    },
    /// Team byte outside `0..=1`; the culprit carried.
    UnknownTeam {
        /// The rejected team byte.
        value: u8,
    },
    /// Breach byte outside `0..=4`; the culprit carried.
    UnknownBreachHow {
        /// The rejected breach byte.
        value: u8,
    },
    /// Device byte outside `1..=3` (`0` means none); the culprit carried.
    UnknownDevice {
        /// The rejected device byte.
        value: u8,
    },
    /// Squad index past [`MAX_SQUADS`]; the index carried.
    SquadOutOfRange {
        /// The rejected squad.
        squad: u8,
    },
    /// An id of zero, which names nothing; which id carried.
    ZeroId {
        /// The id kind that was zero.
        what: IdKind,
    },
    /// Point axis past [`MAX_COORD_MM`]; both axes carried.
    PointOutOfRange {
        /// X in millimetres.
        x_mm: i32,
        /// Y in millimetres.
        y_mm: i32,
    },
    /// Tick before the record before it; both carried.
    TickOutOfOrder {
        /// Tick of the record before.
        prev: u64,
        /// Tick offered later in the tape.
        got: u64,
    },
    /// Tape already at [`MAX_TAPE_ORDERS`]; the cap carried.
    TapeFull {
        /// Orders a tape accepts.
        cap: usize,
    },
}

impl fmt::Display for OrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderError::UnknownVersion { got } => write!(f, "orders: unknown tape version {got}"),
            OrderError::TooManyOrders { got } => {
                write!(f, "orders: {got} orders past the {MAX_TAPE_ORDERS} cap")
            }
            OrderError::Truncated { need, got } => {
                write!(f, "orders: tape ended at {got} bytes, field needed {need}")
            }
            OrderError::TrailingBytes { extra } => {
                write!(f, "orders: {extra} bytes after the last order")
            }
            OrderError::UnknownOrder { value } => write!(f, "orders: unknown order {value}"),
            OrderError::UnknownTeam { value } => write!(f, "orders: unknown team {value}"),
            OrderError::UnknownBreachHow { value } => write!(f, "orders: unknown breach {value}"),
            OrderError::UnknownDevice { value } => write!(f, "orders: unknown device {value}"),
            OrderError::SquadOutOfRange { squad } => {
                write!(f, "orders: squad {squad} past the {MAX_SQUADS} roster")
            }
            OrderError::ZeroId { what } => {
                let name = match what {
                    IdKind::Door => "door",
                    IdKind::Agent => "agent",
                    IdKind::Item => "item",
                };
                write!(f, "orders: {name} id 0 names nothing")
            }
            OrderError::PointOutOfRange { x_mm, y_mm } => {
                write!(f, "orders: point {x_mm},{y_mm} mm past {MAX_COORD_MM}")
            }
            OrderError::TickOutOfOrder { prev, got } => {
                write!(f, "orders: tick {got} before {prev}")
            }
            OrderError::TapeFull { cap } => write!(f, "orders: tape full at {cap} orders"),
        }
    }
}

impl std::error::Error for OrderError {}

/// What each squad holds after a tape replay.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OrderBook {
    held: [Option<SquadOrder>; MAX_SQUADS],
    applied: usize,
    digest: u64,
}

impl OrderBook {
    fn new() -> Self {
        Self {
            held: [None; MAX_SQUADS],
            applied: 0,
            digest: 0,
        }
    }

    /// Order squad `squad` holds, if the tape ever ordered it.
    pub fn held(&self, squad: u8) -> Option<SquadOrder> {
        self.held.get(usize::from(squad)).copied().flatten()
    }

    /// Records the tape applied.
    pub fn applied(&self) -> usize {
        self.applied
    }

    /// Digest of the final per-squad orders, in squad order.
    ///
    /// Units: one FNV-1a64 word. Determinism: squad order and the wire bytes
    /// of each held order only, so two tapes that end alike agree.
    pub fn digest(&self) -> u64 {
        self.digest
    }
}

/// A versioned order tape: recorded orders in tick order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct OrderTape {
    records: Vec<OrderRecord>,
}

impl OrderTape {
    /// An empty tape.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Record one order after checking it; a refused order records nothing.
    ///
    /// Units: ticks and squad index into [`MAX_SQUADS`].
    /// Errors: [`OrderError::TapeFull`], [`OrderError::SquadOutOfRange`],
    /// [`OrderError::TickOutOfOrder`], plus every [`SquadOrder::validate`]
    /// refusal.
    pub fn push(&mut self, tick: u64, squad: u8, order: SquadOrder) -> Result<(), OrderError> {
        if self.records.len() >= MAX_TAPE_ORDERS {
            return Err(OrderError::TapeFull {
                cap: MAX_TAPE_ORDERS,
            });
        }
        if usize::from(squad) >= MAX_SQUADS {
            return Err(OrderError::SquadOutOfRange { squad });
        }
        if let Some(last) = self.records.last() {
            if tick < last.tick {
                return Err(OrderError::TickOutOfOrder {
                    prev: last.tick,
                    got: tick,
                });
            }
        }
        order.validate()?;
        self.records.push(OrderRecord { tick, squad, order });
        Ok(())
    }

    /// Recorded orders, in tick order.
    pub fn records(&self) -> &[OrderRecord] {
        &self.records
    }

    /// Orders recorded.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when no order is recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Encode the tape: version, count, then one record per order.
    ///
    /// Units: little-endian bytes. Determinism: record order only; encoding
    /// the same tape twice gives the same bytes.
    pub fn encode(&self) -> Vec<u8> {
        // Push caps a tape far under `u16::MAX`, so the count always fits.
        let count = u16::try_from(self.records.len()).unwrap_or(u16::MAX);
        let mut out = Vec::with_capacity(HEADER_LEN + self.records.len() * MAX_ORDER_BYTES);
        out.extend_from_slice(&TAPE_VERSION.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        let mut code = [0u8; MAX_ORDER_BYTES];
        for record in &self.records {
            out.extend_from_slice(&record.tick.to_le_bytes());
            out.push(record.squad);
            let used = record.order.encode_into(&mut code);
            out.extend_from_slice(&code[..used]);
        }
        out
    }

    /// Decode a recorded tape; every bad byte refuses typed.
    ///
    /// Errors: [`OrderError::UnknownVersion`], [`OrderError::TooManyOrders`],
    /// [`OrderError::Truncated`], [`OrderError::TrailingBytes`], plus every
    /// [`OrderTape::push`] refusal.
    pub fn decode(bytes: &[u8]) -> Result<Self, OrderError> {
        let version = read_u16(bytes, 0)?;
        if version != TAPE_VERSION {
            return Err(OrderError::UnknownVersion { got: version });
        }
        let count = usize::from(read_u16(bytes, 2)?);
        if count > MAX_TAPE_ORDERS {
            return Err(OrderError::TooManyOrders { got: count });
        }
        let mut tape = Self::new();
        let mut at = HEADER_LEN;
        for _ in 0..count {
            let tick = read_u64(bytes, at)?;
            let squad = read_u8(bytes, at + 8)?;
            let (order, used) = decode_order(bytes, at + 9)?;
            at += 9 + used;
            tape.push(tick, squad, order)?;
        }
        if at != bytes.len() {
            return Err(OrderError::TrailingBytes {
                extra: bytes.len() - at,
            });
        }
        Ok(tape)
    }

    /// Fold the tape into the order each squad holds.
    ///
    /// Units: one FNV-1a64 word. Determinism: record order for the held
    /// orders, squad order for the digest; the same tape replays the same
    /// twice and touches no heap.
    pub fn replay(&self) -> OrderBook {
        let mut book = OrderBook::new();
        for record in &self.records {
            book.held[usize::from(record.squad)] = Some(record.order);
            book.applied += 1;
        }
        let mut hash = Fnv64::new();
        let mut code = [0u8; MAX_ORDER_BYTES];
        for (slot, squad) in book.held.iter().zip(0u8..) {
            if let Some(order) = *slot {
                hash.write_u8(squad);
                let used = order.encode_into(&mut code);
                hash.write(&code[..used]);
            }
        }
        book.digest = hash.finish();
        book
    }

    /// Digest of the encoded tape: one FNV-1a64 word over its bytes.
    pub fn digest(&self) -> u64 {
        hash_bytes(&self.encode())
    }
}

/// Read `N` bytes at `at`; a short tape refuses with the need and the size.
fn read_array<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], OrderError> {
    match bytes.get(at..at + N) {
        Some(slice) => {
            let mut raw = [0u8; N];
            raw.copy_from_slice(slice);
            Ok(raw)
        }
        None => Err(OrderError::Truncated {
            need: at + N,
            got: bytes.len(),
        }),
    }
}

fn read_u8(bytes: &[u8], at: usize) -> Result<u8, OrderError> {
    match bytes.get(at) {
        Some(byte) => Ok(*byte),
        None => Err(OrderError::Truncated {
            need: at + 1,
            got: bytes.len(),
        }),
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, OrderError> {
    Ok(u16::from_le_bytes(read_array::<2>(bytes, at)?))
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, OrderError> {
    Ok(u64::from_le_bytes(read_array::<8>(bytes, at)?))
}

fn read_i32(bytes: &[u8], at: usize) -> Result<i32, OrderError> {
    Ok(i32::from_le_bytes(read_array::<4>(bytes, at)?))
}

/// Read one order and its payload; returns the order and bytes it took.
fn decode_order(bytes: &[u8], at: usize) -> Result<(SquadOrder, usize), OrderError> {
    let order = match read_u8(bytes, at)? {
        OP_MOVE => SquadOrder::Move(PointMm {
            x_mm: read_i32(bytes, at + 1)?,
            y_mm: read_i32(bytes, at + 5)?,
        }),
        OP_HOLD => SquadOrder::Hold,
        OP_FALL_IN => SquadOrder::FallIn,
        OP_STACK => SquadOrder::Stack(DoorId(read_u16(bytes, at + 1)?)),
        OP_MIRROR => SquadOrder::Mirror(DoorId(read_u16(bytes, at + 1)?)),
        OP_WEDGE => SquadOrder::Wedge(DoorId(read_u16(bytes, at + 1)?)),
        OP_BREACH => {
            let door = DoorId(read_u16(bytes, at + 1)?);
            let how = BreachHow::from_u8(read_u8(bytes, at + 3)?)?;
            let then = match read_u8(bytes, at + 4)? {
                0 => None,
                value => Some(DeviceId(value)),
            };
            SquadOrder::Breach { door, how, then }
        }
        OP_RESTRAIN => SquadOrder::Restrain(AgentId(read_u16(bytes, at + 1)?)),
        OP_COLLECT => SquadOrder::Collect(ItemId(read_u16(bytes, at + 1)?)),
        OP_SPLIT => SquadOrder::Split(Team::from_u8(read_u8(bytes, at + 1)?)?),
        value => return Err(OrderError::UnknownOrder { value }),
    };
    Ok((order, order.wire_len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn full_tape_rows() -> Vec<(u64, u8, SquadOrder)> {
        let door = DoorId(7);
        let mut rows = Vec::new();
        let mut tick = 0u64;
        let mut add = |squad: u8, order: SquadOrder| {
            rows.push((tick, squad, order));
            tick += 1;
        };
        add(
            0,
            SquadOrder::Move(PointMm {
                x_mm: 12_345,
                y_mm: -6_789,
            }),
        );
        add(1, SquadOrder::Hold);
        add(2, SquadOrder::FallIn);
        add(3, SquadOrder::Stack(door));
        add(4, SquadOrder::Mirror(door));
        add(5, SquadOrder::Wedge(door));
        add(
            6,
            SquadOrder::Breach {
                door,
                how: BreachHow::Open,
                then: None,
            },
        );
        add(
            6,
            SquadOrder::Breach {
                door,
                how: BreachHow::Kick,
                then: Some(DeviceId::BANG),
            },
        );
        add(
            6,
            SquadOrder::Breach {
                door,
                how: BreachHow::Ram,
                then: Some(DeviceId::STINGER),
            },
        );
        add(
            6,
            SquadOrder::Breach {
                door,
                how: BreachHow::Shotgun,
                then: Some(DeviceId::GAS),
            },
        );
        add(
            6,
            SquadOrder::Breach {
                door,
                how: BreachHow::Charge,
                then: None,
            },
        );
        add(7, SquadOrder::Restrain(AgentId(3)));
        add(8, SquadOrder::Collect(ItemId(9)));
        add(9, SquadOrder::Split(Team::Red));
        add(10, SquadOrder::Split(Team::Blue));
        rows
    }

    fn tape_from(rows: &[(u64, u8, SquadOrder)]) -> OrderTape {
        let mut tape = OrderTape::new();
        for &(tick, squad, order) in rows {
            tape.push(tick, squad, order).expect("valid order rows");
        }
        tape
    }

    fn one(order: SquadOrder) -> Vec<u8> {
        tape_from(&[(5, 0, order)]).encode()
    }

    #[test]
    fn recorded_tape_replays_x2_from_the_same_bytes() {
        let rows = full_tape_rows();
        let a = tape_from(&rows);
        let b = tape_from(&rows);
        let bytes = a.encode();
        assert_eq!(bytes, b.encode(), "same orders encode the same bytes twice");
        assert_eq!(a.encode(), bytes, "encoding twice is byte-identical");
        assert_eq!(a.digest(), b.digest());
        let back = OrderTape::decode(&bytes).expect("a recorded tape decodes");
        assert_eq!(
            back.records(),
            a.records(),
            "decode returns the same orders"
        );
        assert_eq!(back.encode(), bytes, "a decoded tape re-encodes the same");
        let first = a.replay();
        let second = a.replay();
        assert_eq!(first, second, "replay x2 is identical");
        assert_eq!(first.applied(), rows.len());
        let (first_squad, first_order) = (rows[0].1, rows[0].2);
        assert_eq!(first.held(first_squad), Some(first_order));
        let (last_squad, last_order) = (rows[rows.len() - 1].1, rows[rows.len() - 1].2);
        assert_eq!(first.held(last_squad), Some(last_order));
        assert_eq!(first.held(42), None, "a squad never ordered holds nothing");
    }

    #[test]
    fn replay_holds_the_last_order_per_squad() {
        let door = DoorId(2);
        let a = tape_from(&[
            (0, 0, SquadOrder::Move(PointMm { x_mm: 0, y_mm: 0 })),
            (1, 0, SquadOrder::Stack(door)),
            (2, 1, SquadOrder::Hold),
        ]);
        let book = a.replay();
        assert_eq!(book.applied(), 3);
        assert_eq!(book.held(0), Some(SquadOrder::Stack(door)));
        assert_eq!(book.held(1), Some(SquadOrder::Hold));
        assert_eq!(book.held(2), None);
        // A different tape that ends on the same orders keeps the same state
        // digest, while the tape digest moves with the records.
        let b = tape_from(&[
            (0, 0, SquadOrder::Hold),
            (1, 0, SquadOrder::Stack(door)),
            (2, 1, SquadOrder::Hold),
        ]);
        assert_eq!(a.replay().digest(), b.replay().digest());
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn wire_bytes_stay_fixed_for_recorded_tapes() {
        // A recorded tape outlives the build that wrote it: the version and
        // the record layout are a contract, so pin the bytes.
        assert_eq!(TAPE_VERSION, 1);
        let bytes = one(SquadOrder::Stack(DoorId(0x0102)));
        assert_eq!(
            bytes,
            vec![1, 0, 1, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 3, 2, 1],
            "version 1, one order, tick 5, squad 0, stack, door 0x0102"
        );
        let empty = OrderTape::new();
        assert_eq!(empty.encode(), vec![1, 0, 0, 0]);
        assert_eq!(OrderTape::decode(&[1, 0, 0, 0]).expect("empty tape"), empty);
    }

    #[test]
    fn breach_then_names_none_bang_stinger_gas() {
        let cases = [
            (None, 0u8),
            (Some(DeviceId::BANG), 1),
            (Some(DeviceId::STINGER), 2),
            (Some(DeviceId::GAS), 3),
        ];
        for (then, byte) in cases {
            let order = SquadOrder::Breach {
                door: DoorId(9),
                how: BreachHow::Charge,
                then,
            };
            let bytes = one(order);
            assert_eq!(bytes[17], byte, "the follow-up device byte");
            let back = OrderTape::decode(&bytes).expect("breach tape decodes");
            assert_eq!(back.records()[0].order(), order);
        }
    }

    #[test]
    fn bad_tape_bytes_refuse_typed() {
        let good = one(SquadOrder::Stack(DoorId(0x0102)));
        assert!(OrderTape::decode(&good).is_ok());

        let mut version = good.clone();
        version[0] = 2;
        assert_eq!(
            OrderTape::decode(&version),
            Err(OrderError::UnknownVersion { got: 2 })
        );

        let mut count = good.clone();
        let over = u16::try_from(MAX_TAPE_ORDERS + 1).expect("cap fits u16");
        count[2..4].copy_from_slice(&over.to_le_bytes());
        assert_eq!(
            OrderTape::decode(&count),
            Err(OrderError::TooManyOrders {
                got: MAX_TAPE_ORDERS + 1
            })
        );

        assert_eq!(
            OrderTape::decode(&good[..2]),
            Err(OrderError::Truncated { need: 4, got: 2 })
        );
        let mut short = good.clone();
        short.pop();
        assert_eq!(
            OrderTape::decode(&short),
            Err(OrderError::Truncated {
                need: good.len(),
                got: short.len()
            })
        );
        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(
            OrderTape::decode(&extra),
            Err(OrderError::TrailingBytes { extra: 1 })
        );

        let mut op = one(SquadOrder::Hold);
        op[13] = 10;
        assert_eq!(
            OrderTape::decode(&op),
            Err(OrderError::UnknownOrder { value: 10 })
        );

        let mut team = one(SquadOrder::Split(Team::Red));
        team[14] = 2;
        assert_eq!(
            OrderTape::decode(&team),
            Err(OrderError::UnknownTeam { value: 2 })
        );

        let mut how = one(SquadOrder::Breach {
            door: DoorId(3),
            how: BreachHow::Open,
            then: None,
        });
        how[16] = 5;
        assert_eq!(
            OrderTape::decode(&how),
            Err(OrderError::UnknownBreachHow { value: 5 })
        );

        let mut device = one(SquadOrder::Breach {
            door: DoorId(3),
            how: BreachHow::Open,
            then: None,
        });
        device[17] = 4;
        assert_eq!(
            OrderTape::decode(&device),
            Err(OrderError::UnknownDevice { value: 4 })
        );

        let mut door = one(SquadOrder::Stack(DoorId(0x0102)));
        door[14] = 0;
        door[15] = 0;
        assert_eq!(
            OrderTape::decode(&door),
            Err(OrderError::ZeroId { what: IdKind::Door })
        );

        let mut agent = one(SquadOrder::Restrain(AgentId(3)));
        agent[14] = 0;
        agent[15] = 0;
        assert_eq!(
            OrderTape::decode(&agent),
            Err(OrderError::ZeroId {
                what: IdKind::Agent
            })
        );

        let mut item = one(SquadOrder::Collect(ItemId(3)));
        item[14] = 0;
        item[15] = 0;
        assert_eq!(
            OrderTape::decode(&item),
            Err(OrderError::ZeroId { what: IdKind::Item })
        );

        let mut point = one(SquadOrder::Move(PointMm { x_mm: 0, y_mm: 0 }));
        point[14..18].copy_from_slice(&(MAX_COORD_MM + 1).to_le_bytes());
        assert_eq!(
            OrderTape::decode(&point),
            Err(OrderError::PointOutOfRange {
                x_mm: MAX_COORD_MM + 1,
                y_mm: 0
            })
        );

        let mut squad = one(SquadOrder::Hold);
        squad[12] = u8::try_from(MAX_SQUADS).expect("squad cap fits u8");
        assert_eq!(
            OrderTape::decode(&squad),
            Err(OrderError::SquadOutOfRange {
                squad: u8::try_from(MAX_SQUADS).expect("squad cap fits u8")
            })
        );

        let mut late = one(SquadOrder::Hold);
        late[2..4].copy_from_slice(&2u16.to_le_bytes());
        late.extend_from_slice(&4u64.to_le_bytes());
        late.push(0);
        late.push(SquadOrder::Hold.as_u8());
        assert_eq!(
            OrderTape::decode(&late),
            Err(OrderError::TickOutOfOrder { prev: 5, got: 4 })
        );
    }

    #[test]
    fn push_refuses_bad_orders_and_records_nothing() {
        let mut tape = OrderTape::new();
        assert_eq!(
            tape.push(0, 0, SquadOrder::Stack(DoorId(0))),
            Err(OrderError::ZeroId { what: IdKind::Door })
        );
        assert_eq!(
            tape.push(0, 64, SquadOrder::Hold),
            Err(OrderError::SquadOutOfRange { squad: 64 })
        );
        assert_eq!(
            tape.push(
                0,
                0,
                SquadOrder::Move(PointMm {
                    x_mm: 0,
                    y_mm: MAX_COORD_MM + 1
                })
            ),
            Err(OrderError::PointOutOfRange {
                x_mm: 0,
                y_mm: MAX_COORD_MM + 1
            })
        );
        for then in [
            Some(DeviceId(0)),
            Some(DeviceId(4)),
            Some(DeviceId(u8::MAX)),
        ] {
            assert_eq!(
                tape.push(
                    0,
                    0,
                    SquadOrder::Breach {
                        door: DoorId(1),
                        how: BreachHow::Open,
                        then
                    }
                ),
                Err(OrderError::UnknownDevice {
                    value: then.expect("a device was named").0
                })
            );
        }
        assert!(tape.is_empty(), "a refused order records nothing");
        tape.push(10, 0, SquadOrder::Hold).expect("valid order");
        assert_eq!(
            tape.push(9, 1, SquadOrder::Hold),
            Err(OrderError::TickOutOfOrder { prev: 10, got: 9 })
        );
        assert_eq!(tape.len(), 1);
        assert_eq!(tape.records()[0].tick(), 10);
        assert_eq!(tape.records()[0].squad(), 0);
        assert_eq!(tape.records()[0].order(), SquadOrder::Hold);
    }

    #[test]
    fn full_tape_refuses_past_the_cap() {
        let mut tape = OrderTape::new();
        for tick in 0..MAX_TAPE_ORDERS {
            tape.push(tick as u64, 0, SquadOrder::Hold)
                .expect("orders up to the cap");
        }
        assert_eq!(
            tape.push(MAX_TAPE_ORDERS as u64, 0, SquadOrder::Hold),
            Err(OrderError::TapeFull {
                cap: MAX_TAPE_ORDERS
            })
        );
        assert_eq!(tape.len(), MAX_TAPE_ORDERS);
    }

    #[test]
    fn replay_allocates_nothing() {
        let tape = tape_from(&full_tape_rows());
        let (book, count) = measure(|| tape.replay());
        assert_eq!(count.calls, 0, "a replay must not touch the heap");
        assert_eq!(book.applied(), full_tape_rows().len());
    }
}
