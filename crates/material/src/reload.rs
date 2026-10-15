//! reload: live material edits that keep the last good compile.
//!
//! A [`Reloader`] holds the last record that compiled. [`Reloader::apply_text`]
//! takes new record text: it parses (typed refusal with a path), compiles,
//! checks the WGSL with naga when the `validate` feature is on, and only then
//! replaces the held state. Any failure returns the error and leaves the held
//! compile, its digests and its uniform exactly as they were, so the picture
//! never changes on a bad edit. The result says what the host must do:
//!
//! - `Unchanged`: nothing.
//! - `Values`: same structure; write the new uniform floats (a
//!   buffer write, no pipeline work, well inside 100 ms).
//! - `Structure`: new WGSL; build one pipeline (async, the old one
//!   keeps drawing until it is ready, the live pipeline swap) and write the uniform.

use crate::compile::{compile, Compiled};
use crate::record::{MaterialRecord, RecordError};

/// What a successful edit asks of the host.
#[derive(Clone, PartialEq, Debug)]
pub enum Reload {
    /// The compile is byte-equal to the held one.
    Unchanged,
    /// Same WGSL; only the uniform floats changed.
    Values {
        /// The new uniform stream.
        uniform: Vec<f32>,
    },
    /// New WGSL: a new pipeline is needed.
    Structure {
        /// The new compile (text, key, uniform).
        compiled: Box<Compiled>,
    },
}

/// Why an edit was refused. The held compile is untouched.
#[derive(Clone, PartialEq, Debug)]
pub enum ReloadError {
    /// The text did not parse as a record; the path names the field.
    Record(RecordError),
    /// The generated WGSL did not pass naga (an engine bug, never user data).
    Wgsl(crate::compile::WgslError),
}

impl std::fmt::Display for ReloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Record(e) => write!(f, "{e}"),
            Self::Wgsl(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReloadError {}

/// The held last-good state of one material slot.
#[derive(Clone, Default, Debug)]
pub struct Reloader {
    good: Option<(MaterialRecord, Compiled)>,
}

impl Reloader {
    /// An empty reloader: the first successful edit is a [`Reload::Structure`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The last record that compiled.
    #[must_use]
    pub fn record(&self) -> Option<&MaterialRecord> {
        self.good.as_ref().map(|(r, _)| r)
    }

    /// The last compile.
    #[must_use]
    pub fn compiled(&self) -> Option<&Compiled> {
        self.good.as_ref().map(|(_, c)| c)
    }

    /// Look digest of the held record: the number a capture of the last good
    /// frame is keyed by. `None` before the first success.
    #[must_use]
    pub fn look_digest(&self) -> Option<u64> {
        self.record().map(MaterialRecord::look_digest)
    }

    /// Apply new record text (stand-alone, or against `parent`).
    pub fn apply_text(
        &mut self,
        text: &str,
        parent: Option<&MaterialRecord>,
    ) -> Result<Reload, ReloadError> {
        let record = MaterialRecord::parse_with(text, parent).map_err(ReloadError::Record)?;
        self.apply(record)
    }

    /// Apply an already parsed record.
    pub fn apply(&mut self, record: MaterialRecord) -> Result<Reload, ReloadError> {
        let next = compile(&record);
        #[cfg(feature = "validate")]
        {
            // Only a new structure has new text to check.
            if self.good.as_ref().map(|(_, c)| c.key) != Some(next.key) {
                crate::compile::validate(&next.wgsl).map_err(ReloadError::Wgsl)?;
            }
        }
        let verdict = match &self.good {
            None => Reload::Structure {
                compiled: Box::new(next.clone()),
            },
            Some((_, held)) if held.key != next.key => Reload::Structure {
                compiled: Box::new(next.clone()),
            },
            Some((_, held)) if held.uniform == next.uniform => Reload::Unchanged,
            Some(_) => Reload::Values {
                uniform: next.uniform.clone(),
            },
        };
        self.good = Some((record, next));
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::MaterialSet;

    /// Timber (no wet layer, so no coat) with a roughness and an optional sheen.
    fn marble_text(roughness: &str, sheen: bool) -> String {
        let mut record = MaterialSet::siege_kit()
            .get("timber")
            .cloned()
            .expect("shipped");
        record.look.specular_roughness = roughness.parse().expect("number");
        if sheen {
            record.look.sheen = Some(crate::record::Sheen {
                weight: 0.5,
                color: [0.4, 0.4, 0.4],
                roughness: 0.5,
            });
        }
        record.to_text()
    }

    #[test]
    fn first_apply_is_a_structure_then_values_then_unchanged() {
        let mut r = Reloader::new();
        assert!(matches!(
            r.apply_text(&marble_text("0.35", false), None),
            Ok(Reload::Structure { .. })
        ));
        assert!(matches!(
            r.apply_text(&marble_text("0.35", false), None),
            Ok(Reload::Unchanged)
        ));
        assert!(matches!(
            r.apply_text(&marble_text("0.08", false), None),
            Ok(Reload::Values { .. })
        ));
        assert!(matches!(
            r.apply_text(&marble_text("0.08", true), None),
            Ok(Reload::Structure { .. })
        ));
    }

    #[test]
    fn three_broken_edits_are_refused_and_the_last_good_stays() {
        let mut r = Reloader::new();
        r.apply_text(&marble_text("0.35", false), None)
            .expect("good");
        let digest = r.look_digest();
        let uniform = r.compiled().map(|c| c.uniform.clone());
        for bad in [
            marble_text("1.7", false),
            marble_text("0.35", false).replacen("\"look\"", "\"lookk\"", 1),
            "{ not json".to_string(),
        ] {
            let err = r.apply_text(&bad, None).expect_err("refused");
            assert!(matches!(err, ReloadError::Record(_)));
        }
        assert_eq!(r.look_digest(), digest);
        assert_eq!(r.compiled().map(|c| c.uniform.clone()), uniform);
    }

    #[test]
    fn the_cpu_side_of_a_structure_edit_is_far_under_a_second() {
        let mut r = Reloader::new();
        r.apply_text(&marble_text("0.35", false), None)
            .expect("good");
        let start = std::time::Instant::now();
        for i in 0..20 {
            let with_coat = i % 2 == 0;
            r.apply_text(&marble_text("0.35", with_coat), None)
                .expect("edit");
        }
        // Compile plus naga (when on) for 20 structure edits.
        assert!(start.elapsed().as_millis() < 1000);
    }
}
