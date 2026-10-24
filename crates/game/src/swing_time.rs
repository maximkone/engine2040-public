//! swing_time: the swing clip's clock from the press to the hit (EG19c).
//!
//! Under rules v2 the fight cues `AttackStarted` on the press tick and the blow
//! lands `lead` ticks later (the armed weapon's startup: pugio 12, gladius 15,
//! spatha 18, clava 21, pilum 24). The look starts the swing clip on the press
//! and this module maps real ticks since the press onto clip ticks so the
//! clip's impact frame meets the hit tick: the windup stretches over the
//! lead-in (a heavier weapon winds up slower), and the last [`STRIKE_TICKS`]
//! before the hit play at the authored rate, so the blade snaps through. After
//! the hit the clip plays on 1:1.
//!
//! Consumer: `FighterLook::clip_time` (the sample), `feed_trails` (the ribbon
//! starts with the strike) and the `swing_on_press` proof. Units: ticks at
//! 60 Hz in, clip ticks out. Determinism: pure float maps of integer ticks;
//! presentation only, the sim hash never reads it. Errors: none, every
//! function is total; a lead of zero is the old behavior (the clip plays 1:1
//! from its own start).

/// Ticks before the hit that play at the authored clip rate: the strike. The
/// windup before it takes whatever real time the weapon's startup leaves.
pub const STRIKE_TICKS: u32 = 6;

/// Ticks after the recovery the swing pick lives on, so the follow-through
/// settles before the guard pose returns.
pub const FOLLOW_TAIL_TICKS: u32 = 4;

/// Clip ticks played `since` real ticks after the press, for a swing whose hit
/// lands `lead` ticks after the press and whose clip meets its impact frame
/// `impact` clip ticks in. `lead == 0` (no press cue: the old outcome-driven
/// swing) or `impact == 0` plays the clip 1:1. A lead under the impact plays
/// the whole lead-in faster, evenly; a lead at or over it stretches only the
/// windup. Continuous and monotonic in `since`, so no frame pops. Pure.
#[must_use]
pub fn clip_ticks(since: f32, lead: u32, impact: u32) -> f32 {
    if lead == 0 || impact == 0 || !since.is_finite() {
        return since;
    }
    let (lead_f, impact_f) = (to_f32(lead), to_f32(impact));
    if since >= lead_f {
        return impact_f + (since - lead_f);
    }
    if lead < impact {
        return since * impact_f / lead_f;
    }
    let strike = to_f32(STRIKE_TICKS.min(impact));
    let windup_real = lead_f - strike;
    let windup_clip = impact_f - strike;
    if since < windup_real && windup_real > 0.0 {
        since * windup_clip / windup_real
    } else {
        windup_clip + (since - windup_real)
    }
}

/// True once the strike leg has begun, `since` whole ticks after the press: the
/// blade is moving at speed, so the sword ribbon draws. A swing with no press
/// lead is always live (the old behavior). Pure.
#[must_use]
pub fn strike_live(since: u32, lead: u32) -> bool {
    lead == 0 || since >= lead.saturating_sub(STRIKE_TICKS)
}

/// Real ticks a press swing's pick lives, from the press: the lead-in, the
/// weapon's recovery and a short tail, capped by the clip's own length after
/// its impact frame (`clip_len - impact` clip ticks) and at least one tick
/// past the hit. Pure.
#[must_use]
pub fn swing_total(lead: u32, recovery: u32, clip_len: u32, impact: u32) -> u32 {
    let follow = recovery
        .saturating_add(FOLLOW_TAIL_TICKS)
        .min(clip_len.saturating_sub(impact))
        .max(1);
    lead.saturating_add(follow)
}

fn to_f32(ticks: u32) -> f32 {
    f32::from(u16::try_from(ticks).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMPACT: u32 = 12;

    #[test]
    fn the_impact_frame_meets_the_hit_tick_for_every_weapon() {
        // The shipped startups (content/design/weapons.rows): dagger 12,
        // gladius 15, spatha 18, club 21, pilum 24.
        for lead in [12_u32, 15, 18, 21, 24, 30] {
            let at_hit = clip_ticks(to_f32(lead), lead, IMPACT);
            assert!(
                (at_hit - to_f32(IMPACT)).abs() < 1e-4,
                "lead {lead}: clip tick {at_hit} at the hit"
            );
        }
    }

    #[test]
    fn the_map_is_monotonic_and_continuous_across_the_hit() {
        for lead in [4_u32, 8, 12, 15, 24, 30] {
            let mut last = clip_ticks(0.0, lead, IMPACT);
            assert!(last.abs() < 1e-4, "starts at the clip start");
            for step in 1..=(8 * (lead + 20)) {
                let since = to_f32(step) / 8.0;
                let now = clip_ticks(since, lead, IMPACT);
                assert!(now >= last, "lead {lead}: monotonic at {since}");
                assert!(
                    now - last <= 1.0 / 8.0 * 3.0 + 1e-3,
                    "lead {lead}: no jump at {since}"
                );
                last = now;
            }
        }
    }

    #[test]
    fn the_strike_plays_at_the_authored_rate_and_the_windup_stretches() {
        // Gladius: 15 ticks to the hit. The last six play 1:1; the first nine
        // cover the first six clip ticks (two thirds speed).
        let rate = |a: f32, b: f32| clip_ticks(b, 15, IMPACT) - clip_ticks(a, 15, IMPACT);
        assert!((rate(9.0, 15.0) - 6.0).abs() < 1e-4, "strike is 1:1");
        assert!(
            (rate(0.0, 9.0) - 6.0).abs() < 1e-4,
            "windup covers six clip ticks"
        );
        // Club: 21 ticks, the windup crawls at 0.4x.
        let slow = clip_ticks(15.0, 21, IMPACT) - clip_ticks(0.0, 21, IMPACT);
        assert!((slow - 6.0).abs() < 1e-4, "{slow}");
        // After the hit the clip plays on 1:1.
        let after = clip_ticks(25.0, 15, IMPACT) - clip_ticks(15.0, 15, IMPACT);
        assert!((after - 10.0).abs() < 1e-4);
    }

    #[test]
    fn no_lead_plays_the_clip_straight_and_bad_input_is_total() {
        assert_eq!(clip_ticks(7.5, 0, IMPACT), 7.5);
        assert_eq!(clip_ticks(7.5, 15, 0), 7.5);
        assert!(clip_ticks(f32::NAN, 15, IMPACT).is_nan());
        // A lead under the impact plays the lead-in faster, evenly.
        assert!((clip_ticks(2.0, 4, IMPACT) - 6.0).abs() < 1e-4);
        assert!((clip_ticks(4.0, 4, IMPACT) - 12.0).abs() < 1e-4);
    }

    #[test]
    fn the_ribbon_waits_for_the_strike_and_a_leadless_swing_is_always_live() {
        assert!(!strike_live(0, 15));
        assert!(!strike_live(8, 15));
        assert!(strike_live(9, 15));
        assert!(strike_live(0, 0));
    }

    #[test]
    fn the_pick_outlives_the_hit_by_the_recovery_and_stays_inside_the_clip() {
        assert_eq!(swing_total(15, 8, 60, IMPACT), 15 + 8 + FOLLOW_TAIL_TICKS);
        // A short clip caps the follow-through; the pick still passes the hit.
        assert_eq!(swing_total(15, 40, 20, IMPACT), 15 + 8);
        assert_eq!(swing_total(15, 0, 12, IMPACT), 16);
    }
}
