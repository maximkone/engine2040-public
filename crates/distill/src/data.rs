//! Training rows as plain data (std-only).
//!
//! Consumer: the caller maps recorded decision rows (observation, action,
//! outcome from any teacher) into [`Sample`]s; the tournament maps fork
//! observations the same way at tournament time. Owner: this module owns the row schema and its
//! validation. Units: features are unitless `f32` in `[-MAX_FEATURE_ABS,
//! MAX_FEATURE_ABS]`; the target is an action-class id. Determinism: samples
//! are plain data with no ordering of their own; the trainer consumes the
//! slice in order, so the same slice trains the same student. Errors: every
//! bad row maps to [`DataError`]; [`Sample::new`] never panics.
//!
//! This module never imports arena, combat or input types: the teacher stays
//! behind plain floats by design, so script, provider and recorded-human
//! teachers all feed one schema.

use crate::spec::StudentSpec;

/// Largest absolute feature value accepted. Keeps training floats near unit
/// scale and keeps the export quantization (`INPUT_QUANT_SCALE`) exact.
pub const MAX_FEATURE_ABS: f32 = 8.0;

/// Feature-to-`i8` scale shared by training and export: `q = round(f * 16)`
/// covers `[-8, 8]` in `[-128, 128]`, clamped to `i8` range.
pub const INPUT_QUANT_SCALE: f32 = 16.0;

/// Largest dataset slice accepted by [`train`](crate::train).
pub const MAX_SAMPLES: usize = 65_536;

/// One distilled teacher decision: what the student saw and which action
/// class the teacher took.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Feature vector, exactly `spec.dims.input_dim` finite values in
    /// `[-MAX_FEATURE_ABS, MAX_FEATURE_ABS]`.
    pub features: Vec<f32>,
    /// Teacher action class, below `spec.dims.output_dim`.
    pub target: u8,
}

/// Row validation failures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DataError {
    /// Feature count differs from the spec input dim; carries `(got, want)`.
    BadFeatureLen {
        /// Features on the row.
        got: usize,
        /// Spec input dim.
        want: usize,
    },
    /// A feature is NaN, infinite or past [`MAX_FEATURE_ABS`].
    BadFeature,
    /// Target class is at or above the spec output dim; carries `(got, cap)`.
    BadTarget {
        /// Target class on the row.
        got: usize,
        /// Spec output dim.
        cap: usize,
    },
}

impl core::fmt::Display for DataError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DataError::BadFeatureLen { got, want } => {
                write!(f, "feature len {got}, want {want}")
            }
            DataError::BadFeature => write!(f, "feature is non-finite or past ±{MAX_FEATURE_ABS}"),
            DataError::BadTarget { got, cap } => {
                write!(f, "target class {got} at or above {cap}")
            }
        }
    }
}

impl std::error::Error for DataError {}

impl Sample {
    /// Validate one row against the spec. Never panics.
    pub fn new(features: Vec<f32>, target: u8, spec: &StudentSpec) -> Result<Self, DataError> {
        if features.len() != spec.dims.input_dim {
            return Err(DataError::BadFeatureLen {
                got: features.len(),
                want: spec.dims.input_dim,
            });
        }
        if !features
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_FEATURE_ABS)
        {
            return Err(DataError::BadFeature);
        }
        if usize::from(target) >= spec.dims.output_dim {
            return Err(DataError::BadTarget {
                got: usize::from(target),
                cap: spec.dims.output_dim,
            });
        }
        Ok(Self { features, target })
    }

    /// Features requantized exactly as the runtime sees them: each value
    /// rounded to the nearest `1 / INPUT_QUANT_SCALE` step. Training on
    /// these values keeps the train distribution identical to inference.
    pub fn quantized_features(&self) -> Vec<f32> {
        self.features
            .iter()
            .map(|v| (v * INPUT_QUANT_SCALE).round() / INPUT_QUANT_SCALE)
            .collect()
    }
}

/// Validate a whole slice in order: length cap first, then each row against
/// the spec. Returns the row index of the first bad row.
pub fn check_dataset(dataset: &[Sample], spec: &StudentSpec) -> Result<(), DataErrorAt> {
    if dataset.len() > MAX_SAMPLES {
        return Err(DataErrorAt {
            index: dataset.len(),
            err: DataError::BadFeatureLen {
                got: dataset.len(),
                want: MAX_SAMPLES,
            },
        });
    }
    for (index, sample) in dataset.iter().enumerate() {
        Sample::new(sample.features.clone(), sample.target, spec)
            .map_err(|err| DataErrorAt { index, err })?;
    }
    Ok(())
}

