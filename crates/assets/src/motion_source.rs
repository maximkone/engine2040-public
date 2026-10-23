//! motion_source: a provider-style text-to-motion source and the bake that
//! lands one clip on the fighter rig.
//!
//! Consumer: the offline motion bake only; the tick never calls this module.
//! A [`MotionSource`] takes a [`MotionRequest`] (prompt plus seed) and returns
//! a [`MotionClip`] on its own skeleton. [`MockMotionSource`] is the offline
//! library CI uses; the owner's loopback sidecar ([`LOOPBACK_ENDPOINT`]) and a
//! remote text-to-motion API implement the same trait. No provider here opens a
//! socket.
//!
//! [`bake_motion`] runs one clip through the landable path:
//! import (validated clip doc) -> CT1 retarget with both bind poses proved
//! (row counts, unit rotations, bone geometry, ankle height; never names
//! alone) -> cleanup (loop closure, root-motion gate, contact table) -> CT11
//! compression -> play on the fighter-65 rig with a planted-foot slide meter
//! -> CT12 manifest (model hash, prompt, seed, bind digests, training-data
//! licences). A clip whose model trained on non-commercial data is marked
//! non-commercial and [`BakedMotion::package`] refuses it before it writes.
//!
//! Units: meters, seconds, radians, 60 Hz ticks. Determinism: the mock authors
//! one fixed clip per prompt and folds the seed into an arm-angle mood; the
//! same request gives the same clip, compressed bytes and manifest bytes.
//! Trigonometry is bake-only authoring; the tick hash cannot move.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use engine_anim::contract::{skinning_matrices, Contact, LocalPose, Skeleton};
use engine_anim::feet::slide_mm;
use engine_anim::pose::sample_pose_with_rots;
use engine_core::quat_mul;
use engine_scene::ids::content_digest;

use crate::anim_compress::{
    compress_clip, raw_clip_bytes, AnimCompressError, CompressBudget, CompressedClip,
};
use crate::fighter;
use crate::gen_intake;
use crate::manifest::{Manifest, ManifestError, DEFAULT_THREADS, PROFILE_POTATO};
use crate::retarget::{
    retarget_cook, validate_bind, BindPose, BoneMap, ClipTags, HitWindow, RetargetError,
    RetargetPlan, RetargetedClip,
};
use crate::skin_import::{
    import_clip_doc, ClipDoc, CookConfig, ImportedClip, JointDoc, RotationKey, SkeletonDoc,
    SkinImportError, TranslationKey,
};

/// Joints in the HumanML3D-22 layout the mock authors on.
pub const HML22_JOINTS: usize = 22;
/// Longest prompt in characters, after whitespace is folded.
pub const MAX_PROMPT_CHARS: usize = 256;
/// Planted-foot slide budget in integer millimeters.
pub const MAX_FOOT_SLIDE_MM: u32 = 20;
/// Foot height at or below which the contact cook calls a foot planted, meters.
/// The fighter bind ankle sits at about -0.50 m in its parent joint's frame.
pub const GROUND_Y: f32 = -0.45;
/// Fixed tick length used by the cook and the play meter.
pub const TICK_SECONDS: f32 = 1.0 / 60.0;
/// Default endpoint of the owner's combat-motion sidecar. A real loopback
/// provider would POST a prompt and seed here; the mock never touches it.
pub const LOOPBACK_ENDPOINT: &str = "http://127.0.0.1:7863";

/// Manifest path of the compressed clip artifact.
const ARTIFACT_PATH: &str = "motion.e2anm";
/// Manifest file written beside the clip.
const MANIFEST_NAME: &str = "motion.manifest";
/// Manifest source path of the model card.
const MODEL_PATH: &str = "model.json";
/// Manifest source path of the prompt bytes.
const PROMPT_PATH: &str = "prompt.txt";
/// Shortest bone bind length the proof accepts, meters.
const MIN_BONE_M: f32 = 1e-4;
/// Stand-off from the ground the fighter's rest ankles may not exceed, meters.
const ANKLE_TOLERANCE_M: f32 = 0.25;
/// Joint travel that counts as a moving joint in the play receipt, millimeters.
const MOVING_JOINT_MM: u32 = 20;
/// Longest model or provider id in characters.
const MAX_ID_CHARS: usize = 64;
/// Longest provider name in characters.
const MAX_PROVIDER_CHARS: usize = 32;
/// Longest dataset name in characters.
const MAX_DATASET_CHARS: usize = 128;
/// Longest licence expression in characters.
const MAX_LICENSE_CHARS: usize = 128;

/// One text-to-motion request: a normalized prompt and a seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionRequest {
    prompt: String,
    seed: u64,
}

impl MotionRequest {
    /// Normalize a prompt (trim, lower-case, fold whitespace) and keep a seed.
    ///
    /// Errors: [`MotionError::EmptyPrompt`] when nothing is left,
    /// [`MotionError::PromptTooLong`] over [`MAX_PROMPT_CHARS`], and
    /// [`MotionError::PromptControl`] for a control character.
    pub fn new(prompt: &str, seed: u64) -> Result<Self, MotionError> {
        Ok(Self {
            prompt: normalize_prompt(prompt)?,
            seed,
        })
    }

    /// Normalized prompt.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Seed the source folded into its clip.
    pub fn seed(&self) -> u64 {
        self.seed
    }
}

/// A provider of text-to-motion clips. The mock keeps CI offline; a loopback
/// or remote provider implements the same method and does its I/O outside the
/// tick.
pub trait MotionSource {
    /// Generate one clip for the request, on the provider's own skeleton.
    fn generate(&self, request: &MotionRequest) -> Result<MotionClip, MotionError>;
}

/// One generated clip plus the rig and provenance it came with.
#[derive(Clone, Debug)]
pub struct MotionClip {
    /// Provider name ("mock", "loopback", "remote"); short token.
    pub provider: String,
    /// Source skeleton the clip is authored on.
    pub source: Skeleton,
    /// Source bind pose, in the same row order.
    pub source_bind: BindPose,
    /// `(fighter joint, source joint)` pairs the retarget binds.
    pub pairs: Vec<(String, String)>,
    /// Validated clip on the source skeleton.
    pub clip: ImportedClip,
    /// Sidecar tags: loop, root-motion flag, combat windows.
    pub tags: ClipTags,
    /// Model card: weights digest, weight licence and every training licence.
    pub card: ModelCard,
}

