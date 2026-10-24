//! `rules` pack: simulation steps and game rules an AI can test.
//!
//! Always: the classical goal planner and the live play session. With the off-by-default
//! `rules` feature: the scripted-rules verbs `rules.set`, `rules.test` and `rules.explain`
//! (module `verbs`), backed by one thread-confined worker per session (module `worker`).
//! A script installed with `rules.set` drives the fighter slots it names in every later
//! `play.run` of the session (the arena's controller hook, `engine_arena::controller`);
//! the default build carries no scripting engine.

use super::{PackDef, PackError, VerbSpec};
use crate::plan_goals::{PlanGoalsTool, MCP_TOOL_NAME as TOOL_SIM_PLAN_GOALS};
use crate::play::play_session::{PlayStepTool, TOOL_PLAY_STEP};
use crate::tool_capabilities::CapabilityCard;

#[cfg(feature = "rules")]
mod verbs;
#[cfg(feature = "rules")]
mod worker;

#[cfg(feature = "rules")]
pub(crate) use verbs::RulesSlot;
#[cfg(feature = "rules")]
pub use verbs::{
    RulesExplainArgs, RulesExplainOutput, RulesExplainTool, RulesGetArgs, RulesGetTool,
    RulesScriptOutput, RulesSetArgs, RulesSetOutput, RulesSetTool, RulesTestArgs, RulesTestOutput,
    RulesTestTool, RulesUndoArgs, RulesUndoTool, TOOL_RULES_EXPLAIN, TOOL_RULES_GET,
    TOOL_RULES_SET, TOOL_RULES_TEST, TOOL_RULES_UNDO,
};

/// Pack name.
pub const PACK: &str = "rules";

/// The rules pack: planner plus live play steps, loaded on demand; scripted-rule verbs when
/// the `rules` feature is on.
pub fn pack() -> Result<PackDef, PackError> {
    #[cfg(feature = "rules")]
    let blurb = "game rules and simulation steps: plan closed goals, step a live duel, set a \
                 rule script that drives fighter slots, test it and explain what it did";
    #[cfg(not(feature = "rules"))]
    let blurb = "game rules and simulation steps: plan closed goals, step a live duel; scripted \
                 rule verbs land here";
    let mut pack = PackDef::new(PACK, blurb, false);
    pack.add(VerbSpec::of::<PlanGoalsTool>(
        TOOL_SIM_PLAN_GOALS,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PlayStepTool>(
        TOOL_PLAY_STEP,
        PACK,
        CapabilityCard::Observe,
    )?);
    #[cfg(feature = "rules")]
    {
        pack.add(VerbSpec::of::<RulesSetTool>(
            TOOL_RULES_SET,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<RulesTestTool>(
            TOOL_RULES_TEST,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<RulesExplainTool>(
            TOOL_RULES_EXPLAIN,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<RulesGetTool>(
            TOOL_RULES_GET,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<RulesUndoTool>(
            TOOL_RULES_UNDO,
            PACK,
            CapabilityCard::Observe,
        )?);
    }
    Ok(pack)
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 6,
    name: PACK,
    default_loaded: false,
    tools: &[
        "sim.plan_goals",
        "play.step",
        "rules.set",
        "rules.test",
        "rules.explain",
        "rules.get",
        "rules.undo",
    ],
    build: pack,
};