/// A dataset failure with its row index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataErrorAt {
    /// Index of the offending row (or the slice length for over-cap).
    pub index: usize,
    /// What was wrong with it.
    pub err: DataError,
}

impl core::fmt::Display for DataErrorAt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "row {}: {}", self.index, self.err)
    }
}

impl std::error::Error for DataErrorAt {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.err)
    }
}

/// Longest dataset source id accepted, in chars; longer ids are refused.
pub const MAX_SOURCE_CHARS: usize = 256;
/// Longest license string accepted, in chars; longer strings are refused.
pub const MAX_LICENSE_CHARS: usize = 64;
/// Longest training-purpose note accepted, in chars; longer notes are refused.
pub const MAX_PURPOSE_CHARS: usize = 128;
/// Most datasets one shipped student may list; larger lists are refused.
pub const MAX_DATASETS_PER_MODEL: usize = 16;

/// Licenses a shipped student may train on. Training-data staples
/// (`CC0-1.0`, `CC-BY-4.0`, `CC-BY-SA-4.0`) plus the dependency-license
/// allowlist, so motion clips and code-derived teachers ship under the same
/// gate. Anything else — non-commercial and no-derivative variants, copyleft,
/// proprietary or empty strings — is refused, never guessed.
pub const SHIPPABLE_LICENSES: [&str; 12] = [
    "CC0-1.0",
    "CC-BY-4.0",
    "CC-BY-SA-4.0",
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unlicense",
    "MIT-0",
    "0BSD",
    "Zlib",
];

/// True exactly when the license string names a shippable license, by exact
/// match. Pure and total; empty and unknown strings return false.
pub fn is_shippable_license(license: &str) -> bool {
    SHIPPABLE_LICENSES.contains(&license)
}

/// One training dataset's provenance: where the rows came from, under what
/// license, which content digest the collector stamped, and what the rows may
/// train. Plain data; the digest is opaque here (the tape exporter binds it to
/// replay bytes with the engine hash), while presence, shape and license are
/// checked at this gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetManifest {
    /// Collector id for the rows (tape range, clip pack, teacher name).
    pub source: String,
    /// License the rows ship under; must name [`SHIPPABLE_LICENSES`].
    pub license: String,
    /// Content digest stamped at collection; zero means unstamped and refused.
    pub digest: u64,
    /// What these rows may train (policy family or study name).
    pub trains: String,
}

/// Dataset provenance failures. Every variant names its cause; construction
/// and gate checks never panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceError {
    /// No manifest was supplied: training and export refuse bare datasets.
    MissingManifest,
    /// Source id is empty or past [`MAX_SOURCE_CHARS`]; carries char count.
    BadSource {
        /// Chars on the source id.
        chars: usize,
    },
    /// License names nothing shippable; carries the bounded license string.
    BadLicense {
        /// License string offered, truncated to [`MAX_LICENSE_CHARS`].
        license: String,
    },
    /// Digest is zero: the collector never stamped the rows.
    BadDigest,
    /// Purpose note is empty or past [`MAX_PURPOSE_CHARS`]; carries char count.
    BadPurpose {
        /// Chars on the purpose note.
        chars: usize,
    },
    /// More datasets than [`MAX_DATASETS_PER_MODEL`]; carries the count.
    TooManyDatasets {
        /// Datasets offered.
        got: usize,
    },
}

impl core::fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProvenanceError::MissingManifest => f.write_str("dataset has no manifest"),
            ProvenanceError::BadSource { chars } => {
                write!(
                    f,
                    "dataset source id has {chars} chars, want 1..={MAX_SOURCE_CHARS}"
                )
            }
            ProvenanceError::BadLicense { license } => {
                write!(f, "dataset license {license:?} is not shippable")
            }
            ProvenanceError::BadDigest => f.write_str("dataset digest is zero (unstamped)"),
            ProvenanceError::BadPurpose { chars } => {
                write!(
                    f,
                    "dataset purpose has {chars} chars, want 1..={MAX_PURPOSE_CHARS}"
                )
            }
            ProvenanceError::TooManyDatasets { got } => {
                write!(
                    f,
                    "model lists {got} datasets, cap {MAX_DATASETS_PER_MODEL}"
                )
            }
        }
    }
}

impl std::error::Error for ProvenanceError {}

/// Truncate a diagnostic string to [`MAX_LICENSE_CHARS`] chars.
fn bound_license(text: &str) -> String {
    if text.chars().count() <= MAX_LICENSE_CHARS {
        text.to_string()
    } else {
        text.chars().take(MAX_LICENSE_CHARS).collect()
    }
}