/// One training-data licence a model depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainingData {
    /// Dataset name as the model card spells it.
    pub dataset: String,
    /// SPDX identifier or expression (or the card's exact licence text).
    pub license: String,
    /// True when the dataset may be used commercially.
    pub commercial: bool,
}

/// What a generated clip's model was built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCard {
    /// Hub id or owner/model name.
    pub id: String,
    /// Card version; zero is invalid.
    pub version: u32,
    /// Lowercase blake3 hex of the weights.
    pub weights: String,
    /// SPDX licence of the weights themselves.
    pub weights_license: String,
    /// Every training dataset; empty is refused.
    pub training: Vec<TrainingData>,
}

impl ModelCard {
    /// Check the card and the licences the packaged path would rely on.
    ///
    /// The weights licence reuses the generated-asset gate ([`gen_intake`]);
    /// training data is classified by its own commercial flag plus the
    /// non-commercial marker, because fair training licences (CC-BY-4.0 and
    /// friends) are not all on the shipped-code allow-list.
    ///
    /// Errors: a malformed field, a restricted model, a refused weights
    /// licence, an empty training list, or a commercial claim on a
    /// non-commercial licence.
    pub fn check(&self) -> Result<(), MotionError> {
        if !is_id(&self.id, MAX_ID_CHARS) {
            return Err(bad_model(
                "id",
                "want 1 to max chars of ascii letters, digits, - _ . /",
            ));
        }
        if self.version == 0 {
            return Err(bad_model("version", "want a positive card version"));
        }
        if !is_lower_hex(&self.weights, 64) {
            return Err(bad_model("weights", "want 64 lowercase hex digits"));
        }
        check_line("weights_license", &self.weights_license, MAX_LICENSE_CHARS)?;
        gen_intake::check_model(&self.id).map_err(|error| MotionError::LicenseRefused {
            model: self.id.clone(),
            detail: error.to_string(),
        })?;
        gen_intake::check_license(&self.id, &self.weights_license).map_err(|error| {
            MotionError::LicenseRefused {
                model: self.id.clone(),
                detail: error.to_string(),
            }
        })?;
        if self.training.is_empty() {
            return Err(MotionError::UnlicensedModel {
                model: self.id.clone(),
            });
        }
        for entry in &self.training {
            check_line("training.dataset", &entry.dataset, MAX_DATASET_CHARS)?;
            check_line("training.license", &entry.license, MAX_LICENSE_CHARS)?;
            if entry.commercial && gen_intake::non_commercial_hint(&entry.license) {
                return Err(MotionError::LicenseInconsistent {
                    model: self.id.clone(),
                    license: entry.license.clone(),
                });
            }
        }
        Ok(())
    }

    /// True when every training dataset allows commercial use.
    pub fn commercial(&self) -> bool {
        !self.training.is_empty() && self.training.iter().all(|entry| entry.commercial)
    }

    /// First training dataset that blocks commercial use, if any.
    pub fn non_commercial(&self) -> Option<&TrainingData> {
        self.training.iter().find(|entry| !entry.commercial)
    }

    /// Mark this card carries into the manifest and the package gate.
    pub fn license_mark(&self) -> LicenseMark {
        if self.commercial() {
            LicenseMark::Commercial
        } else {
            LicenseMark::NonCommercial
        }
    }

    /// Canonical card bytes: the model hash covers every field below.
    pub fn canonical(&self) -> Vec<u8> {
        let mut text = String::from("e2040-motion-model 1\n");
        let _ = writeln!(text, "id {}", self.id);
        let _ = writeln!(text, "version {}", self.version);
        let _ = writeln!(text, "weights {}", self.weights);
        let _ = writeln!(text, "weights-license {}", self.weights_license);
        for entry in &self.training {
            let mark = if entry.commercial {
                "commercial"
            } else {
                "non-commercial"
            };
            let _ = writeln!(
                text,
                "training\t{}\t{}\t{}",
                entry.dataset, entry.license, mark
            );
        }
        text.into_bytes()
    }

    /// blake3 hex over [`ModelCard::canonical`].
    pub fn digest(&self) -> String {
        content_digest(&self.canonical())
    }
}

/// Commercial standing of one generated clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenseMark {
    /// Every training licence allows commercial use.
    Commercial,
    /// At least one training licence blocks commercial use.
    NonCommercial,
}

impl LicenseMark {
    /// Stable manifest spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Commercial => "commercial",
            Self::NonCommercial => "non-commercial",
        }
    }

    /// True for [`LicenseMark::Commercial`].
    pub fn is_commercial(self) -> bool {
        matches!(self, Self::Commercial)
    }
}

mod mock;
pub use mock::{
    hml22_bind, hml22_pairs, hml22_skeleton, hml22_skeleton_doc, MockMotionSource, HML22,
    HML22_TO_RIG,
};

/// One bake's runtime settings.
#[derive(Clone, Debug)]
pub struct BakeConfig {
    /// Thread count posted in the manifest; the mock bakes serially.
    pub threads: usize,
    /// Target profile posted in the manifest.
    pub profile: String,
    /// Contact threshold handed to the CT1 table cook, meters.
    pub ground_y: f32,
    /// CT11 error budget.
    pub budget: CompressBudget,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            threads: DEFAULT_THREADS,
            profile: PROFILE_POTATO.to_owned(),
            ground_y: GROUND_Y,
            budget: CompressBudget::default(),
        }
    }
}

/// The bind-pose proof, not just a name map.
#[derive(Clone, Debug, PartialEq)]
pub struct BindReport {
    /// Pairs the retarget bound.
    pub pairs: usize,
    /// Source joints the clip was validated against.
    pub source_joints: usize,
    /// Target fighter joints the cook produced.
    pub target_joints: usize,
    /// blake3 over the source skeleton, bind and pair list.
    pub source_digest: String,
    /// blake3 over the target skeleton, bind and pair list.
    pub target_digest: String,
    /// Rest world height of the fighter's left and right ankle, meters.
    pub ankle_y_m: [f32; 2],
    /// Largest target/source bone-length ratio the proof measured.
    pub max_bone_scale: f32,
}

