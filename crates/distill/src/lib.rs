//! engine-distill: offline tiny-student trainer.
//!
//! Consumer: the tournament trains one [`Student`] per candidate from a fixed
//! teacher dataset, exports the winner with [`export`] and loads the bytes in
//! the `engine_nn` runtime.
//! Owner: this crate owns the student schema, the training loop and the
//! float-to-int export fold. Units: features are unitless `f32` in
//! `[-8, 8]`; runtime inputs are `i8` at 16 steps per unit; the exported
//! dense layer carries `i8` weights, `i32` biases and an integer
//! multiplier-plus-shift requantization. Determinism scope: fixed seeds,
//! fixed batch order (the burn trainer runs full-batch; [`fit_mlp`] shuffles
//! with the seeded engine rng) and no threads spawned here; the same
//! dataset plus [`StudentSpec`] trains the same student on the same machine
//! (same binary, CPU and thread count). Cross-machine float-bit
//! reproducibility is NOT claimed, and the `i8` export is the portable
//! artifact. Errors: bad specs, rows and exports map to typed errors;
//! training outside the validated shapes is rejected before burn runs.
//!
//! ## Backend policy
//!
//! Pure-Rust CPU only: `burn` with `ndarray` (burn-ndarray) plus `train`
//! (burn-train, autodiff, dataset helpers). `fusion` stays off: in burn
//! 0.21 it only fuses CubeCL kernels, so it is a no-op for NdArray.
//! Excluded on purpose: `tch` (needs the libtorch C++ runtime), `wgpu` /
//! `cuda` / `rocm` (need a GPU), `candle` (optional native BLAS paths) and
//! `remote`/`collective` (network). The default build uses none of burn at
//! all and stays std-only.
//!
//! ## Pipeline
//!
//! Duel pipeline (the arena's depth brain as teacher): [`duel`] taps
//! the arena through its public controller seam to record `(lanes, intent)`
//! rows on fixed seeds, [`ReplayBuffer::from_episodes`] digests them,
//! [`fit_mlp`] fits a two-layer student, [`export_mlp`] folds it to an int8
//! `.e2nn`, and [`strength_gate`] plays it against the depth brain in real
//! arena bouts with a binomial test. The `distill_duel` binary runs the
//! whole chain and writes `fixtures/nets/depth_student.e2nn`.
//!
//! Generic pipeline: any teacher (provider, script, recorded human play)
//! records decision rows; the caller maps them to [`Sample`]s. [`train`]
//! fits one dense layer, [`export`]
//! quantizes it into the converter's single-layer `.e2nn` shape, and the
//! `engine_nn` runtime runs it in the tick. [`tournament`] then plays the
//! exported bytes against a scripted stand-in over seeded fork scripts and
//! reports the neural-policy evidence: the student wins at least as often as
//! scripted, inside the balance verifier's mirrored bounds.

mod data;
mod export;
pub mod duel;
/// FPS budget gate for the tiny in-tick student: static op/byte caps plus the
/// offline byte check the ship generator runs. Units, errors and determinism
/// scope live on the module items.
pub mod fps;
mod mlp;
mod replay;
mod spec;
mod student;
mod tournament;
#[cfg(feature = "train")]
mod train;

