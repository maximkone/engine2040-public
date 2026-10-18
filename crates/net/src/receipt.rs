//! receipt: the played proofs' posted evidence as JSON.
//!
//! Purpose: one text form for the numbers the two-machine proofs post: the
//! bout verdict, checkpoint equality, the drop and rejoin ledger, the loss and
//! latency distributions, the soak hashes and the fault matrix. The
//! `net-bout` binary writes these files under `target/mp6` (and `target/on30`)
//! where `tools/mp6-check.mjs` finds them. Owner: net lane.
//! Units: ticks at 60 Hz, whole milliseconds, basis points, bytes.
//! Determinism: every field is an integer or a hex digest taken from a
//! deterministic report; wall-clock resim evidence is left out. Errors: none,
//! rendering is total.

use std::fmt::{Display, Write};

use crate::dedicated::ticks_to_ms;
use crate::link::{LinkReport, LinkStats, Peer};
use crate::played::{BoutReport, SeatReport};
use crate::scale::SoakReport;
use crate::wan_soak::FaultReport;

/// Tiny ordered JSON object builder: `{ "k": v, ... }`, one key per line.
#[derive(Default)]
pub struct Obj {
    fields: Vec<String>,
}

impl Obj {
    /// Empty object.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a number or any value whose `Display` is a JSON number.
    pub fn num(&mut self, key: &str, value: impl Display) -> &mut Self {
        self.fields.push(format!("\"{key}\": {value}"));
        self
    }

    /// Add a boolean.
    pub fn flag(&mut self, key: &str, value: bool) -> &mut Self {
        self.num(key, value)
    }

    /// Add a string; quotes and backslashes escape, control bytes drop.
    pub fn text(&mut self, key: &str, value: &str) -> &mut Self {
        let mut safe = String::with_capacity(value.len());
        for c in value.chars() {
            match c {
                '"' | '\\' => {
                    safe.push('\\');
                    safe.push(c);
                }
                c if c.is_control() => {}
                c => safe.push(c),
            }
        }
        self.fields.push(format!("\"{key}\": \"{safe}\""));
        self
    }

    /// Add a 64-bit digest as a 16-digit lowercase hex string.
    pub fn hex(&mut self, key: &str, value: u64) -> &mut Self {
        self.text(key, &format!("{value:016x}"))
    }

    /// Add an optional number: `null` when absent.
    pub fn opt(&mut self, key: &str, value: Option<u32>) -> &mut Self {
        match value {
            Some(v) => self.num(key, v),
            None => {
                self.fields.push(format!("\"{key}\": null"));
                self
            }
        }
    }

    /// Add an already rendered JSON value.
    pub fn raw(&mut self, key: &str, json: String) -> &mut Self {
        self.fields.push(format!("\"{key}\": {json}"));
        self
    }

    /// Render the object.
    #[must_use]
    pub fn finish(&self) -> String {
        format!("{{\n  {}\n}}\n", self.fields.join(",\n  "))
    }

    /// Render the object on one line, for nesting inside arrays.
    #[must_use]
    pub fn inline(&self) -> String {
        format!("{{ {} }}", self.fields.join(", "))
    }
}

/// Render a slice of numbers as a JSON array.
fn numbers<T: Display>(items: &[T]) -> String {
    let mut out = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{item}");
    }
    out.push(']');
    out
}

/// Render rendered values as a JSON array, one per line.
#[must_use]
pub fn lines(items: Vec<String>) -> String {
    if items.is_empty() {
        return String::from("[]");
    }
    format!("[\n    {}\n  ]", items.join(",\n    "))
}

fn peer_label(peer: Peer) -> String {
    match peer {
        Peer::Server => String::from("server"),
        Peer::Client(seat) => format!("client{seat}"),
    }
}

/// The bout receipt: shape, verdict and the digests two runs must share.
#[must_use]
pub fn bout_json(label: &str, report: &BoutReport, repeat: Option<&BoutReport>) -> String {
    let mut o = Obj::new();
    o.text("profile", label)
        .num("seed", report.config.seed)
        .num("fighters", report.config.fighters)
        .num("humans", report.humans)
        .num("ai", report.ai)
        .num("ticks", report.config.ticks)
        .num("hold_ticks", report.config.hold)
        .num("input_tape_len", report.tape_len)
        .hex("input_tape_hash", report.tape_hash)
        .hex("server_tip_hash", report.server_tip.hash)
        .flag("tape_replays_to_tip", report.replay_equal)
        .flag("converged", report.converged())
        .hex("digest", report.digest());
    if let Some(twin) = repeat {
        o.hex("digest_run2", twin.digest())
            .flag("hash_x2_identical", twin.digest() == report.digest());
    }
    o.raw("fight_events", {
        let mut f = Obj::new();
        f.num("before_drop", report.fight.before_drop)
            .num("during_drop", report.fight.during_drop)
            .num("after_rejoin", report.fight.after_rejoin)
            .raw("per_second", numbers(&report.fight.per_second));
        f.inline()
    });
    o.finish()
}

