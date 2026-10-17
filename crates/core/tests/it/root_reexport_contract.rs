//! Crate-root re-export contract: the `engine_core::X` a caller writes must
//! resolve to the module-path owner and keep that owner's value.
//! Owner: engine-core (test-only harness); each value's owner stays its module.
//! Consumer: any crate that imports engine-core through the crate root
//! (`engine_core::Fnv64`, `::FIXED_DT`, `::quat_mul`, `::IntAcc`, ...).
//! Units: ticks and hashes are u64, nanoseconds u64, seconds f32, quaternions
//! `[x, y, z, w]`. Determinism: integer and bit compares only, no clock, no IO,
//! no network. Errors: panics via asserts; `expect` names proven local bounds.
//!
//! Why this file exists: `crates/core/src/lib.rs` changed 21 times in 30 days
//! yet holds zero `#[test]`, and no module of this folder names it. Dropping
//! one `pub use` line or drifting one root constant still compiles this crate
//! and silently changes what every caller sees, so each test below names a
//! root path a caller writes plus the value that path must hand back.

use engine_core::fixed::Fixed;
use engine_core::{counter_rng, hash, quat, rng, seal_key, tick, time};

/// Stillness window a caller must see at 60 Hz: 3 s of ticks.
const STUCK_WINDOW_TICKS_3S: u32 = 180;

#[test]
fn root_hash_owners_are_the_one_owner() {
    // Published FNV-1a 64 vectors, read through the root path a caller uses.
    assert_eq!(engine_core::hash_bytes(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(engine_core::hash_hex(b""), "cbf29ce484222325");
    assert_eq!(
        engine_core::hash_bytes(b"foobar"),
        hash::hash_bytes(b"foobar")
    );
    assert_eq!(engine_core::hash_u64s(&[1, 2]), hash::hash_u64s(&[1, 2]));
    assert_eq!(
        engine_core::mix64(0),
        0,
        "zero is the finalizer fixed point"
    );
    assert_eq!(engine_core::splitmix64(0), 0xe220_a839_7b1d_cdaf);
    // One type, not two lookalikes: a module-path state resumes in a root value.
    let mut chained: engine_core::Fnv64 = hash::Fnv64::with_state(hash::hash_bytes(b"engine"));
    chained.write(b"2040");
    assert_eq!(chained.finish(), engine_core::hash_bytes(b"engine2040"));
}

#[test]
fn root_tick_constants_keep_the_fixed_step() {
    assert_eq!(
        engine_core::FIXED_DT.to_bits(),
        tick::DT.to_bits(),
        "the root step must be the tick owner's bits"
    );
    assert_eq!(engine_core::TICK_HZ_U32, 60, "60 Hz as the narrow form");
    assert_eq!(u64::from(engine_core::TICK_HZ_U32), tick::TICK_HZ);
    assert_eq!(engine_core::STUCK_WINDOW_TICKS, STUCK_WINDOW_TICKS_3S);
    assert_eq!(
        engine_core::STUCK_WINDOW_TICKS,
        3 * engine_core::TICK_HZ_U32,
        "the window is 3 s of ticks the root rate owns"
    );
    assert_eq!(engine_core::MAX_STEPS, tick::MAX_STEPS);
    assert_eq!(
        engine_core::MAX_STEPS,
        5,
        "the catch-up cap callers plan around"
    );
    assert_eq!(
        engine_core::Tick(7).0,
        7,
        "the tick index is the tuple field"
    );
    let mut state: engine_core::TickState = tick::TickState::new(2040);
    assert_eq!(state.step(9), engine_core::fold_hash(2040, 0, 9));
}

#[test]
#[expect(
    deprecated,
    reason = "the frozen legacy accumulator alias stays importable for old receipts"
)]
fn root_legacy_accumulator_alias_stays_importable() {
    let legacy = engine_core::FixedAcc::fixed();
    // Same type, not a copy: a caller may pass either name to the same code.
    let mut typed: engine_core::LegacyFixedAcc = legacy;
    assert_eq!(
        typed.push(tick::DT),
        1,
        "one fixed step in gives one step out"
    );
    let alpha = typed.alpha();
    assert!((0.0..=1.0).contains(&alpha), "render alpha stays in range");
    assert_eq!(engine_core::LEGACY_DT.to_bits(), tick::LEGACY_DT.to_bits());
    assert_eq!(engine_core::LEGACY_MAX_FRAME_DT, tick::LEGACY_MAX_FRAME_DT);
}