/// Cleanup receipt: loop closure, root intent and contact ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupReport {
    /// Sidecar says the clip tiles.
    pub looped: bool,
    /// Sidecar says the clip drives root motion.
    pub root_motion: bool,
    /// Last pose was forced to the first pose.
    pub loop_closed: bool,
    /// Planar root path length the cook extracted, millimeters.
    pub root_travel_mm: u32,
    /// Contact ticks the clip cooked.
    pub ticks: u32,
}

/// CT11 receipt: serialized size against the raw f32 clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressionReport {
    /// Uncompressed f32 footprint.
    pub raw_bytes: usize,
    /// Serialized compressed clip length.
    pub bytes: usize,
    /// Measured worst position error.
    pub max_error_cm: f32,
    /// Measured worst rotation error.
    pub max_error_deg: f32,
}

/// The headless "plays on the fighter" receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayReport {
    /// Contact ticks played at 60 Hz.
    pub ticks: u32,
    /// Ticks the left foot was planted.
    pub left_plants: u32,
    /// Ticks the right foot was planted.
    pub right_plants: u32,
    /// Down-to-up transitions on the left foot.
    pub left_lifts: u32,
    /// Down-to-up transitions on the right foot.
    pub right_lifts: u32,
    /// Worst planted-foot motion between consecutive down ticks, millimeters.
    pub foot_slide_mm: u32,
    /// Farthest any joint moved from its bind-pose world position, millimeters.
    pub max_joint_travel_mm: u32,
    /// Joints that moved more than [`MOVING_JOINT_MM`].
    pub moving_joints: u32,
}

/// One baked clip: source, target, compressed artifact and CT12 manifest.
#[derive(Clone, Debug)]
pub struct BakedMotion {
    /// The generated source clip and its provenance.
    pub motion: MotionClip,
    /// blake3 of the canonical model card.
    pub model_hash: String,
    /// Commercial standing derived from the card.
    pub license: LicenseMark,
    /// Bind-pose proof.
    pub bind: BindReport,
    /// Cleanup receipt.
    pub cleanup: CleanupReport,
    /// CT11 receipt.
    pub compression: CompressionReport,
    /// Retargeted clip plus authoritative tick tables and tags.
    pub retargeted: RetargetedClip,
    /// Compressed artifact bytes.
    pub compressed: CompressedClip,
    /// Headless play receipt.
    pub play: PlayReport,
    /// Seeded CT12 manifest.
    pub manifest: Manifest,
    /// Canonical manifest bytes.
    pub manifest_bytes: Vec<u8>,
    /// blake3 over [`BakedMotion::manifest_bytes`].
    pub manifest_digest: String,
}

/// Files a commercial package wrote.
#[derive(Clone, Debug)]
pub struct PackagedMotion {
    /// Directory holding the clip and manifest.
    pub dir: PathBuf,
    /// Written compressed clip.
    pub clip_path: PathBuf,
    /// Written manifest.
    pub manifest_path: PathBuf,
    /// blake3 of the manifest bytes.
    pub manifest_digest: String,
    /// Compressed clip length in bytes.
    pub clip_bytes: usize,
}

/// Bake one generated clip onto the fighter rig.
///
/// Errors: provider, licence, bind, import, retarget, cleanup, play, slide,
/// compression and manifest failures. A slide over [`MAX_FOOT_SLIDE_MM`] and a
/// root drift on a clip tagged in place are refused, never rounded down.
pub fn bake_motion(
    source: &dyn MotionSource,
    request: &MotionRequest,
    settings: &BakeConfig,
) -> Result<BakedMotion, MotionError> {
    let mut motion = checked_motion(source, request)?;
    let target = fighter::fighter_skeleton().map_err(MotionError::Rig)?;
    let target_bind = fighter::fighter_bind().map_err(MotionError::Rig)?;
    let (map, bind) = bind_rig(&motion, &target, &target_bind)?;
    let (retargeted, cleanup, play) =
        cooked_on_target(&mut motion, &target, &target_bind, &map, settings)?;
    let compressed =
        compress_clip(&retargeted.clip, &settings.budget).map_err(MotionError::Compress)?;
    let header = compressed.header().map_err(MotionError::Compress)?;
    let compression = CompressionReport {
        raw_bytes: raw_clip_bytes(&retargeted.clip),
        bytes: compressed.byte_len(),
        max_error_cm: header.max_error_cm,
        max_error_deg: header.max_error_deg,
    };
    let model_hash = motion.card.digest();
    let license = motion.card.license_mark();
    let settings_text = settings_bytes(
        request,
        &motion,
        &model_hash,
        &bind,
        &cleanup,
        &compression,
        &play,
    );
    let mut manifest = Manifest::new(request.seed(), settings.threads, &settings.profile)
        .map_err(MotionError::Manifest)?;
    manifest
        .set_settings(settings_text.as_bytes())
        .map_err(MotionError::Manifest)?;
    manifest
        .add_source(MODEL_PATH, "model", &motion.card.canonical())
        .map_err(MotionError::Manifest)?;
    manifest
        .add_source(PROMPT_PATH, "prompt", request.prompt().as_bytes())
        .map_err(MotionError::Manifest)?;
    manifest
        .add_artifact(ARTIFACT_PATH, compressed.as_bytes())
        .map_err(MotionError::Manifest)?;
    let manifest_bytes = manifest.to_bytes().map_err(MotionError::Manifest)?;
    let manifest_digest = content_digest(&manifest_bytes);
    Ok(BakedMotion {
        motion,
        model_hash,
        license,
        bind,
        cleanup,
        compression,
        retargeted,
        compressed,
        play,
        manifest,
        manifest_bytes,
        manifest_digest,
    })
}

/// Generate one clip and refuse a card or clip shape the bake cannot trust.
fn checked_motion(
    source: &dyn MotionSource,
    request: &MotionRequest,
) -> Result<MotionClip, MotionError> {
    let motion = source.generate(request)?;
    motion.card.check()?;
    check_id("provider", &motion.provider, MAX_PROVIDER_CHARS)?;
    if !motion.clip.duration.is_finite() || motion.clip.duration <= 0.0 {
        return Err(MotionError::Cleanup {
            detail: "clip duration must be positive".to_owned(),
        });
    }
    if motion.pairs.is_empty() {
        return Err(MotionError::BadClip {
            field: "pairs",
            detail: "want at least one fighter/source pair".to_owned(),
        });
    }
    Ok(motion)
}