impl DatasetManifest {
    /// Validate the four fields and pack the manifest. Never panics.
    pub fn new(
        source: &str,
        license: &str,
        digest: u64,
        trains: &str,
    ) -> Result<Self, ProvenanceError> {
        let manifest = Self {
            source: source.to_string(),
            license: license.to_string(),
            digest,
            trains: trains.to_string(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Re-check a manifest built by any path (struct literal included):
    /// source shape, license readability, stamped digest, purpose shape.
    /// Pure; never panics.
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        let source_chars = self.source.chars().count();
        if source_chars == 0 || source_chars > MAX_SOURCE_CHARS {
            return Err(ProvenanceError::BadSource {
                chars: source_chars,
            });
        }
        if !is_shippable_license(&self.license) {
            return Err(ProvenanceError::BadLicense {
                license: bound_license(&self.license),
            });
        }
        if self.digest == 0 {
            return Err(ProvenanceError::BadDigest);
        }
        let purpose_chars = self.trains.chars().count();
        if purpose_chars == 0 || purpose_chars > MAX_PURPOSE_CHARS {
            return Err(ProvenanceError::BadPurpose {
                chars: purpose_chars,
            });
        }
        Ok(())
    }

    /// Fixed shippable manifest for tests: synthetic teacher rows under CC0.
    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        Self::new("fixture-teacher", "CC0-1.0", 0xF1A7u64, "tiny-fixtures")
            .expect("fixture manifest is legal")
    }
}

/// Training gate: refuse a missing manifest, re-validate a supplied one so
/// hand-built manifests face the same license check as constructed ones.
/// Returns the manifest on success. Pure; never panics.
pub fn check_provenance(
    manifest: Option<&DatasetManifest>,
) -> Result<&DatasetManifest, ProvenanceError> {
    match manifest {
        None => Err(ProvenanceError::MissingManifest),
        Some(manifest) => {
            manifest.validate()?;
            Ok(manifest)
        }
    }
}

/// Export gate: refuse an empty dataset list, re-validate every license and
/// cap the list at [`MAX_DATASETS_PER_MODEL`]. Pure; never panics.
pub fn check_manifests_for_export(manifests: &[DatasetManifest]) -> Result<(), ProvenanceError> {
    if manifests.is_empty() {
        return Err(ProvenanceError::MissingManifest);
    }
    if manifests.len() > MAX_DATASETS_PER_MODEL {
        return Err(ProvenanceError::TooManyDatasets {
            got: manifests.len(),
        });
    }
    for manifest in manifests {
        manifest.validate()?;
    }
    Ok(())
}

/// What ships beside one `.e2nn` file: the byte length plus the full dataset
/// list in caller order, so every shipped model names its training. Built
/// only by [`Shipment::new`], which refuses empty and oversize lists; the
/// byte-to-digest binding itself belongs to the tape exporter, which hashes
/// replay bytes with the engine hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shipment {
    /// Length of the shipped `.e2nn` bytes the list travels with.
    pub model_len: usize,
    /// Dataset manifests in caller order, `1..=MAX_DATASETS_PER_MODEL`.
    pub datasets: Vec<DatasetManifest>,
}

impl Shipment {
    /// Pair emitted bytes with their dataset list. Never panics.
    pub fn new(model_bytes: &[u8], datasets: &[DatasetManifest]) -> Result<Self, ProvenanceError> {
        check_manifests_for_export(datasets)?;
        Ok(Self {
            model_len: model_bytes.len(),
            datasets: datasets.to_vec(),
        })
    }
}

