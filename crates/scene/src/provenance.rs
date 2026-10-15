//! provenance: AI-made content records. Source (human, script, model),
//! provider and model ID, request digest, verifier results and parent receipt
//! IDs. No credentials, no raw prompts, no provider types. The binary codec
//! below is always available; canonical JSON rides the `json` feature for
//! authored-file tooling. The editor fills these records through commands.

use engine_core::hash_bytes;

/// Provenance schema version; gated on decode, stamped on encode.
pub const PROVENANCE_VERSION: u16 = 1;
/// Oldest provenance version that migrates forward.
pub const MIN_PROVENANCE_VERSION: u16 = 1;
/// Largest accepted provenance document (binary or JSON) in bytes.
pub const MAX_PROVENANCE_BYTES: usize = 64 << 10;
/// Maximum verifier results per record.
pub const MAX_VERIFIERS: usize = 8;
/// Maximum parent receipt ids per record.
pub const MAX_PARENTS: usize = 8;
/// Maximum source/model/verifier text length.
pub const MAX_TEXT_LEN: usize = 256;

/// Where a change came from. Model records carry provider and model IDs
/// only — never prompts, credentials or session secrets.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// Human editor action.
    Human,
    /// Script or automation, named.
    Script(String),
    /// Model proposal through the provider service.
    Model {
        /// Provider ID (e.g. adapter name), bounded.
        provider: String,
        /// Model ID, bounded.
        model: String,
    },
}

/// One verifier outcome attached to a record.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VerifierResult {
    /// Verifier name, bounded.
    pub verifier: String,
    /// Whether the verifier passed.
    pub passed: bool,
    /// Bounded detail line.
    pub detail: String,
}

/// One provenance record for an object or artifact version.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Provenance {
    /// Schema version (always [`PROVENANCE_VERSION`] on encode).
    pub version: u16,
    /// What made this change.
    pub source: Source,
    /// Digest of the originating request (zeros for human edits).
    pub request_digest: [u8; 32],
    /// Digest of the artifact version this record describes.
    pub artifact_digest: [u8; 32],
    /// Verifier outcomes (`<= MAX_VERIFIERS`).
    pub verifiers: Vec<VerifierResult>,
    /// Parent receipt ids forming the per-object chain.
    pub parents: Vec<u64>,
}

impl Provenance {
    /// Canonical receipt id: FNV-1a over the canonical bytes.
    pub fn receipt(&self) -> u64 {
        hash_bytes(&self.canonical())
    }

