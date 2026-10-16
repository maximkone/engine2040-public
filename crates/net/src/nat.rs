//! nat: NAT traversal slice for QUIC through the invite-code relay.
//!
//! Purpose: bind the three pieces a real-NAT leg needs into one plan the
//! QUIC socket adapter (`quic::RelayTransport`) consumes: the invite code
//! (who may join), the relay leg (where bytes ride when the punch falls
//! back) and the road decision (direct or relay). The UDP courier keeps its
//! own rendezvous (`split::rendezvous`); this module reuses its shapes
//! (`courier::RelayLeg`, `courier::Road`, `split::invite_ids`) without
//! editing them, so the LAN proof is untouched.
//! Owner: net lane. Units: ticks at 60 Hz, bytes on the wire, addresses.
//! Determinism: invite mint, leg mapping, candidate order and road decisions
//! are integer-only and seeded; sockets and wall clocks never enter sim
//! hashes. Errors: [`RelayError`]; invalid wire or stale tickets refuse loudly.

use std::net::SocketAddr;

use crate::courier::{RelayLeg, Road};
use crate::relay::{
    decide_join, gather_candidates, Candidate, InviteCode, InviteTicket, NatKind, RelayError,
    Signal,
};
use crate::split::invite_ids;

/// Which side of an invite this machine plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuicRole {
    /// Machine A: prints the code, hosts the bout.
    Host,
    /// Machine B: types the code, joins the bout.
    Join,
}

/// Relay leg for one side of an invite: the relay address plus this
/// machine's id and the other machine's id, derived from the code. Both
/// machines call this with mirrored roles; the ids never collide and never
/// read the wall clock, so the same code always maps the same pair.
#[must_use]
pub fn invite_leg(code: &InviteCode, relay: SocketAddr, token: u64, role: QuicRole) -> RelayLeg {
    let (host_id, join_id) = invite_ids(code);
    let (me, peer) = match role {
        QuicRole::Host => (host_id, join_id),
        QuicRole::Join => (join_id, host_id),
    };
    RelayLeg {
        addr: relay,
        token,
        me,
        peer,
    }
}

/// Mint a code plus its ticket from `rng`; the same seed mints the same
/// code on every machine. Version zero never issues.
pub fn mint_ticket(
    rng: &mut u64,
    version: u16,
    rules: u64,
    now: u32,
) -> Result<(InviteCode, InviteTicket), RelayError> {
    let code = InviteCode::mint(rng);
    let ticket = InviteTicket::issue(code, version, rules, now)?;
    Ok((code, ticket))
}

/// Encode this machine's join request for `ticket`: code plus build
/// identity plus client token. Zero tokens and zero versions fail before
/// any byte is built.
pub fn join_request(
    ticket: &InviteTicket,
    version: u16,
    rules: u64,
    token: u64,
) -> Result<Vec<u8>, RelayError> {
    Signal::Request {
        code: ticket.code.as_bytes(),
        version,
        rules,
        token,
    }
    .encode()
}

/// Host-side invite hook: decode one request off the QUIC reliable lane,
/// decide against `ticket` at `now`, and encode the reply. Unknown codes
/// read as expired, build drift carries the digest pair, junk refuses.
pub fn answer_join(
    ticket: &InviteTicket,
    request: &[u8],
    slot: u8,
    now: u32,
) -> Result<Vec<u8>, RelayError> {
    let Signal::Request {
        code,
        version,
        rules,
        token,
    } = Signal::decode(request)?
    else {
        return Err(RelayError::BadInput);
    };
    decide_join(ticket, code, version, rules, token, slot, now).encode()
}

/// Decode one join reply off the QUIC reliable lane; junk refuses loudly.
pub fn decode_reply(bytes: &[u8]) -> Result<Signal, RelayError> {
    Signal::decode(bytes)
}

/// Slot granted by an accept that echoes `token`; `None` for refuses,
/// token mismatches and junk. The caller treats `None` as not joined.
#[must_use]
pub fn accept_slot(reply: &[u8], token: u64) -> Option<u8> {
    match Signal::decode(reply) {
        Ok(Signal::Accept { token: got, slot }) if got == token => Some(slot),
        _ => None,
    }
}

/// Gather this machine's ICE-lite set for the QUIC punch: host always,
/// server-reflexive when the relay's STUN echo learned one, best first. No
/// relay candidate: the relay leg rides out of band and always works, and a
/// punch probe sent at the relay address would only pollute its junk count.
pub fn gather_quic(
    host: SocketAddr,
    srflx: Option<SocketAddr>,
) -> Result<Vec<Candidate>, RelayError> {
    gather_candidates(host, srflx, None)
}