pub use data::{
    check_dataset, check_manifests_for_export, check_provenance, DataError, DataErrorAt,
    DatasetManifest, ProvenanceError, Sample, Shipment, INPUT_QUANT_SCALE, MAX_FEATURE_ABS,
    MAX_SAMPLES,
};
pub use export::{export, export_mlp, quantize_input, E2nnBytes, ExportError};
pub use duel::{
    play_bout, teacher_episode, teacher_rows_per_episode, BoutOutcome, DuelError, Opponent,
    StudentController, TeacherDriver, DUEL_CLASSES, DUEL_LANES, DUEL_LANE_STEPS,
};
pub use fps::{
    check_fps, mlp_tick_ops, FpsError, FpsReport, FPS_CLASSES, FPS_LANES, MAX_FPS_BYTES,
    MAX_FPS_TICK_OPS,
};
pub use mlp::{fit_mlp, FitError, Mlp, MlpSpec, MAX_HIDDEN};
pub use replay::{
    judge_buffer_student, Episode, KillCause, ReplayBuffer, ReplayConfig, ReplayError,
    ReplayVerdict,
};
pub use spec::{Dims, SpecError, StudentSpec, MAX_EPOCHS, MAX_SPEC_DIM};
pub use student::{argmax, mse_loss, Student, StudentError};
pub use tournament::{
    binomial_two_sided_p, score_against_teacher, scripted_decide, strength_gate, tournament,
    wilson95, Comparison, MatchScript, StrengthError, StrengthReport, TournamentError,
    TournamentReport, BALANCE_MAX_PERMILLE, BALANCE_MIN_PERMILLE, DEFAULT_BOUTS,
    DEFAULT_BOUT_TICKS, DEFAULT_MATCHES, MAX_BOUTS, MAX_CLASSES, MAX_MATCHES,
    MAX_OBSERVATIONS_PER_MATCH,
};
#[cfg(feature = "train")]
pub use train::{train, TrainError, TRAIN_LR};

/// Root wiring vectors: the distill crate root as its callers see it.
///
/// Consumer: the tournament, the `distill_duel` binary and any future caller
/// linking `engine-distill` reach every number through the crate root
/// (`crate::StudentSpec`, `::quantize_input`, ...). Data owner stays each
/// module; this module owns only the root re-export contract. Units:
/// feature units, quant steps, permille, p-values. Determinism: integer
/// compares and exact float equality only, no clock, no IO, no burn (the
/// default build stays std-only). Errors are typed per module, asserted
/// through the root path.
///
/// Why this module exists: `crates/distill/src/lib.rs` changed 6 times in
/// 30 days yet held zero `#[test]` and no `tests/it` module named it, so a
/// dropped `pub use` would compile the crate but break every caller at
/// their `use` line. Each vector below names the root path, so removing or
/// gating a re-export fails here first.
#[cfg(test)]
mod root_wiring_tests {
    use crate::{
        argmax, binomial_two_sided_p, check_dataset, check_fps, mlp_tick_ops, mse_loss,
        quantize_input, score_against_teacher, scripted_decide, wilson95, DataError, FpsReport,
        Sample, Student, StudentSpec, BALANCE_MAX_PERMILLE, BALANCE_MIN_PERMILLE, DEFAULT_BOUTS,
        DEFAULT_BOUT_TICKS, DEFAULT_MATCHES, DUEL_CLASSES, DUEL_LANES, DUEL_LANE_STEPS,
        FPS_CLASSES, FPS_LANES, INPUT_QUANT_SCALE, MAX_BOUTS, MAX_CLASSES, MAX_EPOCHS,
        MAX_FEATURE_ABS, MAX_FPS_BYTES, MAX_FPS_TICK_OPS, MAX_HIDDEN, MAX_MATCHES,
        MAX_OBSERVATIONS_PER_MATCH, MAX_SAMPLES, MAX_SPEC_DIM,
    };