#[test]
fn root_rng_paths_share_one_stream() {
    let mut root = engine_core::Rng::new(2040);
    let mut module = rng::Rng::new(2040);
    for word in 0..8 {
        assert_eq!(
            root.next_u64(),
            module.next_u64(),
            "root Rng must be the rng owner, word {word}"
        );
    }
    let keyed: engine_core::CounterRng = counter_rng::CounterRng::new(2040);
    assert_eq!(keyed.key(), counter_rng::CounterRng::new(2040).key());
    assert_eq!(
        keyed.sample(9),
        counter_rng::CounterRng::new(2040).sample(9)
    );
    assert_ne!(keyed.sample(9), keyed.sample(10), "counters draw apart");
    // A root cursor walks the same words as a module-path stream drawn by counter.
    let mut cursor: engine_core::CounterCursor = counter_rng::CounterCursor::from_rng(keyed, 0);
    assert_eq!(cursor.next_u64(), keyed.sample(0));
    assert_eq!(cursor.position(), 1, "the cursor advanced one word");
    assert_eq!(cursor.rng().key(), keyed.key());
}

#[test]
fn root_product_matches_the_quat_owner() {
    const ONE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const I: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const K: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    assert_eq!(engine_core::quat_mul(I, [0.0, 1.0, 0.0, 0.0]), K);
    assert_eq!(
        engine_core::quat_mul(ONE, K).map(f32::to_bits),
        K.map(f32::to_bits)
    );
    let a = [0.1, -0.2, 0.3, 0.9];
    let b = [-0.4, 0.5, 0.25, 0.7];
    assert_eq!(
        engine_core::quat_mul(a, b).map(f32::to_bits),
        quat::quat_mul(a, b).map(f32::to_bits),
        "root and module product must be one implementation"
    );
    let fa = [
        Fixed::from_int(1),
        Fixed::from_int(2),
        Fixed::from_int(3),
        Fixed::from_int(4),
    ];
    let fb = [
        Fixed::from_int(5),
        Fixed::from_int(6),
        Fixed::from_int(7),
        Fixed::from_int(8),
    ];
    let root_fixed = engine_core::quat_mul_fixed(fa, fb).map(Fixed::to_bits);
    assert_eq!(root_fixed, quat::quat_mul_fixed(fa, fb).map(Fixed::to_bits));
    assert_eq!(
        root_fixed[0],
        24 * Fixed::SCALE,
        "Hamilton order at the root"
    );
    assert_eq!(root_fixed[3], -6 * Fixed::SCALE);
}

#[test]
fn root_seal_key_guards_fail_closed() {
    assert_eq!(
        engine_core::MIN_SEAL_KEY_BYTES,
        32,
        "32-byte floor at the root"
    );
    assert_eq!(engine_core::parse_hex4(b"00ff"), Some(0x00ff));
    assert_eq!(
        engine_core::parse_hex4(b"00ff"),
        seal_key::parse_hex4(b"00ff")
    );
    assert_eq!(engine_core::parse_hex4(b"00gz"), None, "non-hex refuses");
    let short = "ab".repeat(16);
    assert_eq!(
        engine_core::parse_seal_key(&short),
        Err(engine_core::SealKeyError::TooShort),
        "a 16-byte key must not pad up to the floor"
    );
    assert_eq!(
        engine_core::parse_seal_key("abc"),
        Err(engine_core::SealKeyError::NotHex)
    );
    let full = "0123456789abcdef".repeat(4);
    let key = engine_core::parse_seal_key(&full).expect("64 hex chars parse");
    assert_eq!(key.len(), engine_core::MIN_SEAL_KEY_BYTES);
}

#[test]
fn root_step_agrees_with_the_thirds_accumulator() {
    assert_eq!(engine_core::STEP_NS, 16_666_666);
    assert_eq!(engine_core::NS_PER_SEC, 1_000_000_000);
    assert_eq!(engine_core::MAX_FRAME_NS, 100_000_000);
    assert_eq!(
        time::STEP_NS * 3 + u64::from(time::STEP_REM_THIRDS),
        engine_core::STEP_THIRDS,
        "the root step times three must be the thirds accumulator"
    );
    assert_eq!(engine_core::quantize_wall_secs(0.01), Ok(10_000_000));
    assert_eq!(
        engine_core::quantize_wall_secs(-1.0),
        Err(engine_core::TimeError::NegativeDt)
    );
    assert_eq!(
        engine_core::quantize_wall_secs(f32::NAN),
        Err(engine_core::TimeError::NonFiniteDt)
    );
    let mut acc: engine_core::IntAcc = time::IntAcc::new();
    let mut steps = 0u32;
    for _ in 0..6 {
        steps += acc.push_ns(10_000_000).expect("10 ms push cannot overflow");
    }
    assert_eq!(steps, 3, "6 x 10 ms is 3 fixed steps: 10 ms is exact in ns");
    assert_eq!(acc.tick(), 3);
    let mut capped = engine_core::IntAcc::new();
    assert_eq!(
        capped.push_ns(engine_core::MAX_FRAME_NS),
        Ok(engine_core::MAX_STEPS),
        "a 100 ms frame is capped at MAX_STEPS, never banked"
    );
    assert_eq!(capped.tick(), u64::from(engine_core::MAX_STEPS));
}
