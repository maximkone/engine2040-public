//! `physics` pack: what one blow does to a fighter's balance.
//!
//! One read-only verb, `physics.read`: for a stagger line and a blow strength
//! (both Q8 impulse units, 256 reads unity) it answers the verdict the blow
//! warrants (`Steady`, `Staggered`, `Stumbling` or `Fallen`), the ticks the
//! stance then needs to go legal again, and the five standard rows of the
//! shove-response matrix for that tuning. The numbers themselves live in
//! `crates/physics/src/balance.rs`; this pack only reads its owner API
//! ([`engine_physics::balance::response_matrix`], `recovery_ticks` and
//! [`engine_physics::balance::BalanceState`]), so one tuning has one answer
//! in sim, harness and door.
//!
//! Caller: [`pack`] is registered by `crate::packs::PackRegistry::shipped`,
//! so the MCP door lists and dispatches `physics.read` for an AI that tunes
//! an encounter instead of guessing a blow strength.
//!
//! The verb takes `CapabilityCard::Observe`: it is pure integer math, reads
//! no bout and writes nothing, and allocates only the five response rows.
//! The door's live bout runs without the balance rules, so the live facts
//! (per-slot balance, bone velocities, bone contacts, joint drift) answer
//! `not_wired`, never a zero that would read as a measurement. Errors:
//! `bad_args` with the legal range named. Determinism: the same request gives
//! the same bytes on every machine.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the
//! discovery schemas the door budgets.

use super::bad_args as bad;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use engine_physics::balance::{
    recovery_ticks, response_matrix, BalanceParams, BalanceState, GaitVerdict, DEFAULT_STAGGER_Q8,
    MAX_IMPULSE_Q8,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "physics";
/// Verb: what one blow does to a fighter's balance.
pub const TOOL_PHYSICS_READ: &str = "physics.read";

/// Word every fact answers that only the live bout could know. The door's
/// bout carries no balance rules (`balance` is off in the arena rules), so a
/// live reading is not merely zero: it is unwired, and this word says so.
pub const NOT_WIRED: &str = "not_wired";

/// Stagger line and blow strength; both default to the shipped tuning, so
/// the default request reads the blow that lands exactly on the line.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsReadArgs {
    pub stagger_line_q8: u16,
    pub blow_strength_q8: u16,
}

impl Default for PhysicsReadArgs {
    fn default() -> Self {
        Self {
            stagger_line_q8: DEFAULT_STAGGER_Q8,
            blow_strength_q8: DEFAULT_STAGGER_Q8,
        }
    }
}

/// One row of the shove-response matrix: a standard strength, what the blow
/// warrants there and the ticks that stance needs to recover.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsRowOut {
    pub strength_q8: u16,
    pub verdict: String,
    pub recovery_ticks: u32,
}

/// The live-bout facts this verb cannot answer yet: each one carries
/// [`NOT_WIRED`] instead of a zero.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsLiveBoutOut {
    pub per_slot_balance: String,
    pub bone_velocities: String,
    pub bone_contacts: String,
    pub joint_drift: String,
}

impl PhysicsLiveBoutOut {
    /// Every live fact unwired: one `not_wired` word per fact, never a zero.
    fn not_wired() -> Self {
        let word = || NOT_WIRED.to_owned();
        Self {
            per_slot_balance: word(),
            bone_velocities: word(),
            bone_contacts: word(),
            joint_drift: word(),
        }
    }
}

/// The blow's verdict, its recovery ticks, the two lines above the stagger
/// line and the five matrix rows for the same tuning.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsReadOutput {
    pub stagger_line_q8: u16,
    pub blow_strength_q8: u16,
    pub stumble_q8: u16,
    pub fall_q8: u16,
    pub verdict: String,
    pub recovery_ticks: u32,
    pub matrix: Vec<PhysicsRowOut>,
    pub live_bout: PhysicsLiveBoutOut,
}

