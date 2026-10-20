//! TURN allocation discipline: Allocate/Refresh lifetimes, CreatePermission
//! before a forward, and ChannelBind multiplexing on the relay path.
//!
//! Shapes follow TURN (RFC 8656): allocations and channels last ten minutes,
//! permissions five, all in tick counts so replays stay exact. Owner: net lane.
//! Errors: [`RelayError`]; a lapsed grant refuses, never silently forwards.

use std::net::SocketAddr;

use super::{RelayError, MAX_RELAY_PEERS};

/// Allocation lifetime granted when a request names none: 10 minutes.
pub const TURN_ALLOC_TTL_TICKS: u32 = 36_000;
/// Longest allocation lifetime a client may request: one hour.
pub const TURN_ALLOC_MAX_TICKS: u32 = 216_000;
/// Permission lifetime: five minutes, refreshed by a bind or a repeat.
pub const TURN_PERMISSION_TTL_TICKS: u32 = 18_000;
/// Channel lifetime: ten minutes, refreshed by a rebind.
pub const TURN_CHANNEL_TTL_TICKS: u32 = 36_000;
/// First channel number a client may bind.
pub const TURN_CHANNEL_FIRST: u16 = 0x4000;
/// Last channel number a client may bind.
const TURN_CHANNEL_LAST: u16 = 0x7FFF;
/// Permissions and channels one allocation holds at most.
const TURN_MAX_PERMISSIONS: usize = 32;
const TURN_MAX_CHANNELS: usize = 16;

/// Result of a TURN request or gate decision.
type TurnResult<T> = Result<T, RelayError>;

/// One allocation: the relayed port plus the refresh clock bounding it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TurnAllocation {
    /// Client the relayed port belongs to.
    pub client_id: u64,
    /// Port on the relay that stands for this client.
    pub relayed_port: u16,
    /// Tick of the grant or last refresh.
    pub refreshed_at: u32,
    /// Granted lifetime, at most [`TURN_ALLOC_MAX_TICKS`].
    pub ttl_ticks: u32,
}

impl TurnAllocation {
    /// True while `now` is inside `[refreshed_at, refreshed_at + ttl]`.
    pub fn is_live(&self, now: u32) -> bool {
        now.wrapping_sub(self.refreshed_at) <= self.ttl_ticks
    }
}

/// TURN counters: grants, gate permits and refusals by class.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TurnStats {
    /// Allocations granted.
    pub allocated: u64,
    /// Refreshes granted.
    pub refreshed: u64,
    /// Allocations released by a zero refresh.
    pub released: u64,
    /// CreatePermission and ChannelBind grants.
    pub permissions: u64,
    /// Channel binds accepted.
    pub channels: u64,
    /// Forwards the gate permitted.
    pub permitted: u64,
    /// Forwards refused for a missing permission or channel.
    pub refused_unpermitted: u64,
    /// Forwards refused for no live allocation.
    pub refused_stale: u64,
}

/// TURN allocation table and forward gate. Lifetimes are tick counts read
/// from [`TurnAllocator::advance`], never wall clocks, so one call sequence
/// always gives the same permits. One allocation per client, bounded peers.
pub struct TurnAllocator {
    base_port: u16,
    now: u32,
    allocations: Vec<TurnAllocation>,
    permissions: Vec<(u64, SocketAddr, u32)>,
    channels: Vec<(u64, u16, SocketAddr, u32)>,
    stats: TurnStats,
}

impl TurnAllocator {
    /// Open the table; relayed ports start at `base_port` and walk upward.
    pub fn new(base_port: u16) -> TurnResult<Self> {
        if base_port == 0 || u16::MAX - base_port < 64 {
            return Err(RelayError::BadInput);
        }
        Ok(Self {
            base_port,
            now: 0,
            allocations: Vec::new(),
            permissions: Vec::new(),
            channels: Vec::new(),
            stats: TurnStats::default(),
        })
    }

    /// Move the table clock; it never runs backwards.
    pub fn advance(&mut self, now: u32) {
        self.now = self.now.max(now);
    }

    /// Counters so far.
    pub fn stats(&self) -> TurnStats {
        self.stats
    }

    /// The client's live allocation, if any.
    pub fn live(&self, client_id: u64) -> Option<TurnAllocation> {
        self.allocation_at(client_id).map(|at| self.allocations[at])
    }

    /// Allocate a relayed port for `client_id`; a zero ask takes the default.
    pub fn allocate(&mut self, client_id: u64, requested_ttl: u32) -> TurnResult<TurnAllocation> {
        if client_id == 0 {
            return Err(RelayError::BadInput);
        }
        self.purge();
        if self.live(client_id).is_some() {
            return Err(RelayError::SlotFull);
        }
        if self.allocations.len() >= MAX_RELAY_PEERS {
            return Err(RelayError::SlotFull);
        }
        let mut port = self.base_port;
        while self.allocations.iter().any(|a| a.relayed_port == port) {
            port = port.checked_add(1).ok_or(RelayError::SlotFull)?;
        }
        let allocation = TurnAllocation {
            client_id,
            relayed_port: port,
            refreshed_at: self.now,
            ttl_ticks: Self::ttl(requested_ttl),
        };
        self.allocations.push(allocation);
        self.stats.allocated += 1;
        Ok(allocation)
    }