/// Road after the punch: direct only when both machines confirmed a
/// two-way exchange and this side heard where it came from. Anything less
/// rides the relay, which always completes the join.
#[must_use]
pub fn select_road(
    local_confirmed: bool,
    remote_confirmed: bool,
    heard_from: Option<SocketAddr>,
) -> Road {
    match heard_from {
        Some(_) if local_confirmed && remote_confirmed => Road::Direct,
        _ => Road::Relay,
    }
}

/// Predicted road before probing, from the two NAT shapes: a symmetric NAT
/// on either side never punches through, everything else tries direct
/// first. Mirrors the [`HolePunch`](crate::relay::HolePunch) outcome when
/// checks succeed, so the plan and the punch agree.
#[must_use]
pub fn path_for_nats(local: NatKind, remote: NatKind) -> Road {
    if local == NatKind::Symmetric || remote == NatKind::Symmetric {
        Road::Relay
    } else {
        Road::Direct
    }
}

/// Everything the QUIC socket adapter needs to meet the other machine:
/// its relay leg, its candidates and the predicted road.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NatPlan {
    /// This machine's registration at the relay.
    pub leg: RelayLeg,
    /// This machine's candidates, best first.
    pub candidates: Vec<Candidate>,
    /// Predicted road from the two NAT shapes.
    pub road: Road,
}

