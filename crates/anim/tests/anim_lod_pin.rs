//! Anim lod plus rebuild pin: golden tiers and digests over the public api.
//!
//! Owner: engine anim. Units: meters, screen fraction zero to one, mask
//! bytes, fixed point com in meters. Determinism: same inputs give same
//! tiers and digests twice, order is index order, no clock. Errors: typed
//! lod and rebuild errors, bad input refuses. Budget: small fixed tapes,
//! no steady per tick work here.

use engine_anim::hit_react::hit_mask;
use engine_anim::injured::{digest as single_digest, rebuild_single, stance_contains, Loss, STANCE_MIN};
use engine_anim::injured_double::{digest as double_digest, rebuild_double, rebuild_double_from_joints, rebuild_many};
use engine_anim::lod::{assign_tiers, tier_for, LodError, Tier};
use engine_anim::{blend, clip_hash, dequantize, quantize, sample, Clip, Track};

fn two_key() -> Clip {
    Clip {
        tracks: vec![Track {
            times: vec![0.0, 1.0],
            values: vec![[0.0, 0.0, 0.0], [2.0, 4.0, 6.0]],
        }],
    }
}

#[test]
fn lod_tier_boundaries_pin() {
    assert_eq!(tier_for(5.0, 0.002), Ok(Tier::Full));
    assert_eq!(tier_for(12.0, 0.002), Ok(Tier::Half));
    assert_eq!(tier_for(20.0, 0.002), Ok(Tier::Half));
    assert_eq!(tier_for(30.0, 0.002), Ok(Tier::Quarter));
    assert_eq!(tier_for(45.0, 0.002), Ok(Tier::Quarter));
    assert_eq!(tier_for(60.0, 0.002), Ok(Tier::Far));
    assert_eq!(tier_for(80.0, 0.002), Ok(Tier::Far));
    assert_eq!(tier_for(100.0, 0.08), Ok(Tier::Full));
    assert_eq!(tier_for(100.0, 0.04), Ok(Tier::Full));
    assert_eq!(tier_for(100.0, 0.039), Ok(Tier::Half));
    assert_eq!(tier_for(100.0, 0.01), Ok(Tier::Half));
    assert_eq!(tier_for(100.0, 0.009), Ok(Tier::Far));
    assert_eq!(tier_for(80.0, 0.02), Ok(Tier::Half));
    assert_eq!(Tier::Full.period_ticks(), 1);
    assert_eq!(Tier::Half.period_ticks(), 2);
    assert_eq!(Tier::Quarter.period_ticks(), 4);
    assert_eq!(Tier::Far.period_ticks(), 8);
    assert!(!Tier::Far.ik_on());
    assert!(!Tier::Far.springs_on());
    assert!(Tier::Full.ik_on() && Tier::Full.springs_on());
    assert!(Tier::Half.ik_on() && Tier::Half.springs_on());
    assert!(Tier::Quarter.ik_on() && Tier::Quarter.springs_on());
    assert!(Tier::Full.updates_this_tick(7, 3));
    assert!(Tier::Half.updates_this_tick(4, 0));
    assert!(!Tier::Half.updates_this_tick(5, 0));
    assert!(Tier::Half.updates_this_tick(5, 1));
    assert!(Tier::Quarter.updates_this_tick(8, 0));
    assert!(!Tier::Quarter.updates_this_tick(9, 0));
    assert!(Tier::Far.updates_this_tick(16, 0));
    assert!(!Tier::Far.updates_this_tick(17, 0));
}

#[test]
fn lod_assign_sorted_mixed_tape_pin() {
    let dists = vec![5.0, 15.0, 25.0, 35.0, 45.0, 65.0, 70.0, 75.0, 80.0, 85.0];
    let screens = vec![0.002; 10];
    let mut tiers = vec![Tier::Far; 10];
    assign_tiers(&dists, &screens, &mut tiers).expect("sorted tape assigns");
    assert_eq!(
        tiers,
        vec![
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Far,
            Tier::Far
        ]
    );
    let mut again = vec![Tier::Far; 10];
    assign_tiers(&dists, &screens, &mut again).expect("sorted tape repeats");
    assert_eq!(tiers, again);
}

#[test]
fn lod_assign_shuffled_tape_pin() {
    let dists = vec![85.0, 5.0, 80.0, 15.0, 75.0, 25.0, 70.0, 35.0, 65.0, 45.0];
    let screens = vec![0.002; 10];
    let mut tiers = vec![Tier::Far; 10];
    assign_tiers(&dists, &screens, &mut tiers).expect("shuffled tape assigns");
    assert_eq!(
        tiers,
        vec![
            Tier::Far,
            Tier::Full,
            Tier::Far,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full
        ]
    );
}

#[test]
fn lod_assign_all_far_tape_pin() {
    let dists = vec![
        80.0, 81.0, 82.0, 83.0, 84.0, 85.0, 86.0, 87.0, 88.0, 89.0, 90.0, 91.0,
    ];
    let screens = vec![0.002; 12];
    let mut tiers = vec![Tier::Far; 12];
    assign_tiers(&dists, &screens, &mut tiers).expect("all far tape assigns");
    assert_eq!(
        tiers,
        vec![
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Full,
            Tier::Far,
            Tier::Far,
            Tier::Far,
            Tier::Far
        ]
    );
}

