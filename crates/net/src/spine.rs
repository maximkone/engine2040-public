//! spine: rejoin resim over `Spine::snapshot` bytes.
//!
//! Consumer: a client that dropped and rejoins the match. [`SpineRollback`](crate::spine::SpineRollback)
//! keeps bounded authoritative checkpoints in canonical spine snapshot
//! bytes, restores the drop tick and replays the authoritative suffix the
//! server sends (or loads the drop tick back from a save payload). Data
//! owner: the caller owns the rollback and the [`Spine`](engine_ecs::spine::Spine); `engine-net` only
//! orders the bytes. Units: ticks (`u64`), bytes. Determinism: the same
//! spine gives the same snapshot bytes, and convergence is byte equality
//! (`save::rollback::converges`). Errors: [`SpineRollbackError`](crate::spine::SpineRollbackError).

use engine_ecs::spine::{Spine, SpineError};
use engine_save::persist::{self, PersistErr};
use engine_save::rollback::{self, RollbackErr, RollbackRing};

use crate::netcode::Channel;
use crate::quic::{send_on_channel, ControlKind, PeerId, Transport};
use crate::session::NetError;

/// Send one spine rollback frame (`save::rollback::encode_frame` bytes) on
/// the reliable snapshot channel: QUIC rides an ordered stream, while the
/// UDP transport keeps its documented datagram downgrade. The rejoin
/// catch-up is the one caller (ARC5).
pub fn send_spine_frame<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    frame: &[u8],
) -> Result<(), NetError> {
    send_on_channel(
        transport,
        to,
        Channel::Reliable,
        ControlKind::Snapshot,
        frame,
    )
}

/// Why one spine checkpoint was refused. Nothing is forged on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpineRollbackError {
    /// The spine bytes could not be captured or restored.
    Spine(SpineError),
    /// The rollback frame was refused (version, length or snapshot gate).
    Frame(RollbackErr),
    /// The save payload was refused (version, length or snapshot gate).
    Persist(PersistErr),
}

impl std::fmt::Display for SpineRollbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spine(err) => write!(f, "spine: {err}"),
            Self::Frame(err) => write!(f, "spine frame: {err}"),
            Self::Persist(err) => write!(f, "spine payload: {err}"),
        }
    }
}

impl std::error::Error for SpineRollbackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spine(err) => Some(err),
            Self::Frame(err) => Some(err),
            Self::Persist(err) => Some(err),
        }
    }
}

impl From<SpineError> for SpineRollbackError {
    fn from(err: SpineError) -> Self {
        Self::Spine(err)
    }
}

impl From<RollbackErr> for SpineRollbackError {
    fn from(err: RollbackErr) -> Self {
        Self::Frame(err)
    }
}

impl From<PersistErr> for SpineRollbackError {
    fn from(err: PersistErr) -> Self {
        Self::Persist(err)
    }
}

/// Bounded spine checkpoints plus the wire and save codecs the rejoin uses.
///
/// The ring stores the exact `Spine::snapshot` bytes of each captured tick;
/// [`Self::restore`] is the only reader that decodes them back into a live
/// spine, so a rejoin never guesses. Save payloads ([`Self::save_payload`])
/// carry the same bytes through the versioned save envelope.
#[derive(Clone, Debug)]
pub struct SpineRollback {
    ring: RollbackRing,
}