fn seat_json(seat: &SeatReport, server_tip: u64) -> String {
    let mut o = Obj::new();
    o.num("slot", seat.slot)
        .hex("tip_hash", seat.tip.hash)
        .flag("equals_server_tip", seat.tip.hash == server_tip)
        .flag("checkpoints_equal_full_length", seat.equal)
        .opt("first_divergence", seat.first_divergence)
        .num("corrected_ticks", seat.mispredict_ticks)
        .num("states_applied", seat.stats.applied);
    o.inline()
}

/// Checkpoint equality: server tip, per-seat verdicts and every 60th
/// checkpoint of the server and both clients side by side.
#[must_use]
pub fn checkpoints_json(label: &str, report: &BoutReport) -> String {
    let mut o = Obj::new();
    o.text("profile", label)
        .num("ticks", report.checkpoints.len())
        .hex("server_tip_hash", report.server_tip.hash)
        .raw(
            "seats",
            lines(
                report
                    .seats
                    .iter()
                    .map(|seat| seat_json(seat, report.server_tip.hash))
                    .collect(),
            ),
        )
        .raw(
            "every_60_ticks",
            lines(
                report
                    .checkpoints
                    .iter()
                    .filter(|cp| (cp.tick + 1).is_multiple_of(60))
                    .map(|cp| {
                        let mut row = Obj::new();
                        row.num("tick", cp.tick).hex("server", cp.hash);
                        row.inline()
                    })
                    .collect(),
            ),
        );
    o.finish()
}

/// The scripted drop and rejoin: what the stall watch, the supervisor, the
/// AI takeover and the catch-up stream did, and how long the way back took.
#[must_use]
pub fn drop_json(label: &str, report: &BoutReport) -> String {
    let drop = report.config.drop;
    let host = &report.host;
    let mut o = Obj::new();
    o.text("profile", label)
        .num("seat", drop.seat)
        .num("link_dark_at", drop.at)
        .num("link_back_at", drop.back)
        .opt("stalled_out_at_tick", host.stalled_at)
        .num("reclaim_not_before_tick", host.not_before)
        .opt("reclaimed_at_step", host.reclaimed_at)
        .num("ai_cover_ticks", host.ai_cover_ticks)
        .num("ai_slots_normal", host.ai_base)
        .num("ai_slots_peak", host.ai_peak)
        .num("catchup_packets", host.catchup_packets)
        .num("catchup_bytes", host.catchup_bytes)
        .opt("rejoin_ticks", report.rejoin_ticks())
        .num("heals_applied", host.heals_applied)
        .num("join_refusals", host.refused)
        .flag(
            "granted",
            report.seats[usize::from(drop.seat)].stats.granted,
        )
        .flag("fight_live_at_both_ends", report.mid_fight())
        .flag("converged_mid_fight", report.converged());
    o.finish()
}

fn link_json(link: &LinkReport) -> String {
    let s: &LinkStats = &link.stats;
    let mut o = Obj::new();
    o.text("from", &peer_label(link.from))
        .text("to", &peer_label(link.to))
        .num("datagrams_sent", s.sent)
        .num("datagrams_dropped", s.dropped)
        .num("datagrams_lost_to_outage", s.outage)
        .num("random_loss_bp", s.loss_bp())
        .num("reordered", s.reordered)
        .num("streams", s.streams)
        .num("bytes", s.bytes)
        .num("delay_p50_ticks", s.delay_pct(50))
        .num("delay_p95_ticks", s.delay_pct(95))
        .num("delay_p99_ticks", s.delay_pct(99))
        .num("delay_max_ticks", s.delay_max())
        .num("delay_p50_ms", ticks_to_ms(s.delay_pct(50)))
        .num("delay_p99_ms", ticks_to_ms(s.delay_pct(99)))
        .raw("delay_hist_per_tick", numbers(&s.delay));
    o.inline()
}