    /// Canonical little-endian bytes: version, source tag + texts,
    /// digests, verifier rows, parents.
    pub fn canonical(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.version.to_le_bytes());
        match &self.source {
            Source::Human => bytes.push(0),
            Source::Script(name) => {
                bytes.push(1);
                push_text(&mut bytes, name);
            }
            Source::Model { provider, model } => {
                bytes.push(2);
                push_text(&mut bytes, provider);
                push_text(&mut bytes, model);
            }
        }
        bytes.extend_from_slice(&self.request_digest);
        bytes.extend_from_slice(&self.artifact_digest);
        bytes.push(self.verifiers.len() as u8);
        for verifier in &self.verifiers {
            push_text(&mut bytes, &verifier.verifier);
            bytes.push(u8::from(verifier.passed));
            push_text(&mut bytes, &verifier.detail);
        }
        bytes.push(self.parents.len() as u8);
        for parent in &self.parents {
            bytes.extend_from_slice(&parent.to_le_bytes());
        }
        bytes
    }

    /// Validate bounds: text lengths, verifier/parent caps.
    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.version != PROVENANCE_VERSION {
            return Err(ProvenanceError::UnsupportedVersion {
                got: self.version,
                min: MIN_PROVENANCE_VERSION,
                current: PROVENANCE_VERSION,
            });
        }
        match &self.source {
            Source::Human => {}
            Source::Script(name) => check_text(name)?,
            Source::Model { provider, model } => {
                check_text(provider)?;
                check_text(model)?;
            }
        }
        if self.verifiers.len() > MAX_VERIFIERS || self.parents.len() > MAX_PARENTS {
            return Err(ProvenanceError::TooLong);
        }
        for verifier in &self.verifiers {
            check_text(&verifier.verifier)?;
            check_text(&verifier.detail)?;
        }
        Ok(())
    }

    /// Encode: validate, then version tag plus canonical bytes.
    pub fn encode(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.validate()?;
        Ok(self.canonical())
    }

    /// Decode: length gate, version gate, structural parse, validation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProvenanceError> {
        if bytes.len() > MAX_PROVENANCE_BYTES {
            return Err(ProvenanceError::TooLong);
        }
        let mut reader = Reader { bytes, pos: 0 };
        let version = reader.u16()?;
        if version != PROVENANCE_VERSION {
            return Err(ProvenanceError::UnsupportedVersion {
                got: version,
                min: MIN_PROVENANCE_VERSION,
                current: PROVENANCE_VERSION,
            });
        }
        let source = match reader.u8()? {
            0 => Source::Human,
            1 => Source::Script(reader.text()?),
            2 => {
                let provider = reader.text()?;
                Source::Model {
                    model: reader.text()?,
                    provider,
                }
            }
            _ => return Err(ProvenanceError::Truncated),
        };
        let mut request_digest = [0u8; 32];
        request_digest.copy_from_slice(reader.take(32)?);
        let mut artifact_digest = [0u8; 32];
        artifact_digest.copy_from_slice(reader.take(32)?);
        let verifiers_len = usize::from(reader.u8()?);
        if verifiers_len > MAX_VERIFIERS {
            return Err(ProvenanceError::TooLong);
        }
        let mut verifiers = Vec::with_capacity(verifiers_len);
        for _ in 0..verifiers_len {
            verifiers.push(VerifierResult {
                verifier: reader.text()?,
                passed: reader.u8()? != 0,
                detail: reader.text()?,
            });
        }
        let parents_len = usize::from(reader.u8()?);
        if parents_len > MAX_PARENTS {
            return Err(ProvenanceError::TooLong);
        }
        let mut parents = Vec::with_capacity(parents_len);
        for _ in 0..parents_len {
            parents.push(reader.u64()?);
        }
        if reader.pos != bytes.len() {
            return Err(ProvenanceError::Truncated);
        }
        let record = Provenance {
            version,
            source,
            request_digest,
            artifact_digest,
            verifiers,
            parents,
        };
        record.validate()?;
        Ok(record)
    }

    /// Migrate a decoded record forward. v1 is current, so migration is a
    /// checked stamp: only records at a supported version pass.
    pub fn migrate(&mut self) -> Result<(), ProvenanceError> {
        if self.version != PROVENANCE_VERSION {
            return Err(ProvenanceError::UnsupportedVersion {
                got: self.version,
                min: MIN_PROVENANCE_VERSION,
                current: PROVENANCE_VERSION,
            });
        }
        Ok(())
    }
}

/// Per-object provenance chain: records linked by parent receipt ids.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProvenanceChain {
    /// Records in append order; each record's parents name earlier receipts.
    pub records: Vec<Provenance>,
}

impl ProvenanceChain {
    /// Append a record: every parent must name an earlier receipt (or be the
    /// chain's first record with no parents).
    pub fn append(&mut self, record: Provenance) -> Result<u64, ProvenanceError> {
        record.validate()?;
        if self.records.is_empty() {
            if !record.parents.is_empty() {
                return Err(ProvenanceError::BadParent);
            }
        } else {
            let known: Vec<u64> = self.records.iter().map(|r| r.receipt()).collect();
            if record.parents.is_empty() || record.parents.iter().any(|p| !known.contains(p)) {
                return Err(ProvenanceError::BadParent);
            }
        }
        let receipt = record.receipt();
        self.records.push(record);
        Ok(receipt)
    }

    /// Verify every link: validation plus parent membership, in order.
    pub fn verify(&self) -> Result<(), ProvenanceError> {
        let mut known: Vec<u64> = Vec::new();
        for record in &self.records {
            record.validate()?;
            if known.is_empty() {
                if !record.parents.is_empty() {
                    return Err(ProvenanceError::BadParent);
                }
            } else if record.parents.is_empty() || record.parents.iter().any(|p| !known.contains(p))
            {
                return Err(ProvenanceError::BadParent);
            }
            known.push(record.receipt());
        }
        Ok(())
    }
}

