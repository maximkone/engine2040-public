//! provenance: release gate over asset provenance rows.
//!
//! Consumer: the release audit (walks every shipped asset and refuses gaps)
//! and the bundle release check. Data owner: `engine-assets`. Units: chars
//! for field bounds. Determinism: slice order decides which gap is named;
//! row encoding uses fixed key order, so the same records encode the same
//! bytes twice. Errors: typed [`ProvenanceError`] with stable codes.
//! Offline cook only, never the tick.

use engine_core::hash_bytes;
use std::collections::BTreeSet;
use std::fmt;

/// Licences a releasable part may carry (the `deny.toml` allow-list).
/// The import-gated intake keeps its own copy (`gen_intake::ALLOWED_SPDX`)
/// so headless builds need no import feature; the feature-gated test below
/// fails the build if the two ever drift.
pub const ALLOWED_SPDX: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "Zlib",
    "ISC",
    "CC0-1.0",
    "Unlicense",
    "0BSD",
    "MIT-0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
];

/// Longest asset name in chars (ASCII only, so bytes match).
pub const MAX_NAME_CHARS: usize = 64;
/// Longest free-text field (URL, author, generator, model) in chars.
pub const MAX_FIELD_CHARS: usize = 256;
/// Shortest accepted hex digest in chars.
pub const SHA_MIN_LEN: usize = 16;
/// Longest accepted hex digest in chars.
pub const SHA_MAX_LEN: usize = 64;
/// Largest release the gate checks in one call.
pub const MAX_RECORDS: usize = 4096;

/// Provenance of one releasable asset: either the hand-made chain (`url` +
/// `spdx` + `author` + `sha`) or the AI generator chain (`generator` +
/// `model` + `prompt_digest`). `spdx` and `sha` ride both paths: the model
/// has a licence and every asset needs a digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceRecord {
    /// Asset name (`content/arena/sand.glb`): lowercase ASCII, digits,
    /// `_`, `-`, `/` and `.`.
    pub name: String,
    /// Source URL of the hand-made chain; unused on the AI path.
    pub url: String,
    /// Single SPDX identifier from [`ALLOWED_SPDX`].
    pub spdx: String,
    /// Human or studio author of the hand-made chain; unused on the AI path.
    pub author: String,
    /// Lowercase hex digest of the source bytes.
    pub sha: String,
    /// AI runner that made the asset, when the AI path applies.
    pub generator: Option<String>,
    /// AI model id, required on the AI path.
    pub model: Option<String>,
    /// Hex digest of the prompt text or source image, required on the AI path.
    pub prompt_digest: Option<String>,
}

/// Why a release was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceError {
    /// The asset name is empty, too long or has outside chars.
    BadName {
        /// The name given.
        name: String,
    },
    /// The hand-made chain has no usable URL.
    MissingUrl {
        /// Asset that names the gap.
        asset: String,
    },
    /// The SPDX text is not a single identifier.
    BadSpdx {
        /// Asset that names the gap.
        asset: String,
        /// The text given.
        spdx: String,
    },
    /// The SPDX identifier is well formed but off the allow-list.
    LicenseRefused {
        /// Asset that names the gap.
        asset: String,
        /// The identifier given.
        spdx: String,
    },
    /// The hand-made chain names no author.
    MissingAuthor {
        /// Asset that names the gap.
        asset: String,
    },
    /// The digest is missing or not lowercase hex of accepted length.
    BadSha {
        /// Asset that names the gap.
        asset: String,
    },
    /// The AI chain misses a link.
    AiChain {
        /// Asset that names the gap.
        asset: String,
        /// Which link (`generator`, `model` or `prompt_digest`).
        field: &'static str,
    },
    /// Two records share a name.
    Duplicate {
        /// The repeated name.
        asset: String,
    },
    /// The release holds more records than the gate checks.
    TooMany {
        /// Records given.
        count: usize,
        /// Records accepted.
        max: usize,
    },
}