/// Verdict word for one impact verdict: the four words an encounter tuner
/// switches on. Pure; the same verdict reads the same word twice.
#[must_use]
pub fn verdict_word(verdict: GaitVerdict) -> &'static str {
    match verdict {
        GaitVerdict::Steady => "Steady",
        GaitVerdict::Staggered => "Staggered",
        GaitVerdict::Stumbling => "Stumbling",
        GaitVerdict::Fallen => "Fallen",
    }
}

/// `physics.read` handler: validate the two Q8 numbers, then ask the balance
/// owner what this blow warrants under this line. Pure: one local
/// [`BalanceState`] carries the question, no bout is read and nothing is
/// written; the only allocation is the five-row matrix.
fn physics_read(
    _server: &McpServer,
    args: &PhysicsReadArgs,
) -> Result<PhysicsReadOutput, McpToolError> {
    let line = args.stagger_line_q8;
    if line == 0 || line > MAX_IMPULSE_Q8 {
        return Err(bad(format!(
            "stagger_line_q8 {line} must be 1..={MAX_IMPULSE_Q8} (Q8 impulse units, \
             256 reads unity)"
        )));
    }
    let blow = args.blow_strength_q8;
    if blow > MAX_IMPULSE_Q8 {
        return Err(bad(format!(
            "blow_strength_q8 {blow} must be 0..={MAX_IMPULSE_Q8} (Q8 impulse units, \
             256 reads unity)"
        )));
    }
    let params = BalanceParams::new(line).map_err(|error| {
        bad(format!(
            "{error}; re-run with stagger_line_q8 1..={MAX_IMPULSE_Q8}"
        ))
    })?;
    let mut state = BalanceState::new(params);
    let verdict = state.apply_impact(blow).map_err(|error| {
        bad(format!(
            "{error}; re-run with blow_strength_q8 0..={MAX_IMPULSE_Q8}"
        ))
    })?;
    Ok(PhysicsReadOutput {
        stagger_line_q8: line,
        blow_strength_q8: blow,
        stumble_q8: state.stumble_q8(),
        fall_q8: state.fall_q8(),
        verdict: verdict_word(verdict).to_owned(),
        recovery_ticks: recovery_ticks(verdict),
        matrix: response_matrix(params)
            .iter()
            .map(|row| PhysicsRowOut {
                strength_q8: row.strength_q8,
                verdict: verdict_word(row.verdict).to_owned(),
                recovery_ticks: row.recovery_ticks,
            })
            .collect(),
        live_bout: PhysicsLiveBoutOut::not_wired(),
    })
}

crate::e2_tool!(
    /// `physics.read`: what one blow does to a fighter's balance.
    PhysicsReadTool,
    name = TOOL_PHYSICS_READ,
    description = "Read what one blow does to a fighter's balance (stagger_line_q8 1..=4096, blow_strength_q8 0..=4096): the verdict word, its recovery ticks and the five standard shove-response rows for that tuning. Pure integer, reads no bout; live-bout facts answer not_wired. Read-only.",
    readonly = true,
    args = PhysicsReadArgs,
    output = PhysicsReadOutput,
    invoke = physics_read
);

