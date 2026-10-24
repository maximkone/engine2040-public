//! endpoint: agent driving seam. Core is std-only; the `mcp` feature adds
//! the rmcp stdio server over the command service (see `bin/e2040-mcp.rs`).
//! transport -> tools -> sandbox -> receipts.

/// Sealed assert runs over recorded tapes (harness only, never in tick).
pub mod assert_run;
#[cfg(feature = "mcp")]
pub mod assistant;
/// Tamper-evident log over every gateway call (std-only, integer math).
pub mod audit;
pub mod brief;
pub mod build_character;
pub mod build_encounter;
pub mod build_game;
#[cfg(feature = "mcp")]
pub mod build_level;
pub mod canon;
/// API6 guardrail over the advertised tool list: a pinned retain list plus an
/// adaptive char budget, deterministic and never a model call.
#[cfg(feature = "mcp")]
pub mod collapse;
/// EG26: the dev console's read-only door over the session's live play slot.
#[cfg(feature = "mcp")]
pub mod console_bridge;
#[cfg(feature = "mcp")]
pub mod discover;
/// Audited dispatch seam: one audit record per door call, pinned by holders.
#[cfg(feature = "mcp")]
pub mod door_dispatch;
/// DOOR7: six fixed prompts over the real door verbs, with a deterministic tape.
#[cfg(feature = "mcp")]
pub mod e2e_ladder;
#[cfg(feature = "mcp")]
pub mod explain;
/// DOOR1: play the level a session built and read a win or a loss. The objective runs on
/// `engine-rules`, so this verb rides the off-by-default `rules` feature like the scripted
/// rule verbs do.
#[cfg(all(feature = "mcp", feature = "rules"))]
pub mod game_play;
/// Shared guard block for tool writes (std-only, no `mcp` feature needed).
pub mod guards;
pub mod injection;
pub mod insight;
#[cfg(feature = "mcp")]
pub mod intents;
#[cfg(feature = "mcp")]
pub mod log_tools;
#[cfg(feature = "mcp")]
pub mod look_host;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(all(test, feature = "mcp"))]
mod mcp_transcript_tests;
/// Door v2 verb packs: core tools plus packs loaded on demand, one guarded
/// registration API for every verb.
#[cfg(feature = "mcp")]
pub mod packs;
/// Pick manifest: a capture pixel to an exact object or floor point (EV21).
#[cfg(feature = "mcp")]
pub mod pick;
#[cfg(feature = "mcp")]
pub mod plan_goals;
#[cfg(feature = "mcp")]
pub mod play;
pub mod receipts;
/// BRP-shaped loopback shim over the verb table (in-process, no socket).
#[cfg(feature = "mcp")]
pub mod remote_shim;
/// D16: the four MCP resources the door serves (crate map, door reference,
/// error catalog, game.toml schema).
#[cfg(feature = "mcp")]
pub mod resources;
pub mod sandbox;
/// D16: the five scenario prompts the door publishes.
#[cfg(feature = "mcp")]
pub mod scenarios;
#[cfg(feature = "mcp")]
pub mod scene_next;
#[cfg(feature = "mcp")]
pub mod scene_pick;
#[cfg(feature = "mcp")]
pub mod scene_query;
#[cfg(feature = "mcp")]
pub mod scene_tools;
pub mod secret_launch;
/// Published skills: `skills/*/SKILL.md` served as MCP prompts.
#[cfg(feature = "mcp")]
pub mod skills;
#[cfg(feature = "mcp")]
mod spatial_glue;
#[cfg(feature = "mcp")]
pub mod startup;
#[cfg(feature = "mcp")]
pub mod tool_capabilities;
#[cfg(feature = "mcp")]
pub mod tool_macro;
pub mod tools;
pub mod transport;
/// D19: the per-instance verifier checks and the SWE-bench style buckets.
#[cfg(feature = "mcp")]
pub mod verify_tasks;
#[cfg(feature = "mcp")]
pub mod watch_ledger;
/// Witnessed-plan gate plus evidence receipts (std-only, no `mcp` needed).
pub mod witness;
#[cfg(feature = "mcp")]
/// BRP `world.query` over the staged document: filters, strict and a hit cap.
pub mod world_query;