impl ProvenanceError {
    /// Stable machine code for doors and receipts.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadName { .. } => "bad_name",
            Self::MissingUrl { .. } => "missing_url",
            Self::BadSpdx { .. } => "bad_spdx",
            Self::LicenseRefused { .. } => "license_refused",
            Self::MissingAuthor { .. } => "missing_author",
            Self::BadSha { .. } => "bad_sha",
            Self::AiChain { .. } => "ai_chain",
            Self::Duplicate { .. } => "duplicate",
            Self::TooMany { .. } => "too_many",
        }
    }
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName { name } => write!(f, "provenance refused `{name}`: bad asset name"),
            Self::MissingUrl { asset } => {
                write!(f, "provenance refused `{asset}`: missing source URL")
            }
            Self::BadSpdx { asset, spdx } => {
                write!(
                    f,
                    "provenance refused `{asset}`: `{spdx}` is not an SPDX identifier"
                )
            }
            Self::LicenseRefused { asset, spdx } => {
                write!(
                    f,
                    "provenance refused `{asset}`: licence `{spdx}` is off the allow-list"
                )
            }
            Self::MissingAuthor { asset } => {
                write!(f, "provenance refused `{asset}`: missing author")
            }
            Self::BadSha { asset } => {
                write!(f, "provenance refused `{asset}`: bad source digest")
            }
            Self::AiChain { asset, field } => {
                write!(f, "provenance refused `{asset}`: AI chain misses `{field}`")
            }
            Self::Duplicate { asset } => {
                write!(f, "provenance refused `{asset}`: duplicate asset name")
            }
            Self::TooMany { count, max } => {
                write!(
                    f,
                    "provenance refused the release: {count} records over {max}"
                )
            }
        }
    }
}

impl std::error::Error for ProvenanceError {}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '/' | '.')
}

/// True for one allow-listed SPDX identifier (case-insensitive).
fn is_allowed_spdx(id: &str) -> bool {
    ALLOWED_SPDX
        .iter()
        .any(|known| known.eq_ignore_ascii_case(id))
}

/// True for a lowercase hex digest of accepted length.
fn is_digest(text: &str) -> bool {
    (SHA_MIN_LEN..=SHA_MAX_LEN).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn check_name(name: &str) -> Result<(), ProvenanceError> {
    let ok = !name.is_empty() && name.len() <= MAX_NAME_CHARS && name.chars().all(is_name_char);
    if ok {
        Ok(())
    } else {
        Err(ProvenanceError::BadName { name: name.into() })
    }
}

fn check_spdx(asset: &str, spdx: &str) -> Result<(), ProvenanceError> {
    let shaped =
        !spdx.is_empty() && spdx.len() <= MAX_FIELD_CHARS && !spdx.chars().any(char::is_whitespace);
    if !shaped {
        return Err(ProvenanceError::BadSpdx {
            asset: asset.into(),
            spdx: spdx.into(),
        });
    }
    if is_allowed_spdx(spdx) {
        Ok(())
    } else {
        Err(ProvenanceError::LicenseRefused {
            asset: asset.into(),
            spdx: spdx.into(),
        })
    }
}

fn check_sha(asset: &str, sha: &str) -> Result<(), ProvenanceError> {
    if is_digest(sha) {
        Ok(())
    } else {
        Err(ProvenanceError::BadSha {
            asset: asset.into(),
        })
    }
}

fn ai_link(asset: &str, field: &'static str) -> ProvenanceError {
    ProvenanceError::AiChain {
        asset: asset.into(),
        field,
    }
}

fn has_text(value: Option<&str>) -> bool {
    matches!(value, Some(text) if !text.trim().is_empty())
}

/// Check one record: the hand-made chain or the AI chain, never a mix
/// with gaps. The first gap wins and names its asset.
pub fn check_record(record: &ProvenanceRecord) -> Result<(), ProvenanceError> {
    check_name(&record.name)?;
    check_spdx(&record.name, &record.spdx)?;
    check_sha(&record.name, &record.sha)?;
    let generator = record.generator.as_deref().unwrap_or("");
    if generator.trim().is_empty() {
        if has_text(record.model.as_deref()) || has_text(record.prompt_digest.as_deref()) {
            return Err(ai_link(&record.name, "generator"));
        }
        if record.url.trim().is_empty()
            || record.url.len() > MAX_FIELD_CHARS
            || !record.url.contains("://")
            || record.url.chars().any(char::is_whitespace)
        {
            return Err(ProvenanceError::MissingUrl {
                asset: record.name.clone(),
            });
        }
        if record.author.trim().is_empty() || record.author.len() > MAX_FIELD_CHARS {
            return Err(ProvenanceError::MissingAuthor {
                asset: record.name.clone(),
            });
        }
        return Ok(());
    }
    if generator.len() > MAX_FIELD_CHARS {
        return Err(ai_link(&record.name, "generator"));
    }
    match record.model.as_deref() {
        Some(model) if !model.trim().is_empty() && model.len() <= MAX_FIELD_CHARS => {}
        _ => return Err(ai_link(&record.name, "model")),
    }
    match record.prompt_digest.as_deref() {
        Some(digest) if is_digest(digest) => {}
        _ => return Err(ai_link(&record.name, "prompt_digest")),
    }
    Ok(())
}

/// Check a whole release in slice order: the first gap wins and names its
/// asset. An empty release passes vacuous, like an empty registry.
pub fn check_release(records: &[ProvenanceRecord]) -> Result<(), ProvenanceError> {
    if records.len() > MAX_RECORDS {
        return Err(ProvenanceError::TooMany {
            count: records.len(),
            max: MAX_RECORDS,
        });
    }
    let mut seen = BTreeSet::new();
    for record in records {
        if !seen.insert(record.name.as_str()) {
            return Err(ProvenanceError::Duplicate {
                asset: record.name.clone(),
            });
        }
        check_record(record)?;
    }
    Ok(())
}

fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (u32::from(c)) < 0x20 => {
                let _ = fmt::Write::write_fmt(out, format_args!("\\u{:04x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
}

fn push_field(out: &mut String, key: &str, value: &str) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":\"");
    push_escaped(out, value);
    out.push('"');
}

fn push_opt_field(out: &mut String, key: &str, value: Option<&str>) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    match value {
        Some(text) => {
            out.push('"');
            push_escaped(out, text);
            out.push('"');
        }
        None => out.push_str("null"),
    }
}

