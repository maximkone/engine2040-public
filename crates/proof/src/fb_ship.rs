//! FB3 ship receipt: a template boots headless with zero coaching, and the
//! first-run budget gate holds.
//!
//! Consumer: the FB3 board row; the report lands in `target/evidence/b/ship/`.
//! Data owner: this module owns only the pinned fact list, the step budget
//! table and the report text. The default tier reads the owner sources (editor
//! onboarding and templates, UI menu flow) through `include_str!` and fails when
//! a pinned fact moves. The `m1` tier drives the editor's public template and
//! onboarding APIs headless: each template is applied, verified by a save and
//! reopen, and played through `play_ready`, and the checklist is fed events only.
//! The menu flow lives in `engine-ui`, which proof does not depend on, so UIX1 is
//! pinned by source text here and is not run.
//! Determinism scope: the same sources and the same owner calls give the same
//! report text. Errors: a failed check fails its test with a [`ReceiptError`].

use crate::fb_versions::{golden_checkpoint, ReceiptError};
use crate::GOLDEN_600;

/// First-run gate in seconds: the whole tutorial must fit this budget.
pub const GATE_SECS: u64 = 900;

/// Per-step budgets in seconds, pinned to the owner's `budget_secs` arms by the
/// facts below. Their sum must fit [`GATE_SECS`].
pub const STEP_BUDGET_SECS: &[(&str, u64)] = &[
    ("new-arena", 180),
    ("add-roster", 120),
    ("add-gear", 120),
    ("first-fight", 180),
];

/// One pinned owner line: the owner file, and text one trimmed source line must
/// contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fact {
    /// Stable fact id printed in the report.
    pub id: &'static str,
    /// Owner source file, relative to `crates/`.
    pub owner: &'static str,
    /// Text that one trimmed source line must contain.
    pub needle: &'static str,
}

/// Every pinned owner fact: tutorial ids and budgets, template limits and kinds,
/// and the UIX1 menu contract.
pub const FACTS: &[Fact] = &[
    Fact {
        id: "tutorial-id",
        owner: "editor/src/onboard.rs",
        needle: "pub const TUTORIAL_ID: &str = \"tutorial-1\";",
    },
    Fact {
        id: "fight-budget",
        owner: "editor/src/onboard.rs",
        needle: "pub const TIME_TO_FIRST_FIGHT_BUDGET_SECS: u64 = 900;",
    },
    Fact {
        id: "hint-chars",
        owner: "editor/src/onboard.rs",
        needle: "pub const MAX_HINT_CHARS: usize = 80;",
    },
    Fact {
        id: "step-new-arena",
        owner: "editor/src/onboard.rs",
        needle: "(\"new-arena\", \"Templates tab: pick Arena, press Create\"),",
    },
    Fact {
        id: "step-add-roster",
        owner: "editor/src/onboard.rs",
        needle: "\"add-roster\",",
    },
    Fact {
        id: "step-add-gear",
        owner: "editor/src/onboard.rs",
        needle: "\"add-gear\",",
    },
    Fact {
        id: "step-first-fight",
        owner: "editor/src/onboard.rs",
        needle: "\"first-fight\",",
    },
    Fact {
        id: "budget-arena",
        owner: "editor/src/onboard.rs",
        needle: "Self::Arena => 180,",
    },
    Fact {
        id: "budget-roster",
        owner: "editor/src/onboard.rs",
        needle: "Self::Roster => 120,",
    },
    Fact {
        id: "budget-gear",
        owner: "editor/src/onboard.rs",
        needle: "Self::Gear => 120,",
    },
    Fact {
        id: "budget-first-fight",
        owner: "editor/src/onboard.rs",
        needle: "Self::FirstFight => 180,",
    },
    Fact {
        id: "gate-fits",
        owner: "editor/src/onboard.rs",
        needle: "pub fn fits_budget() -> bool {",
    },
    Fact {
        id: "template-kinds",
        owner: "editor/src/templates.rs",
        needle: "pub const fn all() -> [Self; 4] {",
    },
    Fact {
        id: "template-draws",
        owner: "editor/src/templates.rs",
        needle: "pub const MAX_TEMPLATE_DRAWS: u64 = 64;",
    },
    Fact {
        id: "template-triangles",
        owner: "editor/src/templates.rs",
        needle: "pub const MAX_TEMPLATE_TRIANGLES: u64 = 1_000_000;",
    },
    Fact {
        id: "template-name-chars",
        owner: "editor/src/templates.rs",
        needle: "pub const MAX_NAME_CHARS: usize = 64;",
    },
    Fact {
        id: "menu-fight-seed",
        owner: "ui/src/menus.rs",
        needle: "pub const DEFAULT_FIGHT_SEED: u64 = 0x1C3A_9E77_51F2_0B41;",
    },
    Fact {
        id: "menu-dispatch",
        owner: "ui/src/menus.rs",
        needle: "pub fn dispatch(cmd: MenuCommand, hooks: &ModeHooks)",
    },
];

