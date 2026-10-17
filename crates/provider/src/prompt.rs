//! prompt: system prompts as versioned, content-addressed assets with an eval
//! gate.
//!
//! A [`PromptAsset`] is a prompt text with a name, a version, a provenance
//! line and the eval set that judges it. Its address is [`text_digest`] of
//! the text, so the same words always have the same address and any edit
//! moves it. The digest of the system prompt a request actually carried lands
//! in the [`crate::ladder::LadderReport`] of every HTTP exchange, so a
//! recorded answer says which prompt produced it.
//!
//! [`gate_edit`] is the edit gate: it runs the new asset's evals against both
//! the old text and the new text through an [`EvalModel`] and refuses an edit
//! that lowers the score, keeps the version, or renames the asset. Nothing
//! here dials a network: the model is whatever the caller passes (a cassette
//! replay in CI, a scripted model in tests), and scores are integer permille,
//! so the same series gives the same verdict.
//!
//! Units: scores in permille of expected phrases found. Errors:
//! [`PromptError`].

use engine_core::hash_bytes;

use super::cassette::kind_name;
use super::pins::default_system_prompt;
use super::protocol::TaskKind;
use super::MAX_TEXT_LEN;

/// Most evals one asset may carry.
pub const MAX_EVALS: usize = 64;

/// Every task kind that has a built-in prompt, in a fixed order.
pub const BUILTIN_KINDS: [TaskKind; 6] = [
    TaskKind::Inspect,
    TaskKind::Propose,
    TaskKind::Diagnose,
    TaskKind::Repair,
    TaskKind::Announce,
    TaskKind::Taunt,
];

/// The address of a prompt text: FNV-1a 64 over its bytes.
#[must_use]
pub fn text_digest(text: &str) -> u64 {
    hash_bytes(text.as_bytes())
}

/// Why an asset or an edit was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PromptError {
    /// The name or the text is empty, or the text is longer than the cap.
    BadText,
    /// Versions start at 1.
    BadVersion,
    /// An asset needs one to [`MAX_EVALS`] evals, each with a phrase to find.
    BadEvals,
    /// An edit may not rename the asset it edits.
    NameChanged,
    /// The text changed but the version did not go up.
    VersionNotRaised {
        /// Version before the edit.
        old: u32,
        /// Version the edit carries.
        new: u32,
    },
    /// The edit scored lower than the text it replaces.
    ScoreDropped {
        /// Score of the old text, permille.
        old_permille: u32,
        /// Score of the new text, permille.
        new_permille: u32,
    },
}

impl core::fmt::Display for PromptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadText => f.write_str("prompt name or text empty or too long"),
            Self::BadVersion => f.write_str("prompt version starts at 1"),
            Self::BadEvals => f.write_str("prompt needs 1..=64 evals with a phrase each"),
            Self::NameChanged => f.write_str("a prompt edit may not rename the asset"),
            Self::VersionNotRaised { old, new } => {
                write!(
                    f,
                    "prompt text changed but version stayed {new} (was {old})"
                )
            }
            Self::ScoreDropped {
                old_permille,
                new_permille,
            } => write!(
                f,
                "prompt edit lowers its eval score from {old_permille} to {new_permille} permille"
            ),
        }
    }
}

impl std::error::Error for PromptError {}

/// One eval: ask with `input`; every phrase in `must_contain` should appear
/// in the answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PromptEval {
    /// What the model is asked under the prompt.
    pub input: String,
    /// Phrases a good answer holds.
    pub must_contain: Vec<String>,
}

/// A prompt text with its name, version, provenance and eval set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PromptAsset {
    name: String,
    version: u32,
    text: String,
    provenance: String,
    evals: Vec<PromptEval>,
}

impl PromptAsset {
    /// Build an asset.
    /// Errors: [`PromptError::BadText`], [`PromptError::BadVersion`],
    /// [`PromptError::BadEvals`].
    pub fn new(
        name: &str,
        version: u32,
        text: &str,
        provenance: &str,
        evals: Vec<PromptEval>,
    ) -> Result<Self, PromptError> {
        if name.is_empty() || text.is_empty() || text.len() > MAX_TEXT_LEN {
            return Err(PromptError::BadText);
        }
        if version == 0 {
            return Err(PromptError::BadVersion);
        }
        let judged = evals.iter().all(|eval| !eval.must_contain.is_empty());
        if evals.is_empty() || evals.len() > MAX_EVALS || !judged {
            return Err(PromptError::BadEvals);
        }
        Ok(Self {
            name: name.to_owned(),
            version,
            text: text.to_owned(),
            provenance: provenance.to_owned(),
            evals,
        })
    }