/// Resolve the pair names and prove both binds by geometry before cooking.
fn bind_rig(
    motion: &MotionClip,
    target: &Skeleton,
    target_bind: &BindPose,
) -> Result<(BoneMap, BindReport), MotionError> {
    let pair_refs: Vec<(&str, &str)> = motion
        .pairs
        .iter()
        .map(|(fighter, source)| (fighter.as_str(), source.as_str()))
        .collect();
    let (map, _coverage) = BoneMap::build_checked(&motion.source, target, &pair_refs)
        .map_err(MotionError::Retarget)?;
    let bind = verify_binds(
        &motion.source,
        &motion.source_bind,
        target,
        target_bind,
        &map,
        &pair_refs,
    )?;
    Ok((map, bind))
}

/// Close the loop, run the CT1 cook and meter the played result.
fn cooked_on_target(
    motion: &mut MotionClip,
    target: &Skeleton,
    target_bind: &BindPose,
    map: &BoneMap,
    settings: &BakeConfig,
) -> Result<(RetargetedClip, CleanupReport, PlayReport), MotionError> {
    let loop_closed = close_loop(&mut motion.clip, &motion.tags);
    let cook = CookConfig {
        root_joint: named_joint(target, "hips")?,
        left_foot: named_joint(target, "leftfoot")?,
        right_foot: named_joint(target, "rightfoot")?,
        ground_y: settings.ground_y,
        start_tick: 0,
    };
    let plan = RetargetPlan {
        source_clip: &motion.clip,
        source: &motion.source,
        source_bind: &motion.source_bind,
        target,
        target_bind,
        map,
        config: &cook,
    };
    let retargeted = retarget_cook(&plan, motion.tags.clone()).map_err(MotionError::Retarget)?;
    let ticks = contact_ticks(&retargeted)?;
    let root_travel_mm = root_travel(&retargeted);
    if !motion.tags.root_motion && root_travel_mm > 0 {
        return Err(MotionError::RootDrift {
            travel_mm: root_travel_mm,
        });
    }
    let cleanup = CleanupReport {
        looped: motion.tags.looped,
        root_motion: motion.tags.root_motion,
        loop_closed,
        root_travel_mm,
        ticks,
    };
    let play = play_on_fighter(target, target_bind, &retargeted)?;
    if play.foot_slide_mm > MAX_FOOT_SLIDE_MM {
        return Err(MotionError::FootSlide {
            measured_mm: play.foot_slide_mm,
            budget_mm: MAX_FOOT_SLIDE_MM,
        });
    }
    Ok((retargeted, cleanup, play))
}

#[cfg(not(target_arch = "wasm32"))]
impl BakedMotion {
    /// Write the compressed clip and its CT12 manifest. The commercial gate
    /// runs before any directory or file is touched.
    ///
    /// Errors: [`MotionError::NonCommercial`] for a non-commercial model, a
    /// slide over budget, an I/O failure, or a manifest that does not read
    /// back.
    pub fn package(&self, dir: &Path) -> Result<PackagedMotion, MotionError> {
        if let Some(training) = self.motion.card.non_commercial() {
            return Err(MotionError::NonCommercial {
                model: self.motion.card.id.clone(),
                dataset: training.dataset.clone(),
                license: training.license.clone(),
            });
        }
        if self.play.foot_slide_mm > MAX_FOOT_SLIDE_MM {
            return Err(MotionError::FootSlide {
                measured_mm: self.play.foot_slide_mm,
                budget_mm: MAX_FOOT_SLIDE_MM,
            });
        }
        std::fs::create_dir_all(dir).map_err(io_error)?;
        let clip_path = dir.join(ARTIFACT_PATH);
        std::fs::write(&clip_path, self.compressed.as_bytes()).map_err(io_error)?;
        let manifest_path = dir.join(MANIFEST_NAME);
        self.manifest
            .write(&manifest_path)
            .map_err(MotionError::Manifest)?;
        let back = Manifest::read(&manifest_path).map_err(MotionError::Manifest)?;
        if back != self.manifest {
            return Err(MotionError::Io {
                detail: "manifest read-back differs".to_owned(),
            });
        }
        back.verify_artifacts(dir).map_err(MotionError::Manifest)?;
        Ok(PackagedMotion {
            dir: dir.to_path_buf(),
            clip_path,
            manifest_path,
            manifest_digest: self.manifest_digest.clone(),
            clip_bytes: self.compressed.byte_len(),
        })
    }
}

/// Why a motion request or bake was refused.
#[derive(Debug)]
pub enum MotionError {
    /// The prompt had no words.
    EmptyPrompt,
    /// The prompt is longer than [`MAX_PROMPT_CHARS`].
    PromptTooLong {
        /// Character count after folding.
        chars: usize,
    },
    /// The prompt carries a control character.
    PromptControl,
    /// The mock library has no clip for the prompt.
    NoClip {
        /// The normalized prompt.
        prompt: String,
    },
    /// A clip field is malformed.
    BadClip {
        /// The field name.
        field: &'static str,
        /// What is wrong.
        detail: String,
    },
    /// A model-card field is malformed.
    BadModel {
        /// The field name.
        field: &'static str,
        /// What is wrong.
        detail: String,
    },
    /// The model or weights licence failed the shared gate.
    LicenseRefused {
        /// Model id.
        model: String,
        /// The gate's reason.
        detail: String,
    },
    /// A commercial claim rides on a non-commercial training licence.
    LicenseInconsistent {
        /// Model id.
        model: String,
        /// The licence text.
        license: String,
    },
    /// The card does not name any training data.
    UnlicensedModel {
        /// Model id.
        model: String,
    },
    /// A bind pose, bone or ankle failed the geometric proof.
    Bind {
        /// Bounded detail.
        detail: String,
    },
    /// The fighter rig could not be built.
    Rig(fighter::RigError),
    /// The source clip did not import.
    Import(SkinImportError),
    /// The retarget failed.
    Retarget(RetargetError),
    /// The compression failed.
    Compress(AnimCompressError),
    /// The manifest failed.
    Manifest(ManifestError),
    /// The play sample failed.
    Pose {
        /// Bounded detail.
        detail: String,
    },
    /// Cleanup found no playable duration or contact table.
    Cleanup {
        /// Bounded detail.
        detail: String,
    },
    /// Planted feet slid more than [`MAX_FOOT_SLIDE_MM`].
    FootSlide {
        /// Measured worst slide.
        measured_mm: u32,
        /// The budget.
        budget_mm: u32,
    },
    /// An in-place clip carried root translation.
    RootDrift {
        /// Planar path length.
        travel_mm: u32,
    },
    /// A non-commercial model reached the packaged path.
    NonCommercial {
        /// Model id.
        model: String,
        /// The blocking dataset.
        dataset: String,
        /// Its licence.
        license: String,
    },
    /// A package file could not be written or read back.
    Io {
        /// Bounded detail.
        detail: String,
    },
}