/// The physics pack: one read-only verb, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "balance: what one blow does to a fighter's stance (verdict word, recovery ticks, \
         the five-strength shove-response matrix), pure and integer over the physics balance \
         core, with live-bout facts unwired",
        false,
    );
    pack.add(VerbSpec::of::<PhysicsReadTool>(
        TOOL_PHYSICS_READ,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use engine_physics::balance::{
        CATCH_TICKS, FALL_ABOVE_STAGGER_Q8, FALL_PRONE_TICKS, STAGGER_TICKS,
    };
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn physics_read_orders_blows_over_the_line_and_the_fall_line() {
        let server = server();
        let under = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: DEFAULT_STAGGER_Q8,
                blow_strength_q8: DEFAULT_STAGGER_Q8 - 1,
            },
        )
        .expect("reads");
        assert_eq!(under.verdict, "Steady");
        assert_eq!(under.recovery_ticks, 0, "a steady stance owes no recovery");
        let at = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(at.blow_strength_q8, at.stagger_line_q8);
        assert_eq!(at.verdict, "Staggered");
        assert_eq!(at.recovery_ticks, STAGGER_TICKS);
        let over = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: DEFAULT_STAGGER_Q8,
                blow_strength_q8: DEFAULT_STAGGER_Q8.saturating_add(FALL_ABOVE_STAGGER_Q8),
            },
        )
        .expect("reads");
        assert_eq!(over.verdict, "Fallen");
        assert_eq!(over.recovery_ticks, FALL_PRONE_TICKS + CATCH_TICKS);
        assert_eq!(
            over.fall_q8,
            DEFAULT_STAGGER_Q8.saturating_add(FALL_ABOVE_STAGGER_Q8)
        );
        // The same request twice gives the same bytes.
        let again = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(at, again);
    }

    #[test]
    fn physics_read_matrix_equals_the_balance_owner() {
        let server = server();
        let args = PhysicsReadArgs {
            stagger_line_q8: 300,
            blow_strength_q8: 256,
        };
        let out = PhysicsReadTool::invoke(&server, args.clone()).expect("reads");
        let want = response_matrix(BalanceParams::new(args.stagger_line_q8).expect("legal line"));
        assert_eq!(
            out.matrix.len(),
            want.len(),
            "one row per standard strength"
        );
        for (row, expected) in out.matrix.iter().zip(want.iter()) {
            assert_eq!(row.strength_q8, expected.strength_q8);
            assert_eq!(row.verdict, verdict_word(expected.verdict));
            assert_eq!(row.recovery_ticks, recovery_ticks(expected.verdict));
        }
        // The default tuning falls on its last row; a lower line falls sooner.
        let default_rows =
            PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(
            default_rows
                .matrix
                .iter()
                .map(|row| row.verdict.as_str())
                .collect::<Vec<_>>(),
            vec!["Steady", "Staggered", "Stumbling", "Stumbling", "Fallen"]
        );
        let lower = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: 128,
                ..args
            },
        )
        .expect("reads");
        assert_ne!(out.matrix, lower.matrix, "the line moves the rows");
    }

    #[test]
    fn physics_read_refuses_a_zero_line_and_names_the_legal_range() {
        let server = server();
        let zero = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: 0,
                blow_strength_q8: 256,
            },
        )
        .expect_err("a zero line refuses");
        assert_eq!(zero.code, "bad_args");
        assert!(zero.message.contains("stagger_line_q8"));
        assert!(
            zero.message.contains(&format!("1..={MAX_IMPULSE_Q8}")),
            "the refusal names the legal range: {}",
            zero.message
        );
        for args in [
            PhysicsReadArgs {
                stagger_line_q8: MAX_IMPULSE_Q8 + 1,
                blow_strength_q8: 256,
            },
            PhysicsReadArgs {
                stagger_line_q8: 256,
                blow_strength_q8: MAX_IMPULSE_Q8 + 1,
            },
        ] {
            let refused = PhysicsReadTool::invoke(&server, args).expect_err("over the ceiling");
            assert_eq!(refused.code, "bad_args");
            assert!(refused.message.contains(&MAX_IMPULSE_Q8.to_string()));
        }
    }

    #[test]
    fn physics_read_reports_live_bout_facts_as_not_wired() {
        let server = server();
        let out = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        let live = &out.live_bout;
        for (name, fact) in [
            ("per_slot_balance", &live.per_slot_balance),
            ("bone_velocities", &live.bone_velocities),
            ("bone_contacts", &live.bone_contacts),
            ("joint_drift", &live.joint_drift),
        ] {
            assert_eq!(
                fact, NOT_WIRED,
                "{name} answers not_wired, never a zero reading"
            );
        }
        assert_ne!(NOT_WIRED, "0");
    }

    #[test]
    fn physics_read_pack_registers_one_observe_verb() {
        let pack = pack().expect("pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(pack.verb_names(), vec![TOOL_PHYSICS_READ]);
        assert!(!pack.default_loaded(), "the pack loads on demand");
        assert!(
            !pack.summary().is_empty(),
            "the pack carries its one-line summary"
        );
        for verb in pack.verbs() {
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly());
        }
    }
}