/// Loss and latency distributions measured on the wire plus the rollback
/// depth each client took, per profile; the 8-tick resim window sizing data.
#[must_use]
pub fn distributions_json(profiles: &[(&str, &BoutReport)]) -> String {
    let mut rows = Vec::new();
    for (label, report) in profiles {
        let mut o = Obj::new();
        o.text("profile", label)
            .raw("links", lines(report.links.iter().map(link_json).collect()));
        let rollback: Vec<String> = report
            .seats
            .iter()
            .map(|seat| {
                let s = &seat.stats;
                let mut r = Obj::new();
                r.num("slot", seat.slot)
                    .num("states_applied", s.applied)
                    .num("rollback_p50_ticks", s.rollback_pct(50))
                    .num("rollback_p95_ticks", s.rollback_pct(95))
                    .num("rollback_p99_ticks", s.rollback_pct(99))
                    .num("rollback_max_ticks", s.rollback_max)
                    .num("deeper_than_8_tick_window", s.over_cap)
                    .num("input_delay_ticks", s.input_delay)
                    .num("input_ticks_delivered", s.delivery.input_delivered)
                    .num("input_ticks_lost", s.delivery.input_lost)
                    .num("input_loss_permille", s.delivery.input_loss_permille)
                    .num("state_loss_permille", s.delivery.state_loss_permille)
                    .num("state_ack_latest", s.delivery.state_latest)
                    .num("state_ack_bits", s.delivery.state_bits)
                    .num("catchup_resync_depth_ticks", s.resync_depth)
                    .raw("rollback_hist_per_tick", numbers(&s.rollback));
                r.inline()
            })
            .collect();
        o.raw("rollback", lines(rollback));
        rows.push(o.inline());
    }
    let mut top = Obj::new();
    top.num("resim_window_ticks", crate::scale::RESIM_CAP_TICKS)
        .raw("profiles", lines(rows));
    top.finish()
}

/// The 16-bot soak: both runs and whether they agree.
#[must_use]
pub fn soak_json(first: &SoakReport, second: &SoakReport) -> String {
    let mut o = Obj::new();
    o.num("bots", first.fighters)
        .num("ticks", first.ticks)
        .hex("hash_run1", first.hash)
        .hex("hash_run2", second.hash)
        .flag("hash_x2_identical", first == second)
        .num("snapshot_bytes", first.snapshot_bytes)
        .num("snapshot_hz", first.snapshot_hz)
        .num("bytes_per_sec", first.bytes_per_sec);
    o.finish()
}

/// The WAN fault matrix: one row per fault with the shared checkpoint.
#[must_use]
pub fn matrix_json(matrix: &[FaultReport], repeat: &[FaultReport]) -> String {
    let rows = matrix
        .iter()
        .zip(repeat)
        .map(|(a, b)| {
            let mut o = Obj::new();
            o.text("fault", a.fault.label())
                .hex("hash", a.hash)
                .num("scheduled", a.scheduled)
                .num("delivered", a.delivered)
                .num("dropped", a.dropped)
                .num("reordered", a.reordered)
                .num("resyncs", a.resyncs)
                .num("rollback_max", a.rollback_max)
                .num("converge_ticks", a.converge_ticks)
                .flag("x2_identical", a.digest_key() == b.digest_key());
            o.inline()
        })
        .collect();
    let mut top = Obj::new();
    top.flag(
        "all_faults_hold_one_checkpoint",
        matrix.windows(2).all(|pair| pair[0].hash == pair[1].hash),
    )
    .raw("faults", lines(rows));
    top.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_objects_render_valid_ordered_json() {
        let mut o = Obj::new();
        o.num("a", 1)
            .text("b", "q\"x\\y\n")
            .flag("c", true)
            .hex("d", 0xAB)
            .opt("e", None)
            .opt("f", Some(7))
            .raw("g", numbers(&[1, 2, 3]));
        assert_eq!(
            o.finish(),
            "{\n  \"a\": 1,\n  \"b\": \"q\\\"x\\\\y\",\n  \"c\": true,\n  \"d\": \"00000000000000ab\",\n  \"e\": null,\n  \"f\": 7,\n  \"g\": [1, 2, 3]\n}\n"
        );
        assert_eq!(lines(Vec::new()), "[]");
        assert_eq!(numbers::<u32>(&[]), "[]");
    }

    #[test]
    fn distributions_json_logs_adaptive_input_delay() {
        // MP24: every WAN profile reports its input delay with the receipt,
        // so `mp6-check` can verify the delay without reading the sim.
        let cfg = crate::played::BoutConfig::for_profile(7, crate::link::LinkProfile::WAN);
        let report = crate::played::run_loopback(
            &cfg,
            crate::link::LinkProfile::WAN,
            crate::link::LinkProfile::WAN,
        )
        .expect("bout runs");
        let expected = crate::scale::adaptive_input_delay_for_hold(cfg.hold);
        let json = distributions_json(&[("wan", &report)]);
        assert!(
            json.contains("\"input_delay_ticks\""),
            "WAN receipt must log input delay, got: {json}"
        );
        for seat in &report.seats {
            assert_eq!(
                seat.stats.input_delay, expected,
                "every WAN seat logs its delay"
            );
            let needle = format!("\"input_delay_ticks\": {}", seat.stats.input_delay);
            assert!(
                json.contains(&needle),
                "receipt must carry seat delay {needle}"
            );
        }
    }
}