impl fmt::Display for MotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPrompt => write!(f, "prompt is empty"),
            Self::PromptTooLong { chars } => {
                write!(f, "prompt is {chars} chars, over {MAX_PROMPT_CHARS}")
            }
            Self::PromptControl => write!(f, "prompt carries a control character"),
            Self::NoClip { prompt } => write!(f, "the mock library has no clip for {prompt:?}"),
            Self::BadClip { field, detail } => write!(f, "clip {field}: {detail}"),
            Self::BadModel { field, detail } => write!(f, "model {field}: {detail}"),
            Self::LicenseRefused { model, detail } => {
                write!(f, "licence for {model} refused: {detail}")
            }
            Self::LicenseInconsistent { model, license } => {
                write!(f, "{model} claims commercial use of {license}")
            }
            Self::UnlicensedModel { model } => write!(f, "{model} names no training data"),
            Self::Bind { detail } => write!(f, "bind proof: {detail}"),
            Self::Rig(error) => write!(f, "rig: {error}"),
            Self::Import(error) => write!(f, "import: {error}"),
            Self::Retarget(error) => write!(f, "retarget: {error}"),
            Self::Compress(error) => write!(f, "compress: {error}"),
            Self::Manifest(error) => write!(f, "manifest: {error}"),
            Self::Pose { detail } => write!(f, "play pose: {detail}"),
            Self::Cleanup { detail } => write!(f, "cleanup: {detail}"),
            Self::FootSlide {
                measured_mm,
                budget_mm,
            } => write!(
                f,
                "planted feet slid {measured_mm} mm, budget {budget_mm} mm"
            ),
            Self::RootDrift { travel_mm } => {
                write!(f, "an in-place clip carried {travel_mm} mm of root motion")
            }
            Self::NonCommercial {
                model,
                dataset,
                license,
            } => write!(
                f,
                "{model} trained on {dataset} ({license}) is non-commercial"
            ),
            Self::Io { detail } => write!(f, "i/o: {detail}"),
        }
    }
}

impl std::error::Error for MotionError {}

/// Normalize a prompt: trim, lower-case and fold every whitespace run to one
/// space. Control characters are refused, never stripped.
fn normalize_prompt(prompt: &str) -> Result<String, MotionError> {
    if prompt.chars().any(|ch| ch.is_control()) {
        return Err(MotionError::PromptControl);
    }
    let mut out = String::with_capacity(prompt.len());
    for word in prompt.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for lower in word.chars().flat_map(|ch| ch.to_lowercase()) {
            out.push(lower);
        }
    }
    if out.is_empty() {
        return Err(MotionError::EmptyPrompt);
    }
    let chars = out.chars().count();
    if chars > MAX_PROMPT_CHARS {
        return Err(MotionError::PromptTooLong { chars });
    }
    Ok(out)
}

/// Prove both binds by geometry, not by the pair names alone.
fn verify_binds(
    source: &Skeleton,
    source_bind: &BindPose,
    target: &Skeleton,
    target_bind: &BindPose,
    map: &BoneMap,
    pairs: &[(&str, &str)],
) -> Result<BindReport, MotionError> {
    validate_bind(source, source_bind).map_err(MotionError::Retarget)?;
    validate_bind(target, target_bind).map_err(MotionError::Retarget)?;
    if map.len() != target.len() {
        return Err(MotionError::Bind {
            detail: format!(
                "map rows {} differ from target joints {}",
                map.len(),
                target.len()
            ),
        });
    }
    let mut max_bone_scale = 0.0f32;
    for &(target_name, source_name) in pairs {
        let target_joint = named_joint(target, target_name)?;
        let source_joint = named_joint(source, source_name)?;
        if map.source_of(target_joint) != Some(source_joint) {
            return Err(MotionError::Bind {
                detail: format!("{target_name} does not bind {source_name}"),
            });
        }
        if target.joints[target_joint].parent.is_none()
            || source.joints[source_joint].parent.is_none()
        {
            continue;
        }
        let target_len = bone_length(target, target_bind, target_joint);
        let source_len = bone_length(source, source_bind, source_joint);
        if target_len < MIN_BONE_M || source_len < MIN_BONE_M {
            return Err(MotionError::Bind {
                detail: format!(
                    "{target_name}<-{source_name} bones are {target_len:.6}/{source_len:.6} m"
                ),
            });
        }
        let scale = target_len / source_len;
        if !scale.is_finite() || !(0.05..=20.0).contains(&scale) {
            return Err(MotionError::Bind {
                detail: format!("{target_name}<-{source_name} scale {scale:.6} is out of range"),
            });
        }
        max_bone_scale = max_bone_scale.max(scale);
    }
    let rest = fighter::rest_world(target, target_bind);
    let left = named_joint(target, "leftfoot")?;
    let right = named_joint(target, "rightfoot")?;
    let ankle_y_m = [rest[left][1], rest[right][1]];
    if ankle_y_m
        .iter()
        .any(|height| !height.is_finite() || height.abs() > ANKLE_TOLERANCE_M)
    {
        return Err(MotionError::Bind {
            detail: format!("fighter rest ankles at {ankle_y_m:?} m are not on the ground"),
        });
    }
    Ok(BindReport {
        pairs: pairs.len(),
        source_joints: source.len(),
        target_joints: target.len(),
        source_digest: bind_digest(source, source_bind, pairs),
        target_digest: bind_digest(target, target_bind, pairs),
        ankle_y_m,
        max_bone_scale,
    })
}