impl core::fmt::Display for Shipment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "model {} B from {} dataset(s):",
            self.model_len,
            self.datasets.len()
        )?;
        for manifest in &self.datasets {
            write!(
                f,
                " [{} {} digest {:#x} for {}]",
                manifest.source, manifest.license, manifest.digest, manifest.trains
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> StudentSpec {
        StudentSpec::new(3, 2, 1).expect("test spec is legal")
    }

    #[test]
    fn valid_row_packs() {
        let row = Sample::new(vec![1.0, -2.5, 8.0], 1, &spec()).expect("in-range row is legal");
        assert_eq!(row.features, vec![1.0, -2.5, 8.0]);
        assert_eq!(row.target, 1);
    }

    #[test]
    fn bad_rows_rejected_with_cause() {
        let spec = spec();
        assert_eq!(
            Sample::new(vec![1.0, 2.0], 0, &spec),
            Err(DataError::BadFeatureLen { got: 2, want: 3 })
        );
        assert_eq!(
            Sample::new(vec![1.0, f32::NAN, 0.0], 0, &spec),
            Err(DataError::BadFeature)
        );
        assert_eq!(
            Sample::new(vec![1.0, f32::INFINITY, 0.0], 0, &spec),
            Err(DataError::BadFeature)
        );
        assert_eq!(
            Sample::new(vec![1.0, 8.5, 0.0], 0, &spec),
            Err(DataError::BadFeature)
        );
        assert_eq!(
            Sample::new(vec![1.0, 2.0, 3.0], 2, &spec),
            Err(DataError::BadTarget { got: 2, cap: 2 })
        );
    }

    #[test]
    fn quantized_features_round_to_steps() {
        let row = Sample::new(vec![0.1, -0.2, 0.0], 0, &spec()).expect("legal row");
        let q = row.quantized_features();
        // 0.1 * 16 = 1.6 -> 2 -> 0.125; -0.2 * 16 = -3.2 -> -3 -> -0.1875.
        assert_eq!(q, vec![0.125, -0.1875, 0.0]);
    }

    #[test]
    fn dataset_check_finds_first_bad_row() {
        let spec = spec();
        let good = Sample::new(vec![0.0; 3], 0, &spec).expect("legal");
        let bad = Sample {
            features: vec![0.0; 3],
            target: 7,
        };
        let rows = vec![good.clone(), good, bad];
        let err = check_dataset(&rows, &spec).expect_err("row 2 is bad");
        assert_eq!(err.index, 2);
        assert_eq!(err.err, DataError::BadTarget { got: 7, cap: 2 });
    }

    #[test]
    fn missing_manifest_refused_with_typed_error() {
        assert_eq!(
            check_provenance(None),
            Err(ProvenanceError::MissingManifest)
        );
    }

    #[test]
    fn unreadable_licenses_refused_at_construction() {
        for license in [
            "",
            "CC-BY-NC-4.0",
            "CC-BY-ND-4.0",
            "GPL-3.0-only",
            "proprietary",
            "MIT ",
        ] {
            match DatasetManifest::new("arena-tapes", license, 1, "melee-policy") {
                Err(ProvenanceError::BadLicense { .. }) => {}
                other => panic!("license {license:?} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn shippable_licenses_all_pass() {
        for license in SHIPPABLE_LICENSES {
            DatasetManifest::new("arena-tapes", license, 1, "melee-policy")
                .unwrap_or_else(|e| panic!("license {license} must ship, got {e}"));
        }
    }

    #[test]
    fn manifest_shape_refused_with_cause() {
        assert_eq!(
            DatasetManifest::new("", "CC0-1.0", 1, "melee-policy"),
            Err(ProvenanceError::BadSource { chars: 0 })
        );
        assert_eq!(
            DatasetManifest::new("arena-tapes", "CC0-1.0", 0, "melee-policy"),
            Err(ProvenanceError::BadDigest)
        );
        assert_eq!(
            DatasetManifest::new("arena-tapes", "CC0-1.0", 1, ""),
            Err(ProvenanceError::BadPurpose { chars: 0 })
        );
    }

    #[test]
    fn valid_manifest_passes_and_shipment_lists_it() {
        let manifest = DatasetManifest::new("arena-tapes", "CC0-1.0", 0xD15_7A5E, "melee-policy")
            .expect("shippable manifest is legal");
        assert_eq!(
            check_provenance(Some(&manifest)).expect("valid passes"),
            &manifest
        );
        let bytes = vec![0u8; 8];
        let shipment =
            Shipment::new(&bytes, std::slice::from_ref(&manifest)).expect("one dataset ships");
        assert_eq!(shipment.datasets, vec![manifest]);
        let receipt = shipment.to_string();
        assert!(
            receipt.contains("arena-tapes"),
            "receipt names the source: {receipt}"
        );
        assert!(
            receipt.contains("CC0-1.0"),
            "receipt names the license: {receipt}"
        );
    }

    #[test]
    fn empty_and_oversize_shipments_refused() {
        let bytes = vec![0u8; 8];
        assert_eq!(
            Shipment::new(&bytes, &[]),
            Err(ProvenanceError::MissingManifest)
        );
        let many = vec![DatasetManifest::fixture(); MAX_DATASETS_PER_MODEL + 1];
        assert_eq!(
            Shipment::new(&bytes, &many),
            Err(ProvenanceError::TooManyDatasets {
                got: MAX_DATASETS_PER_MODEL + 1
            })
        );
    }
}