    /// Every re-exported module stays reachable with its headline numbers
    /// intact. Removing one `pub use` line fails this file at its `use`.
    #[test]
    fn root_pins_headline_consts() {
        assert_eq!(INPUT_QUANT_SCALE, 16.0);
        assert_eq!(MAX_FEATURE_ABS, 8.0);
        assert_eq!(MAX_SAMPLES, 65_536);
        assert_eq!(MAX_SPEC_DIM, 1_024);
        assert_eq!(MAX_EPOCHS, 10_000);
        assert_eq!(MAX_HIDDEN, 256);
        assert_eq!(DEFAULT_MATCHES, 100);
        assert_eq!(MAX_MATCHES, 100);
        assert_eq!(BALANCE_MIN_PERMILLE, 350);
        assert_eq!(BALANCE_MAX_PERMILLE, 650);
        assert_eq!(MAX_CLASSES, 256);
        assert_eq!(MAX_OBSERVATIONS_PER_MATCH, 3_600);
        assert_eq!(DEFAULT_BOUTS, 1_000);
        assert_eq!(MAX_BOUTS, 20_000);
        assert_eq!(DEFAULT_BOUT_TICKS, 3_600);
        assert!(DUEL_LANES > 0, "duel reads lanes via root");
        assert!(DUEL_CLASSES > 0, "duel reads classes via root");
        assert_eq!(DUEL_LANE_STEPS.len(), DUEL_LANES);
    }

    /// The student shape packs through the root and bad dims name the axis.
    #[test]
    fn spec_packs_and_rejects_via_root() {
        let spec = StudentSpec::new(2, 3, 7).expect("small dims are legal via root");
        assert_eq!(spec.dims.input_dim, 2);
        assert_eq!(spec.dims.output_dim, 3);
        assert_eq!(spec.seed, 7);
        assert_eq!(spec.weight_count(), 6);
        assert!(StudentSpec::new(0, 2, 0).is_err(), "zero input refuses via root");
        assert!(
            StudentSpec::new(2, MAX_SPEC_DIM + 1, 0).is_err(),
            "over-cap output refuses via root"
        );
    }

    /// Teacher rows gate through the root: one bad row names its index.
    #[test]
    fn sample_and_dataset_gate_via_root() {
        let spec = StudentSpec::new(1, 2, 0).expect("test spec is legal");
        let good = Sample::new(vec![0.5], 1, &spec).expect("in-range row admits");
        assert_eq!(good.quantized_features(), vec![0.5]);
        assert_eq!(
            Sample::new(vec![0.0, 0.0], 0, &spec),
            Err(DataError::BadFeatureLen { got: 2, want: 1 })
        );
        assert_eq!(
            Sample::new(vec![f32::INFINITY], 0, &spec),
            Err(DataError::BadFeature)
        );
        assert_eq!(
            Sample::new(vec![0.0], 2, &spec),
            Err(DataError::BadTarget { got: 2, cap: 2 })
        );
        check_dataset(&[], &spec).expect("empty slice passes via root");
        check_dataset(core::slice::from_ref(&good), &spec).expect("clean row passes");
        let bad = Sample {
            features: vec![0.0],
            target: 9,
        };
        let at = check_dataset(&[good, bad], &spec).expect_err("bad row trips");
        assert_eq!(at.index, 1, "first bad row is second");
    }

    /// The no-knowledge baseline runs through the root: zero params, exact
    /// forward, first-max-wins head and a 0.5 loss on a [0]-vs-class-0 row.
    #[test]
    fn student_forward_argmax_and_mse_via_root() {
        let spec = StudentSpec::new(2, 2, 0).expect("test spec is legal");
        let zeros = Student::zeros(spec);
        assert_eq!(
            zeros.forward_f32(&[7.0, 8.0]),
            Some(vec![0.0, 0.0]),
            "zero params emit zero logits"
        );
        assert_eq!(zeros.forward_f32(&[1.0]), None, "short input refuses");
        assert_eq!(
            zeros.forward_quantized(&[16, 32]),
            Some(vec![0.0, 0.0]),
            "q steps dequantize at 16 per unit"
        );
        assert_eq!(argmax(&[1, 5, 5, 2]), Some(1), "ties keep the first max");
        assert_eq!(argmax(&[]), None, "empty logits refuse");
        assert!(Student::new(spec, vec![0.0; 5], vec![0.0; 2], 0).is_err());
        assert!(Student::new(spec, vec![f32::INFINITY; 4], vec![0.0; 2], 0).is_err());
        let one = StudentSpec::new(1, 2, 0).expect("narrow spec is legal");
        let row = Sample::new(vec![0.0], 0, &one).expect("row admits");
        assert_eq!(
            mse_loss(&Student::zeros(one), core::slice::from_ref(&row)),
            Some(0.5)
        );
        assert_eq!(mse_loss(&Student::zeros(one), &[]), None, "empty set refuses");
    }