/// Compose the plan for one side of an invite: leg from the code, ICE-lite
/// candidates from the addresses, road from the NAT shapes. The only
/// failure is a bad address (port zero); NAT shapes never fail, they pick
/// the relay.
pub fn plan_for(
    code: &InviteCode,
    relay: SocketAddr,
    token: u64,
    role: QuicRole,
    host: SocketAddr,
    srflx: Option<SocketAddr>,
    local_nat: NatKind,
    remote_nat: NatKind,
) -> Result<NatPlan, RelayError> {
    Ok(NatPlan {
        leg: invite_leg(code, relay, token, role),
        candidates: gather_quic(host, srflx)?,
        road: path_for_nats(local_nat, remote_nat),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{RefuseCode, RelayConfig, INVITE_TTL_TICKS};

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn code() -> InviteCode {
        let mut rng = 0x0A11_CE00_1234_5678u64;
        InviteCode::mint(&mut rng)
    }

    #[test]
    fn invite_legs_mirror_per_role() {
        let token = RelayConfig::default().token;
        let host = invite_leg(&code(), addr(7041), token, QuicRole::Host);
        let join = invite_leg(&code(), addr(7041), token, QuicRole::Join);
        assert_ne!(host.me, 0, "relay ids are never zero");
        assert_ne!(host.me, host.peer, "the two machines differ");
        assert_eq!((join.me, join.peer), (host.peer, host.me), "roles mirror");
        assert_eq!(join.addr, host.addr, "both ride the same relay");
        assert_eq!(
            invite_leg(&code(), addr(7041), token, QuicRole::Host),
            host,
            "the same code maps the same leg"
        );
    }

    #[test]
    fn ticket_mint_repeats_per_seed() {
        let (code, ticket) = mint_ticket(&mut 77u64, 3, 0x1234_5678, 100).expect("issues");
        let (twin_code, twin) = mint_ticket(&mut 77u64, 3, 0x1234_5678, 100).expect("issues");
        assert_eq!(
            (twin_code, twin),
            (code, ticket),
            "same seed mints same ticket"
        );
        assert!(ticket.is_live(100 + INVITE_TTL_TICKS));
        assert!(
            mint_ticket(&mut 77u64, 0, 1, 0).is_err(),
            "version zero refuses"
        );
    }

    #[test]
    fn join_roundtrip_accepts_into_slot() {
        let (_, ticket) = mint_ticket(&mut 4242u64, 3, 0x1234_5678, 50).expect("issues");
        let request = join_request(&ticket, 3, 0x1234_5678, 11).expect("request encodes");
        let reply = answer_join(&ticket, &request, 1, 60).expect("host answers");
        assert_eq!(
            decode_reply(&reply),
            Ok(Signal::Accept { token: 11, slot: 1 }),
            "matching builds accept"
        );
        assert_eq!(
            accept_slot(&reply, 11),
            Some(1),
            "echoed token grants the slot"
        );
        assert_eq!(
            accept_slot(&reply, 12),
            None,
            "foreign tokens grant nothing"
        );
    }

    #[test]
    fn join_refuses_with_typed_reasons() {
        let (_, ticket) = mint_ticket(&mut 4242u64, 3, 0x1234_5678, 50).expect("issues");
        let stale = answer_join(
            &ticket,
            &join_request(&ticket, 3, 0x1234_5678, 11).expect("request"),
            1,
            50_000,
        )
        .expect("stale answers");
        assert!(
            matches!(
                decode_reply(&stale),
                Ok(Signal::Refuse {
                    reason: RefuseCode::Expired,
                    ..
                })
            ),
            "past ttl expires"
        );
        assert_eq!(accept_slot(&stale, 11), None, "a refuse grants no slot");
        let drift = answer_join(
            &ticket,
            &join_request(&ticket, 4, 0x1234_5678, 11).expect("request"),
            1,
            60,
        )
        .expect("drift answers");
        match decode_reply(&drift) {
            Ok(Signal::Refuse {
                reason,
                want_version,
                got_version,
                want_rules,
                got_rules,
                ..
            }) => {
                assert_eq!(reason, RefuseCode::VersionMismatch);
                assert_eq!((want_version, got_version), (3, 4));
                assert_eq!((want_rules, got_rules), (0x1234_5678, 0x1234_5678));
            }
            other => panic!("build drift must refuse with digests, got {other:?}"),
        }
        assert!(
            join_request(&ticket, 3, 0x1234_5678, 0).is_err(),
            "zero token refuses"
        );
        assert!(
            answer_join(&ticket, b"junk", 1, 60).is_err(),
            "junk refuses"
        );
        let accept = Signal::Accept { token: 11, slot: 1 }
            .encode()
            .expect("accept encodes");
        assert_eq!(
            answer_join(&ticket, &accept, 1, 60),
            Err(RelayError::BadInput),
            "a reply is not a request"
        );
    }

    #[test]
    fn road_needs_two_confirmations_and_an_address() {
        let heard = Some(addr(6001));
        assert_eq!(select_road(true, true, heard), Road::Direct);
        assert_eq!(select_road(true, false, heard), Road::Relay);
        assert_eq!(select_road(false, true, heard), Road::Relay);
        assert_eq!(select_road(true, true, None), Road::Relay);
        assert_eq!(select_road(false, false, None), Road::Relay);
    }

    #[test]
    fn nat_shapes_predict_the_punch() {
        assert_eq!(path_for_nats(NatKind::Open, NatKind::Open), Road::Direct);
        assert_eq!(path_for_nats(NatKind::Open, NatKind::Cone), Road::Direct);
        assert_eq!(path_for_nats(NatKind::Cone, NatKind::Cone), Road::Direct);
        assert_eq!(
            path_for_nats(NatKind::Open, NatKind::Symmetric),
            Road::Relay
        );
        assert_eq!(
            path_for_nats(NatKind::Symmetric, NatKind::Cone),
            Road::Relay
        );
        assert_eq!(
            path_for_nats(NatKind::Symmetric, NatKind::Symmetric),
            Road::Relay
        );
    }

    #[test]
    fn plan_composes_leg_candidates_and_road() {
        let token = RelayConfig::default().token;
        let plan = plan_for(
            &code(),
            addr(7041),
            token,
            QuicRole::Host,
            addr(5001),
            Some(addr(5002)),
            NatKind::Cone,
            NatKind::Symmetric,
        )
        .expect("valid addrs plan");
        assert_eq!(
            plan.leg,
            invite_leg(&code(), addr(7041), token, QuicRole::Host)
        );
        assert_eq!(plan.candidates.len(), 2, "host plus server-reflexive");
        assert!(plan.candidates[0].priority() > plan.candidates[1].priority());
        assert_eq!(plan.road, Road::Relay, "symmetric remotes predict relay");
        let direct = plan_for(
            &code(),
            addr(7041),
            token,
            QuicRole::Join,
            addr(5003),
            None,
            NatKind::Open,
            NatKind::Cone,
        )
        .expect("valid addrs plan");
        assert_eq!(direct.candidates.len(), 1, "host only without STUN");
        assert_eq!(direct.road, Road::Direct);
        assert!(
            plan_for(
                &code(),
                addr(7041),
                token,
                QuicRole::Host,
                addr(0),
                None,
                NatKind::Open,
                NatKind::Open
            )
            .is_err(),
            "port zero never plans"
        );
    }
}