    /// Renew an allocation; zero deletes it (`Ok(None)`). A lapsed or
    /// unknown allocation refuses, so a refresh never revives one.
    pub fn refresh(
        &mut self,
        client_id: u64,
        requested_ttl: u32,
    ) -> TurnResult<Option<TurnAllocation>> {
        self.purge();
        let at = self.allocation_at(client_id).ok_or(RelayError::Expired)?;
        if requested_ttl == 0 {
            self.allocations.remove(at);
            self.permissions
                .retain(|(client, _, _)| *client != client_id);
            self.channels
                .retain(|(client, _, _, _)| *client != client_id);
            self.stats.released += 1;
            return Ok(None);
        }
        self.allocations[at].refreshed_at = self.now;
        self.allocations[at].ttl_ticks = Self::ttl(requested_ttl);
        self.stats.refreshed += 1;
        Ok(Some(self.allocations[at]))
    }

    /// Permit `peer` for the client's allocation; a repeat refreshes the
    /// grant. No live allocation, or a full table, refuses.
    pub fn create_permission(&mut self, client_id: u64, peer: SocketAddr) -> TurnResult<()> {
        self.purge();
        if self.allocation_at(client_id).is_none() {
            return Err(RelayError::Expired);
        }
        if let Some((_, _, granted)) = self
            .permissions
            .iter_mut()
            .find(|(client, addr, _)| *client == client_id && addr.ip() == peer.ip())
        {
            *granted = self.now;
        } else {
            let held = self
                .permissions
                .iter()
                .filter(|(client, _, _)| *client == client_id)
                .count();
            if held >= TURN_MAX_PERMISSIONS {
                return Err(RelayError::SlotFull);
            }
            self.permissions.push((client_id, peer, self.now));
        }
        self.stats.permissions += 1;
        Ok(())
    }

    /// Bind `channel` to `peer`; a bind implies the permission. A channel
    /// never moves to another peer and a peer holds one channel.
    pub fn bind_channel(
        &mut self,
        client_id: u64,
        channel: u16,
        peer: SocketAddr,
    ) -> TurnResult<()> {
        if !(TURN_CHANNEL_FIRST..=TURN_CHANNEL_LAST).contains(&channel) {
            return Err(RelayError::BadInput);
        }
        self.purge();
        if self.allocation_at(client_id).is_none() {
            return Err(RelayError::Expired);
        }
        let existing = self
            .channels
            .iter()
            .position(|(client, number, _, _)| *client == client_id && *number == channel);
        if let Some(at) = existing {
            if self.channels[at].2 != peer {
                return Err(RelayError::BadInput);
            }
        } else if self
            .channels
            .iter()
            .any(|(client, _, bound, _)| *client == client_id && *bound == peer)
        {
            return Err(RelayError::BadInput);
        }
        let held = self
            .channels
            .iter()
            .filter(|(client, _, _, _)| *client == client_id)
            .count();
        if existing.is_none() && held >= TURN_MAX_CHANNELS {
            return Err(RelayError::SlotFull);
        }
        self.create_permission(client_id, peer)?;
        match existing {
            Some(at) => self.channels[at].3 = self.now,
            None => self.channels.push((client_id, channel, peer, self.now)),
        }
        self.stats.channels += 1;
        Ok(())
    }

    /// The forward gate: a live allocation plus a live channel to `dest` or
    /// a permission for its IP. Refusals are counted by class.
    pub fn permit(&mut self, client_id: u64, dest: SocketAddr) -> TurnResult<()> {
        self.purge();
        if self.allocation_at(client_id).is_none() {
            self.stats.refused_stale += 1;
            return Err(RelayError::Expired);
        }
        let now = self.now;
        let channel = self.channels.iter().any(|(client, _, bound, granted)| {
            *client == client_id
                && *bound == dest
                && now.wrapping_sub(*granted) <= TURN_CHANNEL_TTL_TICKS
        });
        let permission = self.permissions.iter().any(|(client, addr, granted)| {
            *client == client_id
                && addr.ip() == dest.ip()
                && now.wrapping_sub(*granted) <= TURN_PERMISSION_TTL_TICKS
        });
        if channel || permission {
            self.stats.permitted += 1;
            Ok(())
        } else {
            self.stats.refused_unpermitted += 1;
            Err(RelayError::Unauthorized)
        }
    }

    fn allocation_at(&self, client_id: u64) -> Option<usize> {
        let now = self.now;
        self.allocations
            .iter()
            .position(|a| a.client_id == client_id && a.is_live(now))
    }

    fn ttl(requested: u32) -> u32 {
        if requested == 0 {
            TURN_ALLOC_TTL_TICKS
        } else {
            requested.min(TURN_ALLOC_MAX_TICKS)
        }
    }

    fn purge(&mut self) {
        let now = self.now;
        let mut at = 0;
        while at < self.allocations.len() {
            if self.allocations[at].is_live(now) {
                at += 1;
            } else {
                let gone = self.allocations.remove(at).client_id;
                self.permissions.retain(|(client, _, _)| *client != gone);
                self.channels.retain(|(client, _, _, _)| *client != gone);
            }
        }
        self.permissions
            .retain(|(_, _, granted)| now.wrapping_sub(*granted) <= TURN_PERMISSION_TTL_TICKS);
        self.channels
            .retain(|(_, _, _, granted)| now.wrapping_sub(*granted) <= TURN_CHANNEL_TTL_TICKS);
    }
}