#[test]
fn lod_rejects_bad_input_pin() {
    assert_eq!(
        tier_for(f32::NAN, 0.01),
        Err(LodError::NonFinite("distance_m"))
    );
    assert_eq!(
        tier_for(-1.0, 0.01),
        Err(LodError::NegativeDistance(-1.0))
    );
    assert_eq!(tier_for(10.0, 2.0), Err(LodError::BadScreen(2.0)));
    let dists = vec![10.0; 4];
    let screens = vec![0.01; 3];
    let mut tiers = vec![Tier::Far; 4];
    assert_eq!(
        assign_tiers(&dists, &screens, &mut tiers),
        Err(LodError::LengthMismatch {
            expected: 4,
            got: 3
        })
    );
    let wide = vec![80.0; 65];
    let wide_s = vec![0.002; 65];
    let mut wide_out = vec![Tier::Far; 65];
    assert_eq!(
        assign_tiers(&wide, &wide_s, &mut wide_out),
        Err(LodError::TooManyEntries(65))
    );
}

#[test]
fn rebuild_single_golden_pin() {
    let leg = rebuild_single(Loss::LegL, 7).expect("leg loss rebuilds");
    assert_eq!(leg.mask().weights(), &[242, 0, 192, 184, 179, 195]);
    assert_eq!(leg.contacts(), [false, true]);
    assert_eq!(leg.com(), [0.199_218_75, -0.101_562_5]);
    assert_eq!(single_digest(&leg), 0xf581_0aed_56ca_aef8);
    assert!(leg.support_holds());
    let arm = rebuild_single(Loss::ArmL, 7).expect("arm loss rebuilds");
    assert_eq!(arm.mask().weights(), &[242, 235, 192, 0, 179, 195]);
    assert_eq!(arm.contacts(), [true, true]);
    assert_eq!(arm.com(), [0.0, 0.0]);
    assert_eq!(single_digest(&arm), 0xe060_e2f2_be6c_060a);
    assert!(arm.support_holds());
    let pelvis = rebuild_single(Loss::Pelvis, 11).expect("pelvis rebuilds");
    assert_eq!(pelvis.mask().weights(), &[0, 233, 152, 204, 141, 240]);
    assert_eq!(pelvis.contacts(), [true, true]);
    assert_eq!(single_digest(&pelvis), 0xb09b_89f2_e413_f943);
    for kept in 0..6 {
        if leg.mask().weights()[kept] != 0 {
            assert!(
                leg.mask().weights()[kept] >= STANCE_MIN,
                "kept joint stays driven"
            );
        }
    }
    let again = rebuild_single(Loss::LegL, 7).expect("same pair repeats");
    assert_eq!(single_digest(&leg), single_digest(&again));
    assert_ne!(single_digest(&leg), single_digest(&arm));
    assert!(!stance_contains(true, true, [5.0, 5.0]));
    assert!(!stance_contains(true, true, [f32::NAN, 0.0]));
    let hit = hit_mask();
    let both =
        engine_anim::injured::compose(leg.mask(), &hit).expect("masks compose");
    assert_eq!(both.weight(1), 0);
}

#[test]
fn rebuild_double_golden_pin() {
    let both = rebuild_double(Loss::LegL, Loss::ArmL, 7).expect("pair rebuilds");
    assert_eq!(both.mask().weights(), &[242, 0, 192, 0, 179, 195]);
    assert_eq!(both.contacts(), [false, true]);
    assert_eq!(both.com(), [0.199_218_75, -0.101_562_5]);
    assert_eq!(double_digest(&both), 0x9364_3f0a_3418_30ac);
    assert!(both.support_holds());
    let again = rebuild_double(Loss::LegL, Loss::ArmL, 7).expect("pair repeats");
    assert_eq!(double_digest(&both), double_digest(&again));
    assert!(rebuild_double(Loss::LegL, Loss::LegL, 7).is_err());
    assert!(rebuild_double_from_joints(1, 999, 7).is_err());
    assert!(rebuild_many(&[Loss::LegL], 7).is_err());
    assert!(rebuild_many(&[Loss::LegL, Loss::LegR, Loss::ArmL], 7).is_err());
}

#[test]
fn lib_sample_blend_quant_pin() {
    let clip = two_key();
    assert_eq!(sample(&clip, 0, 0.5), [1.0, 2.0, 3.0]);
    assert_eq!(sample(&clip, 0, 0.0), [0.0, 0.0, 0.0]);
    assert_eq!(sample(&clip, 0, 1.0), [2.0, 4.0, 6.0]);
    assert_eq!(sample(&clip, 7, 0.5), [0.0, 0.0, 0.0]);
    assert_eq!(
        blend([0.0, 0.0, 0.0], [2.0, 4.0, 6.0], 0.5),
        [1.0, 2.0, 3.0]
    );
    assert_eq!(
        quantize([3.25, -7.5, 9.99], [-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]),
        [43417, 8192, 65502]
    );
    assert_eq!(
        quantize([0.0, 0.0, 0.0], [-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]),
        [32768, 32768, 32768]
    );
    let back = dequantize(
        quantize([3.25, -7.5, 9.99], [-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]),
        [-10.0, -10.0, -10.0],
        [10.0, 10.0, 10.0],
    );
    for i in 0..3 {
        let want = [3.25, -7.5, 9.99][i];
        assert!((back[i] - want).abs() < 0.001);
    }
    assert_eq!(clip_hash(&two_key()), 0xd228_ddae_2a90_6087);
    assert_eq!(clip_hash(&two_key()), clip_hash(&clip));
}
