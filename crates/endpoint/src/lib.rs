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
#[cfg(feature = "mcp")]
pub mod discover;
/// Audited dispatch seam: one audit record per door call, pinned by holders.
#[cfg(feature = "mcp")]
pub mod door_dispatch;
#[cfg(feature = "mcp")]
pub mod explain;
/// Shared guard block for tool writes (std-only, no `mcp` feature needed).
pub mod guards;
pub mod injection;
pub mod insight;
#[cfg(feature = "mcp")]
pub mod intents;
#[cfg(feature = "mcp")]
pub mod log_tools;
#[cfg(feature = "mcp")]
pub mod mcp;
/// Door v2 verb packs: core tools plus packs loaded on demand, one guarded
/// registration API for every verb.
#[cfg(feature = "mcp")]
pub mod packs;
#[cfg(feature = "mcp")]
pub mod plan_goals;
#[cfg(feature = "mcp")]
pub mod play;
pub mod receipts;
/// BRP-shaped loopback shim over the verb table (in-process, no socket).
#[cfg(feature = "mcp")]
pub mod remote_shim;
pub mod sandbox;
#[cfg(feature = "mcp")]
pub mod scene_query;
#[cfg(feature = "mcp")]
pub mod scene_tools;
pub mod secret_launch;
#[cfg(feature = "mcp")]
pub mod startup;
#[cfg(feature = "mcp")]
pub mod tool_capabilities;
#[cfg(feature = "mcp")]
pub mod tool_macro;
pub mod tools;
pub mod transport;
#[cfg(feature = "mcp")]
pub mod watch_ledger;
/// Witnessed-plan gate plus evidence receipts (std-only, no `mcp` needed).
pub mod witness;