    /// Runtime quantization reads through the root at 16 steps per unit.
    #[test]
    fn quantize_input_matches_runtime_steps_via_root() {
        assert_eq!(quantize_input(&[1.0]), vec![16]);
        assert_eq!(quantize_input(&[0.5]), vec![8]);
        assert_eq!(quantize_input(&[0.0]), vec![0]);
        assert_eq!(quantize_input(&[f32::NAN]), vec![0], "NaN maps to 0");
        assert_eq!(quantize_input(&[1.0, -1.0]).len(), 2, "width survives");
    }

    /// The tournament scoreboard math reads through the root: agreement
    /// permille, first miss, the scripted stand-in, and the report stats.
    #[test]
    fn tournament_scoring_math_via_root() {
        let score = score_against_teacher(&[1, 2, 3], &[1, 0, 3]);
        assert_eq!(score.total, 3);
        assert_eq!(score.agree, 2);
        assert_eq!(score.agree_rate_permille, 666);
        assert_eq!(score.first_divergent_index, Some(1));
        let empty = score_against_teacher(&[], &[]);
        assert_eq!(empty.agree_rate_permille, 1_000, "empty script agrees vacuously");
        assert_eq!(empty.first_divergent_index, None);
        assert_eq!(scripted_decide(&[1, 2, 3], 5), 1, "lane sum mod classes");
        assert_eq!(scripted_decide(&[-1], 4), 3, "negative sums wrap");
        assert_eq!(scripted_decide(&[9, 9], 0), 0, "zero classes refuse");
        assert_eq!(
            scripted_decide(&[4, 5], 7),
            scripted_decide(&[4, 5], 7),
            "stand-in repeats"
        );
        assert_eq!(wilson95(0, 0), (0.0, 1.0), "empty sample spans all");
        let (lo, hi) = wilson95(5, 10);
        assert!(lo >= 0.0 && lo < 0.5 && 0.5 < hi && hi <= 1.0, "interval holds p");
        assert_eq!(wilson95(12, 10), wilson95(10, 10), "overshoot clamps");
        assert_eq!(binomial_two_sided_p(0, 0), 1.0, "empty trial is certain");
        assert_eq!(binomial_two_sided_p(5, 10), 1.0, "even split caps at 1");
        let tail = binomial_two_sided_p(10, 10);
        assert!(
            (tail - 0.001_953_125).abs() < 1e-12,
            "sweep probability {tail}"
        );
        assert_eq!(binomial_two_sided_p(0, 10), tail, "tails mirror");
    }

    /// The FPS budget contract reads through the root: the ai tick shape, the
    /// byte and op caps, the static op sizer and the byte-gate refusal.
    #[test]
    fn fps_budget_pins_the_tick_contract_via_root() {
        assert_eq!((FPS_LANES, FPS_CLASSES), (DUEL_LANES, DUEL_CLASSES));
        assert_eq!((FPS_LANES, FPS_CLASSES), (6, 5));
        assert_eq!(MAX_FPS_BYTES, 1_024);
        assert_eq!(MAX_FPS_TICK_OPS, 512);
        assert_eq!(mlp_tick_ops(FPS_LANES, 24, FPS_CLASSES), 322);
        assert!(check_fps(&[]).is_err(), "empty bytes refuse via root");
        let report = FpsReport {
            digest: 1,
            layers: 2,
            bytes: 406,
            params: 293,
            tick_ops: 322,
        };
        assert!(
            report.to_string().contains("322 ops"),
            "receipt names the cost"
        );
    }
}