/// Provenance failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProvenanceError {
    /// Version outside the supported range.
    UnsupportedVersion {
        /// Version found.
        got: u16,
        /// Oldest migratable version.
        min: u16,
        /// Current version.
        current: u16,
    },
    /// Truncated input or trailing bytes.
    Truncated,
    /// Oversize document, collection or text.
    TooLong,
    /// Parent receipt names no earlier record.
    BadParent,
    /// String is not valid UTF-8.
    BadUtf8,
}

fn check_text(text: &str) -> Result<(), ProvenanceError> {
    if text.len() > MAX_TEXT_LEN {
        return Err(ProvenanceError::TooLong);
    }
    Ok(())
}

fn push_text(bytes: &mut Vec<u8>, text: &str) {
    bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ProvenanceError> {
        let end = self.pos.saturating_add(count);
        if end > self.bytes.len() {
            return Err(ProvenanceError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ProvenanceError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProvenanceError> {
        let mut array = [0u8; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    fn u16(&mut self) -> Result<u16, ProvenanceError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ProvenanceError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn text(&mut self) -> Result<String, ProvenanceError> {
        let len = usize::from(self.u16()?);
        if len > MAX_TEXT_LEN {
            return Err(ProvenanceError::TooLong);
        }
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes)
            .map_err(|_| ProvenanceError::BadUtf8)
            .map(|text| text.to_owned())
    }
}

/// Canonical JSON codec for authored-file tooling (`json` feature).
/// Same version gate and bounds as the binary codec; no secrets ever
/// serialize because no such field exists on the types.
#[cfg(feature = "json")]
pub mod json {
    use super::{Provenance, ProvenanceError, MAX_PROVENANCE_BYTES};

    /// Encode a record as canonical JSON text.
    pub fn encode(record: &Provenance) -> Result<String, ProvenanceError> {
        record.validate()?;
        serde_json::to_string(&record_as_value(record)).map_err(|_| ProvenanceError::TooLong)
    }

    /// Decode JSON text with a byte-size gate first.
    pub fn decode(text: &str) -> Result<Provenance, ProvenanceError> {
        if text.len() > MAX_PROVENANCE_BYTES {
            return Err(ProvenanceError::TooLong);
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|_| ProvenanceError::Truncated)?;
        record_from_value(&value)
    }

    fn record_as_value(record: &Provenance) -> serde_json::Value {
        let (source, provider, model) = match &record.source {
            super::Source::Human => ("human", String::new(), String::new()),
            super::Source::Script(name) => ("script", name.clone(), String::new()),
            super::Source::Model { provider, model } => ("model", provider.clone(), model.clone()),
        };
        serde_json::json!({
            "version": record.version,
            "source": source,
            "provider": provider,
            "model": model,
            "request_digest": hex(&record.request_digest),
            "artifact_digest": hex(&record.artifact_digest),
            "verifiers": record.verifiers.iter().map(|v| serde_json::json!({
                "verifier": v.verifier, "passed": v.passed, "detail": v.detail
            })).collect::<Vec<_>>(),
            "parents": record.parents,
        })
    }

    fn record_from_value(value: &serde_json::Value) -> Result<Provenance, ProvenanceError> {
        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or(ProvenanceError::Truncated)? as u16;
        if version != super::PROVENANCE_VERSION {
            return Err(ProvenanceError::UnsupportedVersion {
                got: version,
                min: super::MIN_PROVENANCE_VERSION,
                current: super::PROVENANCE_VERSION,
            });
        }
        let source_name = value
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or(ProvenanceError::Truncated)?;
        let text_field = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_str())
                .ok_or(ProvenanceError::Truncated)
        };
        let source = match source_name {
            "human" => super::Source::Human,
            "script" => super::Source::Script(text_field("provider")?.to_owned()),
            "model" => super::Source::Model {
                provider: text_field("provider")?.to_owned(),
                model: text_field("model")?.to_owned(),
            },
            _ => return Err(ProvenanceError::Truncated),
        };
        let digest_field = |key: &str| {
            let text = text_field(key)?;
            if text.len() != 64 {
                return Err(ProvenanceError::Truncated);
            }
            let mut digest = [0u8; 32];
            for (index, byte) in digest.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                    .map_err(|_| ProvenanceError::Truncated)?;
            }
            Ok(digest)
        };
        let verifiers = value
            .get("verifiers")
            .and_then(|v| v.as_array())
            .ok_or(ProvenanceError::Truncated)?;
        if verifiers.len() > super::MAX_VERIFIERS {
            return Err(ProvenanceError::TooLong);
        }
        let mut rows = Vec::with_capacity(verifiers.len());
        for entry in verifiers {
            rows.push(super::VerifierResult {
                verifier: entry
                    .get("verifier")
                    .and_then(|v| v.as_str())
                    .ok_or(ProvenanceError::Truncated)?
                    .to_owned(),
                passed: entry
                    .get("passed")
                    .and_then(|v| v.as_bool())
                    .ok_or(ProvenanceError::Truncated)?,
                detail: entry
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .ok_or(ProvenanceError::Truncated)?
                    .to_owned(),
            });
        }
        let parents = value
            .get("parents")
            .and_then(|v| v.as_array())
            .ok_or(ProvenanceError::Truncated)?;
        if parents.len() > super::MAX_PARENTS {
            return Err(ProvenanceError::TooLong);
        }
        let mut links = Vec::with_capacity(parents.len());
        for parent in parents {
            links.push(parent.as_u64().ok_or(ProvenanceError::Truncated)?);
        }
        let record = Provenance {
            version,
            source,
            request_digest: digest_field("request_digest")?,
            artifact_digest: digest_field("artifact_digest")?,
            verifiers: rows,
            parents: links,
        };
        record.validate()?;
        Ok(record)
    }

    fn hex(digest: &[u8; 32]) -> String {
        // Nibble lookup renders the canonical lowercase hex directly.
        const NIBBLE: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(64);
        for byte in digest {
            text.push(NIBBLE[usize::from(byte >> 4)] as char);
            text.push(NIBBLE[usize::from(byte & 0x0f)] as char);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Provenance {
        Provenance {
            version: PROVENANCE_VERSION,
            source: Source::Model {
                provider: "mock".into(),
                model: "mock-1".into(),
            },
            request_digest: [7u8; 32],
            artifact_digest: [9u8; 32],
            verifiers: vec![VerifierResult {
                verifier: "spatial".into(),
                passed: true,
                detail: "clean".into(),
            }],
            parents: Vec::new(),
        }
    }

    #[test]
    fn binary_codec_roundtrips() {
        let bytes = record().encode().unwrap();
        let again = Provenance::decode(&bytes).unwrap();
        assert_eq!(again, record());
        assert_eq!(again.receipt(), record().receipt());
    }

    #[test]
    fn corrupt_inputs_fail_bounded() {
        assert_eq!(Provenance::decode(&[]), Err(ProvenanceError::Truncated));
        let mut bad = record().encode().unwrap();
        bad[0] = 0xFF;
        assert!(matches!(
            Provenance::decode(&bad),
            Err(ProvenanceError::UnsupportedVersion { .. })
        ));
        let mut truncated = record().encode().unwrap();
        truncated.pop();
        assert_eq!(
            Provenance::decode(&truncated),
            Err(ProvenanceError::Truncated)
        );
        let oversize = vec![0u8; MAX_PROVENANCE_BYTES + 1];
        assert_eq!(Provenance::decode(&oversize), Err(ProvenanceError::TooLong));
    }

    #[test]
    fn chain_links_verify() {
        let mut chain = ProvenanceChain::default();
        let first = chain.append(record()).unwrap();
        let mut second = record();
        second.parents.push(first);
        second.artifact_digest = [10u8; 32];
        chain.append(second).unwrap();
        chain.verify().unwrap();
        let mut orphan = record();
        orphan.parents.push(0xDEAD);
        assert_eq!(chain.append(orphan), Err(ProvenanceError::BadParent));
    }

    #[test]
    fn migration_is_a_checked_stamp() {
        let mut record = record();
        record.migrate().unwrap();
        record.version = 99;
        assert!(matches!(
            record.migrate(),
            Err(ProvenanceError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    #[cfg(feature = "json")]
    fn json_codec_roundtrips() {
        let text = json::encode(&record()).unwrap();
        let again = json::decode(&text).unwrap();
        assert_eq!(again, record());
        assert_eq!(json::decode("{oops"), Err(ProvenanceError::Truncated));
        let oversize = " ".repeat(MAX_PROVENANCE_BYTES + 1);
        assert_eq!(json::decode(&oversize), Err(ProvenanceError::TooLong));
    }
}