fn named_joint(skeleton: &Skeleton, name: &str) -> Result<usize, MotionError> {
    skeleton
        .joints
        .iter()
        .position(|joint| joint.name == name)
        .ok_or_else(|| MotionError::Bind {
            detail: format!("the rig has no joint {name}"),
        })
}

fn bone_length(skeleton: &Skeleton, bind: &BindPose, joint: usize) -> f32 {
    if skeleton.joints[joint].parent.is_none() {
        return 0.0;
    }
    // Bind rows are parent-relative offsets, so the joint's own offset is the
    // bone vector under the identity rest rotation the fighter rig uses.
    let offset = bind.positions[joint];
    (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt()
}

fn bind_digest(skeleton: &Skeleton, bind: &BindPose, pairs: &[(&str, &str)]) -> String {
    let mut text = String::from("e2040-motion-bind 1\n");
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let parent = match joint.parent {
            Some(parent) => parent.to_string(),
            None => "-".to_owned(),
        };
        let position = bind.positions[index];
        let rotation = bind.rotations[index];
        let _ = writeln!(
            text,
            "joint {index} {} {parent} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}",
            joint.name,
            position[0],
            position[1],
            position[2],
            rotation[0],
            rotation[1],
            rotation[2],
            rotation[3],
        );
    }
    for &(target, source) in pairs {
        let _ = writeln!(text, "pair {target} {source}");
    }
    content_digest(text.as_bytes())
}

/// Force a looped clip's last pose to its first pose. Returns true when the
/// sidecar asked for a loop, so the receipt records the cleanup.
fn close_loop(clip: &mut ImportedClip, tags: &ClipTags) -> bool {
    if !tags.looped {
        return false;
    }
    for track in &mut clip.positions.tracks {
        let Some(first) = track.values.first().copied() else {
            continue;
        };
        if track.values.len() > 1 {
            if let Some(last) = track.values.last_mut() {
                *last = first;
            }
        }
    }
    for track in &mut clip.rotations {
        let Some(first) = track.values.first().copied() else {
            continue;
        };
        if track.values.len() > 1 {
            if let Some(last) = track.values.last_mut() {
                *last = first;
            }
        }
    }
    true
}

fn contact_ticks(retargeted: &RetargetedClip) -> Result<u32, MotionError> {
    let contacts = retargeted.contacts.contacts.len();
    if contacts == 0 || !contacts.is_multiple_of(2) {
        return Err(MotionError::Cleanup {
            detail: format!("contact table has {contacts} samples, want two per tick"),
        });
    }
    u32::try_from(contacts / 2).map_err(|_| MotionError::Cleanup {
        detail: "contact tick count does not fit u32".to_owned(),
    })
}

fn root_travel(retargeted: &RetargetedClip) -> u32 {
    retargeted.root.deltas.iter().fold(0u32, |total, delta| {
        total.saturating_add(slide_mm([0.0; 3], [delta.dp[0], 0.0, delta.dp[1]]))
    })
}

/// Sample the retargeted clip on the fighter rig at every contact tick and
/// meter what a planted foot actually does.
fn play_on_fighter(
    target: &Skeleton,
    target_bind: &BindPose,
    retargeted: &RetargetedClip,
) -> Result<PlayReport, MotionError> {
    let contacts = &retargeted.contacts.contacts;
    let ticks = contact_ticks(retargeted)?;
    let left_foot = named_joint(target, "leftfoot")?;
    let right_foot = named_joint(target, "rightfoot")?;
    let bind_world = world_at(target, target_bind)?;
    let mut left = FootMeter::default();
    let mut right = FootMeter::default();
    let mut foot_slide_mm = 0u32;
    let mut max_joint_travel_mm = 0u32;
    let mut moving = vec![false; target.len()];

    for frame in 0..ticks {
        let (left_contact, right_contact) = contact_pair(contacts, frame)?;
        let world = posed_world(target, retargeted, frame)?;
        max_joint_travel_mm =
            max_joint_travel_mm.max(joint_travel(&bind_world, &world, &mut moving));
        left.observe(left_contact.down, world[left_foot][3], &mut foot_slide_mm);
        right.observe(right_contact.down, world[right_foot][3], &mut foot_slide_mm);
    }
    let moving_joints =
        u32::try_from(moving.iter().filter(|moved| **moved).count()).unwrap_or(u32::MAX);
    Ok(PlayReport {
        ticks,
        left_plants: left.plants,
        right_plants: right.plants,
        left_lifts: left.lifts,
        right_lifts: right.lifts,
        foot_slide_mm,
        max_joint_travel_mm,
        moving_joints,
    })
}

fn contact_pair(contacts: &[Contact], frame: u32) -> Result<(&Contact, &Contact), MotionError> {
    let base = usize::try_from(frame).map_err(|_| MotionError::Cleanup {
        detail: format!("contact frame {frame} does not fit usize"),
    })? * 2;
    let left = contacts.get(base).ok_or_else(|| MotionError::Cleanup {
        detail: format!("contact frame {frame} is missing its left sample"),
    })?;
    let right = contacts.get(base + 1).ok_or_else(|| MotionError::Cleanup {
        detail: format!("contact frame {frame} is missing its right sample"),
    })?;
    if left.foot != 0 || right.foot != 1 || left.tick != right.tick {
        return Err(MotionError::Cleanup {
            detail: format!("contact frame {frame} is not left/right on one tick"),
        });
    }
    Ok((left, right))
}

fn posed_world(
    target: &Skeleton,
    retargeted: &RetargetedClip,
    frame: u32,
) -> Result<Vec<[[f32; 4]; 4]>, MotionError> {
    let tick = u16::try_from(frame).map_err(|_| MotionError::Cleanup {
        detail: format!("contact frame {frame} is longer than a u16 tick"),
    })?;
    let time = f32::from(tick) * TICK_SECONDS;
    let pose = sample_pose_with_rots(
        target,
        &retargeted.clip.positions,
        &retargeted.clip.rotations,
        time,
    )
    .map_err(|error| MotionError::Pose {
        detail: format!("{error:?}"),
    })?;
    skinning_matrices(target, &pose).map_err(|error| MotionError::Pose {
        detail: format!("{error:?}"),
    })
}