/// Encode one record as a canonical JSON row with fixed key order, so the
/// same record encodes the same bytes twice. Returns the row, never fails.
pub fn provenance_row(record: &ProvenanceRecord) -> String {
    let mut out = String::from("{");
    push_field(&mut out, "author", &record.author);
    out.push(',');
    push_opt_field(&mut out, "generator", record.generator.as_deref());
    out.push(',');
    push_opt_field(&mut out, "model", record.model.as_deref());
    out.push(',');
    push_field(&mut out, "name", &record.name);
    out.push(',');
    push_opt_field(&mut out, "prompt_digest", record.prompt_digest.as_deref());
    out.push(',');
    push_field(&mut out, "sha", &record.sha);
    out.push(',');
    push_field(&mut out, "spdx", &record.spdx);
    out.push(',');
    push_field(&mut out, "url", &record.url);
    out.push('}');
    out
}

/// Digest a whole release in slice order: encode every row, hash the bytes.
/// Same records give the same digest twice; order changes move it.
pub fn release_digest(records: &[ProvenanceRecord]) -> u64 {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend_from_slice(provenance_row(record).as_bytes());
    }
    hash_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hand(name: &str) -> ProvenanceRecord {
        ProvenanceRecord {
            name: name.into(),
            url: "https://example.test/pack/sand.glb".into(),
            spdx: "CC0-1.0".into(),
            author: "Example Studio".into(),
            sha: "4bdc600a9f2e71c8d3a5b6e0f1c2a3b4".into(),
            generator: None,
            model: None,
            prompt_digest: None,
        }
    }

    fn ai(name: &str) -> ProvenanceRecord {
        ProvenanceRecord {
            name: name.into(),
            url: String::new(),
            spdx: "Apache-2.0".into(),
            author: String::new(),
            sha: "0123456789abcdef0123456789abcdef".into(),
            generator: Some("vast-ai/triposg".into()),
            model: Some("triposg-1.0".into()),
            prompt_digest: Some("fedcba9876543210fedcba9876543210".into()),
        }
    }

    #[test]
    fn provenance_hand_record_passes() {
        assert_eq!(check_record(&hand("content/arena/sand.glb")), Ok(()));
        assert_eq!(check_record(&ai("content/props/ai/urn.glb")), Ok(()));
        assert_eq!(check_release(&[]), Ok(()));
    }

    #[test]
    fn provenance_each_hand_gap_is_named() {
        let mut record = hand("content/arena/sand.glb");
        record.url.clear();
        let err = check_record(&record).expect_err("empty URL refuses");
        assert_eq!(err.code(), "missing_url");
        assert!(err.to_string().contains("content/arena/sand.glb"));

        let mut record = hand("content/arena/sand.glb");
        record.author = "   ".into();
        assert_eq!(
            check_record(&record)
                .expect_err("blank author refuses")
                .code(),
            "missing_author"
        );

        let mut record = hand("content/arena/sand.glb");
        record.sha = "xy".into();
        assert_eq!(
            check_record(&record)
                .expect_err("short digest refuses")
                .code(),
            "bad_sha"
        );

        let mut record = hand("content/arena/sand.glb");
        record.spdx = "CC0 1.0".into();
        assert_eq!(
            check_record(&record)
                .expect_err("spaced licence refuses")
                .code(),
            "bad_spdx"
        );

        for licence in ["GPL-3.0-only", "CC-BY-NC-4.0", "LicenseRef"] {
            let mut record = hand("content/arena/sand.glb");
            record.spdx = licence.into();
            let err = check_record(&record).expect_err("off-list licence refuses");
            assert_eq!(err.code(), "license_refused");
            assert!(err.to_string().contains(licence));
        }
    }

    #[test]
    fn provenance_names_stay_lowercase_paths() {
        for name in ["", "Content/arena/sand.glb", "content/arena/sand glb"] {
            let err = check_record(&hand(name)).expect_err("bad name refuses");
            assert_eq!(err.code(), "bad_name");
        }
        let long = "content/".to_string() + &"a".repeat(MAX_NAME_CHARS);
        assert_eq!(
            check_record(&hand(&long))
                .expect_err("long name refuses")
                .code(),
            "bad_name"
        );
    }

    #[test]
    fn provenance_digests_stay_lowercase_hex() {
        for sha in [
            "4BDC600A9F2E71C8D3A5B6E0F1C2A3B4",
            "4bdc600a",
            "zzdc600a9f2e71c8d3a5b6e0f1c2a3b4",
        ] {
            let mut record = hand("content/arena/sand.glb");
            record.sha = sha.into();
            assert_eq!(
                check_record(&record)
                    .expect_err("bad digest refuses")
                    .code(),
                "bad_sha"
            );
        }
    }

    #[test]
    fn provenance_ai_chain_links_are_each_required() {
        let mut record = ai("content/props/ai/urn.glb");
        record.model = None;
        let err = check_record(&record).expect_err("model-less chain refuses");
        assert_eq!(err.code(), "ai_chain");
        assert!(err.to_string().contains("model"));

        let mut record = ai("content/props/ai/urn.glb");
        record.prompt_digest = Some("short".into());
        let err = check_record(&record).expect_err("short digest refuses");
        assert_eq!(err.code(), "ai_chain");
        assert!(err.to_string().contains("prompt_digest"));

        let mut record = hand("content/arena/sand.glb");
        record.model = Some("triposg-1.0".into());
        assert_eq!(
            check_record(&record)
                .expect_err("model without generator refuses")
                .code(),
            "ai_chain"
        );
    }

    #[test]
    fn provenance_release_matrix_of_fifty_names_the_gap() {
        let mut records = Vec::new();
        for i in 0..50 {
            records.push(hand(&format!("content/props/prop-{i:02}")));
        }
        assert_eq!(check_release(&records), Ok(()));
        assert_eq!(check_release(&records), check_release(&records));

        records[37].url.clear();
        let err = check_release(&records).expect_err("gap at 37 refuses");
        assert_eq!(err.code(), "missing_url");
        assert!(
            err.to_string().contains("content/props/prop-37"),
            "names the gapped asset, got {err}"
        );

        records[37].url = "https://example.test/pack/prop-37.glb".into();
        records.push(hand("content/props/prop-00"));
        let err = check_release(&records).expect_err("repeat refuses");
        assert_eq!(err.code(), "duplicate");
        assert!(err.to_string().contains("content/props/prop-00"));
    }

    #[test]
    fn provenance_rows_and_digest_repeat_bit_identical() {
        let records: Vec<ProvenanceRecord> =
            (0..8).map(|i| hand(&format!("content/a-{i}"))).collect();
        let first: Vec<String> = records.iter().map(provenance_row).collect();
        let second: Vec<String> = records.iter().map(provenance_row).collect();
        assert_eq!(first, second);
        assert_eq!(release_digest(&records), release_digest(&records));

        let row = &first[0];
        let author = row.find("\"author\"").expect("author key first");
        let url = row.find("\"url\"").expect("url key last");
        assert!(author < url, "keys encode in fixed order, got {row}");

        let mut swapped = records.clone();
        swapped.swap(0, 1);
        assert_ne!(
            release_digest(&records),
            release_digest(&swapped),
            "order moves the digest"
        );
    }

    #[cfg(all(feature = "import", feature = "registry"))]
    #[test]
    fn provenance_allow_list_matches_intake() {
        assert_eq!(ALLOWED_SPDX, crate::gen_intake::ALLOWED_SPDX);
    }
}