impl SpineRollback {
    /// Empty rollback holding at most `cap` checkpoints (clamped by the
    /// save ring to `1..=128`).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            ring: RollbackRing::new(cap),
        }
    }

    /// Stored checkpoint count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when no checkpoint is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Capture the live spine at `tick` as its canonical snapshot bytes.
    pub fn capture(&mut self, tick: u64, spine: &Spine) -> Result<(), SpineRollbackError> {
        let snapshot = spine.snapshot();
        self.ring.push(tick, &snapshot.bytes)?;
        Ok(())
    }

    /// Stored snapshot bytes at `tick` (the convergence target).
    pub fn bytes(&self, tick: u64) -> Result<&[u8], SpineRollbackError> {
        Ok(self.ring.get(tick)?)
    }

    /// Encode the checkpoint at `tick` as one rollback frame for the wire.
    pub fn frame(&self, tick: u64) -> Result<Vec<u8>, SpineRollbackError> {
        Ok(rollback::encode_frame(tick, self.ring.get(tick)?)?)
    }

    /// Decode one rollback frame and store its checkpoint; answers the tick.
    pub fn apply_frame(&mut self, frame: &[u8]) -> Result<u64, SpineRollbackError> {
        let decoded = rollback::decode_frame(frame)?;
        self.ring.push(decoded.tick, &decoded.spine)?;
        Ok(decoded.tick)
    }

    /// Encode the checkpoint at `tick` as a save payload.
    pub fn save_payload(&self, seed: u64, tick: u64) -> Result<Vec<u8>, SpineRollbackError> {
        Ok(persist::seal_spine(seed, tick, self.ring.get(tick)?)?)
    }

    /// Decode one save payload, store its checkpoint and answer its tick.
    pub fn load_payload(&mut self, payload: &[u8]) -> Result<u64, SpineRollbackError> {
        let saved = persist::open_spine(payload)?;
        self.ring.push(saved.tick, &saved.spine)?;
        Ok(saved.tick)
    }

    /// Restore the live spine captured at `tick`.
    pub fn restore(&self, tick: u64) -> Result<Spine, SpineRollbackError> {
        Ok(Spine::restore(self.ring.get(tick)?)?)
    }

    /// True when the checkpoint at `tick` equals `spine` byte for byte.
    pub fn converges_with(&self, tick: u64, spine: &Spine) -> Result<bool, SpineRollbackError> {
        let snapshot = spine.snapshot();
        Ok(rollback::converges(self.ring.get(tick)?, &snapshot.bytes))
    }

    /// Rejoin resim: restore the checkpoint at `from`, then replay every
    /// authoritative suffix frame in order. Each frame is a full snapshot,
    /// so a replay is the ring's restore step at its tick; answers the spine
    /// after the last frame. An empty suffix answers the `from` checkpoint.
    pub fn rejoin(&mut self, from: u64, suffix: &[Vec<u8>]) -> Result<Spine, SpineRollbackError> {
        let mut latest = from;
        for frame in suffix {
            latest = self.apply_frame(frame)?;
        }
        self.restore(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ecs::spine::{Body, Transform, Vitals};

    fn spine_at(tick: u64) -> Spine {
        let mut spine = Spine::with_capacity(2);
        for slot in 0..2 {
            let offset = f32::from(i16::try_from(tick).expect("test ticks fit i16"));
            spine
                .spawn(
                    Transform {
                        pos: [
                            offset + f32::from(i16::try_from(slot).expect("slot fits")),
                            0.0,
                            0.0,
                        ],
                        yaw: offset,
                    },
                    Body::default(),
                    Vitals {
                        hp_permille: 1000 - u16::try_from(tick).expect("test ticks fit u16"),
                        ..Vitals::default()
                    },
                )
                .expect("fixture has room");
        }
        spine
    }

    #[test]
    fn drop_rejoin_restores_and_replays_spine_bytes() {
        let mut server = SpineRollback::new(32);
        let mut client = SpineRollback::new(32);
        for tick in 0..10u64 {
            let spine = spine_at(tick);
            server.capture(tick, &spine).expect("server captures");
            if tick < 4 {
                client
                    .apply_frame(&server.frame(tick).expect("frame"))
                    .expect("client stores the pre-drop ticks");
            }
        }
        assert!(client.bytes(4).is_err(), "the drop leaves a gap");
        let suffix: Vec<Vec<u8>> = (4..10u64)
            .map(|tick| server.frame(tick).expect("frame"))
            .collect();
        let tip = client.rejoin(4, &suffix).expect("rejoin replays");
        assert_eq!(tip.snapshot().bytes, spine_at(9).snapshot().bytes);
        for tick in 0..10u64 {
            assert!(
                client
                    .converges_with(tick, &server.restore(tick).expect("server restores"))
                    .expect("client has the tick"),
                "tick {tick} converges"
            );
        }
        assert!(server.converges_with(9, &tip).expect("tip compares"));
    }

    #[test]
    fn save_payload_roundtrip_x2_carries_the_spine() {
        let mut ring = SpineRollback::new(8);
        let spine = spine_at(600);
        ring.capture(600, &spine).expect("captures");
        let (a, b) = (
            ring.save_payload(2040, 600).expect("seals"),
            ring.save_payload(2040, 600).expect("seals"),
        );
        assert_eq!(a, b, "same checkpoint seals the same bytes twice");
        let mut back = SpineRollback::new(8);
        assert_eq!(back.load_payload(&a).expect("opens"), 600);
        let restored = back.restore(600).expect("restores");
        assert_eq!(restored.snapshot(), spine.snapshot());
        assert!(back.converges_with(600, &spine).expect("compares"));
    }

    #[test]
    fn bad_bytes_refuse_without_change() {
        let mut ring = SpineRollback::new(4);
        assert_eq!(
            ring.apply_frame(&[]),
            Err(SpineRollbackError::Frame(RollbackErr::Truncated))
        );
        assert_eq!(
            ring.apply_frame(&[rollback::ROLLBACK_SCHEMA_VERSION + 1; 13]),
            Err(SpineRollbackError::Frame(RollbackErr::BadVersion))
        );
        assert_eq!(
            ring.load_payload(&[]),
            Err(SpineRollbackError::Persist(PersistErr::Truncated))
        );
        assert!(ring.is_empty(), "a refused frame stores nothing");
    }
}