    /// The built-in system prompt of `kind` at version 1: the text
    /// [`default_system_prompt`] serves, judged by two evals (it states the
    /// reply grammar, and it names the role).
    #[must_use]
    pub fn builtin(kind: TaskKind) -> Self {
        let role = kind_name(kind);
        let evals = vec![
            PromptEval {
                input: String::from("State the reply shape."),
                must_contain: ["noop", "set_position", "set_label", "diagnostics"]
                    .map(String::from)
                    .to_vec(),
            },
            PromptEval {
                input: String::from("State your role."),
                must_contain: vec![role.to_owned()],
            },
        ];
        Self {
            name: format!("system.{role}"),
            version: 1,
            text: default_system_prompt(kind).to_owned(),
            provenance: String::from("engine2040 pins::default_system_prompt"),
            evals,
        }
    }

    /// Asset name, stable across versions.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version, 1 or more.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The prompt text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the text came from.
    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    /// The eval set that judges this asset.
    #[must_use]
    pub fn evals(&self) -> &[PromptEval] {
        &self.evals
    }

    /// Content address: [`text_digest`] of the text.
    #[must_use]
    pub fn digest(&self) -> u64 {
        text_digest(&self.text)
    }
}

/// The model an eval asks. Offline in CI: a cassette replay or a script.
pub trait EvalModel {
    /// The model's answer to `input` under the system prompt `system`.
    fn answer(&mut self, system: &str, input: &str) -> String;
}

/// Score `text` on `evals`: permille of expected phrases the answers hold.
/// Evals with no phrase make the score 0, never a divide.
pub fn score_permille(text: &str, evals: &[PromptEval], model: &mut impl EvalModel) -> u32 {
    let (mut found, mut wanted) = (0u64, 0u64);
    for eval in evals {
        let answer = model.answer(text, &eval.input);
        for phrase in &eval.must_contain {
            wanted += 1;
            found += u64::from(answer.contains(phrase.as_str()));
        }
    }
    u32::try_from(found * 1000 / wanted.max(1)).unwrap_or(u32::MAX)
}

/// What a passing edit scored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EditReport {
    /// Score of the old text on the new asset's evals, permille.
    pub old_permille: u32,
    /// Score of the new text, permille.
    pub new_permille: u32,
    /// Address of the old text.
    pub old_digest: u64,
    /// Address of the new text.
    pub new_digest: u64,
}