/// Worst joint travel from bind this frame; each joint over the moving budget
/// marks its slot.
fn joint_travel(bind: &[[f32; 3]], world: &[[[f32; 4]; 4]], moving: &mut [bool]) -> u32 {
    let mut worst = 0u32;
    for (joint, matrix) in world.iter().enumerate() {
        let (Some(base), Some(slot)) = (bind.get(joint), moving.get_mut(joint)) else {
            continue;
        };
        let landed = [matrix[3][0], matrix[3][1], matrix[3][2]];
        let travel = slide_mm(*base, landed);
        worst = worst.max(travel);
        if travel > MOVING_JOINT_MM {
            *slot = true;
        }
    }
    worst
}

/// One foot's plant/lift state across a played clip.
#[derive(Clone, Copy, Debug, Default)]
struct FootMeter {
    previous: Option<[f32; 3]>,
    down: bool,
    plants: u32,
    lifts: u32,
}

impl FootMeter {
    fn observe(&mut self, down: bool, world: [f32; 4], worst: &mut u32) {
        let point = [world[0], world[1], world[2]];
        if down {
            self.plants = self.plants.saturating_add(1);
            if let Some(previous) = self.previous {
                *worst = (*worst).max(slide_mm(previous, point));
            }
            self.previous = Some(point);
        } else {
            if self.down {
                self.lifts = self.lifts.saturating_add(1);
            }
            self.previous = None;
        }
        self.down = down;
    }
}

fn world_at(skeleton: &Skeleton, bind: &BindPose) -> Result<Vec<[f32; 3]>, MotionError> {
    let pose = LocalPose {
        positions: bind.positions.clone(),
        rotations: bind.rotations.clone(),
    };
    let matrices = skinning_matrices(skeleton, &pose).map_err(|error| MotionError::Pose {
        detail: format!("{error:?}"),
    })?;
    Ok(matrices
        .iter()
        .map(|matrix| [matrix[3][0], matrix[3][1], matrix[3][2]])
        .collect())
}

fn settings_bytes(
    request: &MotionRequest,
    motion: &MotionClip,
    model_hash: &str,
    bind: &BindReport,
    cleanup: &CleanupReport,
    compression: &CompressionReport,
    play: &PlayReport,
) -> String {
    let mut text = String::from("e2040-motion 1\n");
    let _ = writeln!(text, "provider {}", motion.provider);
    let _ = writeln!(text, "model {}", motion.card.id);
    let _ = writeln!(text, "model-version {}", motion.card.version);
    let _ = writeln!(text, "model-hash {model_hash}");
    let _ = writeln!(text, "weights {}", motion.card.weights);
    let _ = writeln!(text, "weights-license {}", motion.card.weights_license);
    let _ = writeln!(text, "prompt {}", request.prompt());
    let _ = writeln!(text, "seed {}", request.seed());
    let _ = writeln!(text, "license {}", motion.card.license_mark().as_str());
    let _ = writeln!(text, "source-joints {}", bind.source_joints);
    let _ = writeln!(text, "target-joints {}", bind.target_joints);
    let _ = writeln!(text, "pairs {}", bind.pairs);
    let _ = writeln!(text, "source-bind {}", bind.source_digest);
    let _ = writeln!(text, "target-bind {}", bind.target_digest);
    let _ = writeln!(
        text,
        "ankle-y {:.6} {:.6}",
        bind.ankle_y_m[0], bind.ankle_y_m[1]
    );
    let _ = writeln!(text, "looped {}", cleanup.looped);
    let _ = writeln!(text, "loop-closed {}", cleanup.loop_closed);
    let _ = writeln!(text, "root-motion {}", cleanup.root_motion);
    let _ = writeln!(text, "root-travel-mm {}", cleanup.root_travel_mm);
    let _ = writeln!(
        text,
        "compression {} {} {:.3} {:.3}",
        compression.bytes,
        compression.raw_bytes,
        compression.max_error_cm,
        compression.max_error_deg
    );
    let _ = writeln!(text, "foot-slide-mm {}", play.foot_slide_mm);
    for entry in &motion.card.training {
        let mark = if entry.commercial {
            "commercial"
        } else {
            "non-commercial"
        };
        let _ = writeln!(
            text,
            "training\t{}\t{}\t{}",
            entry.dataset, entry.license, mark
        );
    }
    text
}