/// Owner sources the fact check reads, keyed by the owner path the facts use.
const OWNER_SOURCES: &[(&str, &str)] = &[
    (
        "editor/src/onboard.rs",
        include_str!("../../editor/src/onboard.rs"),
    ),
    (
        "editor/src/templates.rs",
        include_str!("../../editor/src/templates.rs"),
    ),
    ("ui/src/menus.rs", include_str!("../../ui/src/menus.rs")),
];

/// True when the fact's owner source has one trimmed line containing its needle.
fn fact_present(fact: &Fact) -> bool {
    OWNER_SOURCES
        .iter()
        .find(|(owner, _)| *owner == fact.owner)
        .is_some_and(|(_, text)| text.lines().any(|line| line.trim().contains(fact.needle)))
}

/// Ids of the facts whose owner source no longer carries them.
pub fn fact_misses() -> Vec<&'static str> {
    FACTS
        .iter()
        .filter(|fact| !fact_present(fact))
        .map(|fact| fact.id)
        .collect()
}

/// Sum of the step budgets in seconds.
pub fn budget_total() -> u64 {
    STEP_BUDGET_SECS.iter().map(|(_, secs)| *secs).sum()
}

/// Build the FB3 receipt text. Same owner sources and owner outputs give the
/// same text.
pub fn build_report() -> Result<String, ReceiptError> {
    let golden = golden_checkpoint()?;
    let misses = fact_misses();
    let tier = if cfg!(feature = "m1") {
        "contract+owner"
    } else {
        "contract"
    };
    let mut lines = vec![
        "fb3 ship receipt".to_string(),
        format!("tier {tier}"),
        format!(
            "golden {golden:016x} {}",
            if golden == GOLDEN_600 { "ok" } else { "CHANGED" }
        ),
        format!("facts {} misses {}", FACTS.len(), misses.len()),
    ];
    for fact in FACTS {
        let state = if misses.contains(&fact.id) {
            "MISSING"
        } else {
            "ok"
        };
        lines.push(format!("fact {} {} {state}", fact.id, fact.owner));
    }
    for (step, secs) in STEP_BUDGET_SECS {
        lines.push(format!("step-budget {step} {secs}"));
    }
    lines.push(format!("budget {} of {GATE_SECS} secs", budget_total()));
    #[cfg(feature = "m1")]
    lines.extend(headless::report_lines()?);
    #[cfg(not(feature = "m1"))]
    lines.push("headless boot not run (build with --features m1)".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

/// The `m1` tier: the editor's own template and onboarding APIs, driven headless.
/// Only events go in; nothing reads a hint or waits on a UI.
#[cfg(feature = "m1")]
mod headless {
    use engine_editor::commands::{Document, Journal};
    use engine_editor::onboard::{Checklist, OnboardError};
    use engine_editor::templates::{TemplateKind, TemplateService};

    use crate::fb_versions::{at, ReceiptError};

    /// Counts for one headless boot. `coaching` is every event that did not
    /// advance a step: a refusal or a no-op. Zero coaching means the boot needed
    /// no help.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Boot {
        /// Events fed to the checklist.
        pub events: u32,
        /// Events that completed a step.
        pub advanced: u32,
        /// Events that completed nothing.
        pub coaching: u32,
        /// Completed steps at the end.
        pub steps_done: usize,
        /// Every step done at the end.
        pub complete: bool,
        /// The booted document saves, reopens equal.
        pub import_ok: bool,
        /// The booted document is play-ready.
        pub play_ready: bool,
    }

    /// Boot an arena, the roster starter and a weapon, then start play. The
    /// three templates are the tutorial's first three steps; the fourth is play.
    pub fn headless_boot() -> Result<Boot, ReceiptError> {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut service = TemplateService::new();
        let mut checklist = Checklist::new();
        let mut events = 0u32;
        let mut advanced = 0u32;
        let mut coaching = 0u32;
        let roster = TemplateKind::all()
            .into_iter()
            .find(|kind| kind.is_roster())
            .ok_or(ReceiptError::Invariant("no roster template kind"))?;
        for (key, kind) in [
            (1u64, TemplateKind::Arena),
            (2, roster),
            (3, TemplateKind::Weapon),
        ] {
            let name = service.default_name(kind);
            service
                .apply(&mut doc, &mut journal, kind, &name, key)
                .map_err(at("apply starter template"))?;
            events += 1;
            if checklist.note_template(kind).is_some() {
                advanced += 1;
            } else {
                coaching += 1;
            }
        }
        let import_ok = TemplateService::verify_import(&doc).is_ok();
        let play_ready = TemplateService::play_ready(&doc);
        events += 1;
        match checklist.note_first_fight(play_ready) {
            Ok(true) => advanced += 1,
            _ => coaching += 1,
        }
        Ok(Boot {
            events,
            advanced,
            coaching,
            steps_done: checklist.done_count(),
            complete: checklist.is_complete(),
            import_ok,
            play_ready,
        })
    }

    /// Every template kind applied alone: save and reopen, then play-ready.
    /// Returns the kind names that booted, in kind order.
    pub fn boot_each_kind() -> Result<Vec<&'static str>, ReceiptError> {
        let mut booted = Vec::new();
        for kind in TemplateKind::all() {
            let mut doc = Document::new();
            let mut journal = Journal::new();
            let mut service = TemplateService::new();
            let name = service.default_name(kind);
            service
                .apply(&mut doc, &mut journal, kind, &name, 1)
                .map_err(at("apply template alone"))?;
            TemplateService::verify_import(&doc).map_err(at("verify template import"))?;
            if !TemplateService::play_ready(&doc) {
                return Err(ReceiptError::Invariant(
                    "a template is not play-ready after its apply",
                ));
            }
            booted.push(kind.name());
        }
        Ok(booted)
    }

    /// True when play before the scene is ready is refused with
    /// [`OnboardError::NotPlayReady`] and leaves the checklist unchanged.
    pub fn early_play_refused() -> bool {
        let mut checklist = Checklist::new();
        let before = checklist;
        let refused = matches!(
            checklist.note_first_fight(false),
            Err(OnboardError::NotPlayReady)
        );
        refused && checklist == before
    }

    /// Report lines for the headless tier.
    pub fn report_lines() -> Result<Vec<String>, ReceiptError> {
        let boot = headless_boot()?;
        let kinds = boot_each_kind()?;
        Ok(vec![
            format!(
                "boot-alone {} of {} kinds: {}",
                kinds.len(),
                TemplateKind::all().len(),
                kinds.join(", ")
            ),
            format!(
                "headless-boot events {} advanced {} coaching {}",
                boot.events, boot.advanced, boot.coaching
            ),
            format!(
                "headless-boot steps {} complete {} import-ok {} play-ready {}",
                boot.steps_done, boot.complete, boot.import_ok, boot.play_ready
            ),
            format!("early-play refused {}", early_play_refused()),
            format!(
                "owner budget fits {} estimated {} of {} secs",
                Checklist::fits_budget(),
                Checklist::estimated_total_secs(),
                engine_editor::onboard::TIME_TO_FIRST_FIGHT_BUDGET_SECS
            ),
        ])
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn template_boots_headless_with_zero_coaching() {
            let boot = headless_boot().expect("headless boot");
            assert_eq!(boot.events, 4);
            assert_eq!(boot.coaching, 0, "{boot:?}");
            assert_eq!(boot.advanced, boot.events);
            assert_eq!(boot.steps_done, 4);
            assert!(
                boot.complete && boot.import_ok && boot.play_ready,
                "{boot:?}"
            );
        }

        #[test]
        fn every_template_kind_boots_alone() {
            let kinds = boot_each_kind().expect("each kind boots");
            assert_eq!(kinds.len(), TemplateKind::all().len());
        }

        #[test]
        fn early_play_is_refused_and_changes_nothing() {
            assert!(early_play_refused());
            assert!(Checklist::fits_budget());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn evidence_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/b/ship")
    }

    #[test]
    fn pinned_facts_match_owner_source() {
        let misses = fact_misses();
        assert!(misses.is_empty(), "facts missing from owner source: {misses:?}");
    }

    #[test]
    fn step_budgets_fit_the_fifteen_minute_gate() {
        assert_eq!(budget_total(), 600);
        assert!(budget_total() <= GATE_SECS);
    }

    #[test]
    fn golden_hash_is_unchanged() {
        assert_eq!(golden_checkpoint(), Ok(GOLDEN_600));
    }

    #[test]
    fn report_is_written_twice_and_reproduces() {
        let first = build_report().expect("first report");
        let second = build_report().expect("second report");
        assert_eq!(first, second);
        let prefix = if cfg!(feature = "m1") {
            "report-owner"
        } else {
            "report"
        };
        let dir = evidence_dir();
        fs::create_dir_all(&dir).expect("evidence folder");
        fs::write(dir.join(format!("{prefix}-1.txt")), &first).expect("write run 1");
        fs::write(dir.join(format!("{prefix}-2.txt")), &second).expect("write run 2");
        let one = fs::read(dir.join(format!("{prefix}-1.txt"))).expect("read run 1");
        let two = fs::read(dir.join(format!("{prefix}-2.txt"))).expect("read run 2");
        assert_eq!(one, two);
    }
}