/// Gate an edit of `old` into `new`: the same asset, a raised version when
/// the text moved, and an eval score that does not drop. Both texts run the
/// new asset's evals, so an edit cannot hide a loss by trimming its evals.
/// Errors: [`PromptError::NameChanged`], [`PromptError::VersionNotRaised`],
/// [`PromptError::ScoreDropped`].
pub fn gate_edit(
    old: &PromptAsset,
    new: &PromptAsset,
    model: &mut impl EvalModel,
) -> Result<EditReport, PromptError> {
    if old.name != new.name {
        return Err(PromptError::NameChanged);
    }
    let moved = old.digest() != new.digest();
    if moved && new.version <= old.version {
        return Err(PromptError::VersionNotRaised {
            old: old.version,
            new: new.version,
        });
    }
    let old_permille = score_permille(&old.text, &new.evals, model);
    let new_permille = score_permille(&new.text, &new.evals, model);
    if new_permille < old_permille {
        return Err(PromptError::ScoreDropped {
            old_permille,
            new_permille,
        });
    }
    Ok(EditReport {
        old_permille,
        new_permille,
        old_digest: old.digest(),
        new_digest: new.digest(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model that follows its prompt exactly: it answers with the prompt.
    /// A phrase the prompt lacks is a phrase the answer lacks.
    struct Echo;

    impl EvalModel for Echo {
        fn answer(&mut self, system: &str, _input: &str) -> String {
            system.to_owned()
        }
    }

    fn edited(old: &PromptAsset, version: u32, text: &str) -> PromptAsset {
        PromptAsset::new(old.name(), version, text, "test edit", old.evals().to_vec())
            .expect("edited asset is well formed")
    }

    #[test]
    fn builtin_prompts_are_addressed_and_pass_their_own_evals() {
        let mut seen = Vec::new();
        for kind in BUILTIN_KINDS {
            let asset = PromptAsset::builtin(kind);
            assert_eq!(asset.text(), default_system_prompt(kind));
            assert_eq!(asset.digest(), text_digest(default_system_prompt(kind)));
            assert_eq!(
                score_permille(asset.text(), asset.evals(), &mut Echo),
                1000,
                "{kind:?} meets its own evals"
            );
            seen.push(asset.digest());
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            BUILTIN_KINDS.len(),
            "each kind has its own address"
        );
    }

    /// F2P: a seeded score-dropping edit (the grammar sentence is cut) fails
    /// the gate; a harmless reword that keeps every phrase passes; the same
    /// verdict comes back twice.
    #[test]
    fn a_seeded_score_dropping_edit_fails_the_gate() {
        let old = PromptAsset::builtin(TaskKind::Propose);
        let cut = old
            .text()
            .split(" Reply with exactly")
            .next()
            .expect("role line");
        let bad = edited(&old, 2, cut);
        let verdict = gate_edit(&old, &bad, &mut Echo);
        assert_eq!(
            verdict,
            Err(PromptError::ScoreDropped {
                old_permille: 1000,
                new_permille: 200,
            })
        );
        assert_eq!(gate_edit(&old, &bad, &mut Echo), verdict, "same verdict x2");

        let reworded = edited(&old, 2, &format!("{} Be brief.", old.text()));
        let report = gate_edit(&old, &reworded, &mut Echo).expect("a harmless edit passes");
        assert_eq!((report.old_permille, report.new_permille), (1000, 1000));
        assert_ne!(
            report.old_digest, report.new_digest,
            "an edit moves the address"
        );
    }

    #[test]
    fn an_edit_keeps_its_name_and_raises_its_version() {
        let old = PromptAsset::builtin(TaskKind::Taunt);
        let same_version = edited(&old, 1, &format!("{} Be brief.", old.text()));
        assert_eq!(
            gate_edit(&old, &same_version, &mut Echo),
            Err(PromptError::VersionNotRaised { old: 1, new: 1 })
        );
        let other = PromptAsset::builtin(TaskKind::Announce);
        assert_eq!(
            gate_edit(&old, &other, &mut Echo),
            Err(PromptError::NameChanged)
        );
        let again = gate_edit(&old, &old, &mut Echo).expect("no edit is not a drop");
        assert_eq!(again.old_digest, again.new_digest);
    }

    #[test]
    fn malformed_assets_are_refused() {
        let eval = PromptEval {
            input: String::from("q"),
            must_contain: vec![String::from("a")],
        };
        let new = |name: &str, version: u32, text: &str, evals: Vec<PromptEval>| {
            PromptAsset::new(name, version, text, "p", evals).err()
        };
        assert_eq!(
            new("", 1, "t", vec![eval.clone()]),
            Some(PromptError::BadText)
        );
        assert_eq!(
            new("n", 1, "", vec![eval.clone()]),
            Some(PromptError::BadText)
        );
        let huge = "x".repeat(MAX_TEXT_LEN + 1);
        assert_eq!(
            new("n", 1, &huge, vec![eval.clone()]),
            Some(PromptError::BadText)
        );
        assert_eq!(
            new("n", 0, "t", vec![eval.clone()]),
            Some(PromptError::BadVersion)
        );
        assert_eq!(new("n", 1, "t", Vec::new()), Some(PromptError::BadEvals));
        let empty = PromptEval {
            input: String::from("q"),
            must_contain: Vec::new(),
        };
        assert_eq!(new("n", 1, "t", vec![empty]), Some(PromptError::BadEvals));
        let many = vec![eval.clone(); MAX_EVALS + 1];
        assert_eq!(new("n", 1, "t", many), Some(PromptError::BadEvals));
        assert_eq!(new("n", 1, "t", vec![eval]), None);
    }
}