fn is_id(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.chars().count() <= max
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn check_id(field: &'static str, text: &str, max: usize) -> Result<(), MotionError> {
    if is_id(text, max) {
        Ok(())
    } else {
        Err(MotionError::BadClip {
            field,
            detail: "want 1 to max chars of ascii letters, digits, - _ . /".to_owned(),
        })
    }
}

fn check_line(field: &'static str, text: &str, max: usize) -> Result<(), MotionError> {
    let ok = !text.is_empty()
        && text.chars().count() <= max
        && text.trim() == text
        && !text.chars().any(|ch| ch.is_control());
    if ok {
        Ok(())
    } else {
        Err(MotionError::BadModel {
            field,
            detail: "want one bounded non-empty line".to_owned(),
        })
    }
}

fn bad_model(field: &'static str, detail: &str) -> MotionError {
    MotionError::BadModel {
        field,
        detail: detail.to_owned(),
    }
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn io_error(error: std::io::Error) -> MotionError {
    MotionError::Io {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::pose::QuatTrack;
    use engine_anim::{Clip, Track};

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn pair_refs(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
        pairs
            .iter()
            .map(|(fighter, source)| (fighter.as_str(), source.as_str()))
            .collect()
    }

    #[test]
    fn prompt_normalizes_and_bounds() {
        let request = MotionRequest::new("  Sword   Slash ", 42).unwrap();
        assert_eq!(request.prompt(), "sword slash");
        assert_eq!(request.seed(), 42);
        assert!(matches!(
            MotionRequest::new("   ", 1),
            Err(MotionError::EmptyPrompt)
        ));
        assert!(matches!(
            MotionRequest::new("bad\nprompt", 1),
            Err(MotionError::PromptControl)
        ));
        let long = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert!(matches!(
            MotionRequest::new(&long, 1),
            Err(MotionError::PromptTooLong { .. })
        ));
    }

    #[test]
    fn hml22_binds_twenty_pairs_onto_the_fighter_rig() {
        let source = hml22_skeleton().unwrap();
        let target = fighter::fighter_skeleton().unwrap();
        let source_bind = hml22_bind();
        let target_bind = fighter::fighter_bind().unwrap();
        assert_eq!(source.len(), HML22_JOINTS);
        assert_eq!(target.len(), 65);
        validate_bind(&source, &source_bind).expect("source bind");
        validate_bind(&target, &target_bind).expect("target bind");
        let pairs = hml22_pairs();
        let refs = pair_refs(&pairs);
        let map = BoneMap::build(&source, &target, &refs).expect("every pair resolves");
        let report =
            verify_binds(&source, &source_bind, &target, &target_bind, &map, &refs).unwrap();
        assert_eq!(report.pairs, 20);
        assert_eq!(report.source_joints, 22);
        assert_eq!(report.target_joints, 65);
        assert!(report.ankle_y_m.iter().all(|height| height.abs() < 0.1));
        assert!(report.max_bone_scale > 0.0);
        assert_eq!(report.source_digest.len(), 64);
        assert_eq!(report.target_digest.len(), 64);
    }

    #[test]
    fn mock_slash_is_deterministic_and_seeded() {
        let source = MockMotionSource::commercial();
        let request = MotionRequest::new("sword slash", 42).unwrap();
        let first = source.generate(&request).unwrap();
        let second = source.generate(&request).unwrap();
        assert_eq!(first.clip, second.clip);
        assert_eq!(first.tags, second.tags);
        assert_eq!(first.card, second.card);
        assert_eq!(first.clip.name, "sword_slash");
        assert_eq!(first.clip.positions.tracks.len(), HML22_JOINTS);
        assert_eq!(first.pairs.len(), 20);
        assert_eq!(first.tags.hit_windows.len(), 1);
        let other = source
            .generate(&MotionRequest::new("sword slash", 43).unwrap())
            .unwrap();
        assert_ne!(first.clip, other.clip, "the seed moves the clip");
        let idle = source
            .generate(&MotionRequest::new("idle", 42).unwrap())
            .unwrap();
        assert_eq!(idle.clip.name, "guard_idle");
        assert!(idle.tags.looped);
        for (positions, rotations) in idle
            .clip
            .positions
            .tracks
            .iter()
            .zip(idle.clip.rotations.iter())
        {
            assert_eq!(positions.values.first(), positions.values.last());
            assert_eq!(rotations.values.first(), rotations.values.last());
        }
    }

    #[test]
    fn unknown_prompt_is_refused_not_guessed() {
        let source = MockMotionSource::commercial();
        let request = MotionRequest::new("backflip", 1).unwrap();
        assert!(matches!(
            source.generate(&request),
            Err(MotionError::NoClip { .. })
        ));
    }

    #[test]
    fn non_commercial_training_licence_is_marked() {
        let source = MockMotionSource::non_commercial();
        let card = source.card();
        assert!(!card.commercial());
        assert_eq!(card.license_mark(), LicenseMark::NonCommercial);
        assert_eq!(card.non_commercial().unwrap().license, "CC-BY-NC-4.0");
        card.check().expect("the NC card is well formed");
        assert!(gen_intake::non_commercial_hint("CC-BY-NC-4.0"));
        assert!(gen_intake::non_commercial_hint("AMASS research only"));
        assert!(!gen_intake::non_commercial_hint("CC0-1.0"));
        assert!(!gen_intake::non_commercial_hint("Apache-2.0"));
        assert!(!gen_intake::non_commercial_hint("CC-BY-4.0"));
    }

    #[test]
    fn a_commercial_claim_on_a_non_commercial_licence_is_refused() {
        let mut card = MockMotionSource::commercial().card().clone();
        card.training[0].license = "CC-BY-NC-4.0".to_owned();
        card.training[0].commercial = true;
        assert!(matches!(
            card.check(),
            Err(MotionError::LicenseInconsistent { .. })
        ));
    }

    #[test]
    fn bind_geometry_refuses_a_zero_length_bone() {
        let source = hml22_skeleton().unwrap();
        let source_bind = hml22_bind();
        let target = fighter::fighter_skeleton().unwrap();
        let mut target_bind = fighter::fighter_bind().unwrap();
        let knee = named_joint(&target, "leftleg").unwrap();
        target_bind.positions[knee] = [0.0, 0.0, 0.0];
        let pairs = hml22_pairs();
        let refs = pair_refs(&pairs);
        let map = BoneMap::build(&source, &target, &refs).unwrap();
        let error =
            verify_binds(&source, &source_bind, &target, &target_bind, &map, &refs).unwrap_err();
        assert!(matches!(error, MotionError::Bind { .. }), "{error}");
    }

    #[test]
    fn bind_digest_moves_with_the_bind() {
        let source = hml22_skeleton().unwrap();
        let mut bind = hml22_bind();
        let pairs = hml22_pairs();
        let refs = pair_refs(&pairs);
        let before = bind_digest(&source, &bind, &refs);
        bind.positions[0][1] += 0.01;
        assert_ne!(before, bind_digest(&source, &bind, &refs));
    }

    #[test]
    fn close_loop_copies_the_first_pose_over_the_last() {
        let mut clip = ImportedClip {
            name: "loop".to_owned(),
            duration: 1.0,
            positions: Clip {
                tracks: vec![Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0; 3], [1.0, 0.0, 0.0]],
                }],
            },
            rotations: vec![QuatTrack {
                times: vec![0.0, 1.0],
                values: vec![IDENTITY, [0.0, 0.0, 0.0, -1.0]],
            }],
        };
        let tags = ClipTags {
            looped: true,
            ..ClipTags::default()
        };
        assert!(close_loop(&mut clip, &tags));
        assert_eq!(clip.positions.tracks[0].values[1], [0.0; 3]);
        assert_eq!(clip.rotations[0].values[1], IDENTITY);
        let open = ClipTags::default();
        assert!(!close_loop(&mut clip, &open));
    }
}
