//! `asset.impostor`: when a legion turns to cards, and what that saves.
//!
//! The answer comes from the frozen tier table in `engine_anim`, not from a
//! second copy: [`CARD_THRESHOLD_M`] is `engine_anim::lod::FAR_DIST_M`, and the
//! per-tier counts come from `engine_anim::lod::assign_tiers` (which also keeps
//! the nearest `engine_anim::lod::FULL_RATE_NEAREST` fighters at full rate) and
//! `engine_anim::crowd_scale::count_tiers`. One distance is assumed for the whole
//! legion: a block of fighters standing together, at `screen_frac = 0.0`
//! (small on screen). Deterministic: same input, same counts, twice.
//!
//! Card cost: `card_bytes` is `cards x bytes_per_card` from the tier preset
//! below, a documented estimate constant while the crowd specimen cook is not
//! in the assets registry. `saves_draws_x100` is fixed point hundredths: a
//! specimen fighter draws two mesh primitives (torso + head) and one card draws
//! one, so each card saves one draw (`MESH_DRAWS_PER_CARD - 1`).

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};

/// Tool name.
pub const TOOL_ASSET_IMPOSTOR: &str = "asset.impostor";
/// Smallest legion the verb prices.
pub const MIN_LEGION: u16 = 1;
/// Largest legion the verb prices (the crowd tape's own cap is higher).
pub const MAX_LEGION: u16 = 512;
/// Nearest distance the verb accepts, in metres (0 = the legion on top of the camera).
pub const MIN_DISTANCE_M: u16 = 0;
/// Farthest distance the verb accepts, in metres.
pub const MAX_DISTANCE_M: u16 = 200;
/// Card quality names, lowest first.
pub const TIER_NAMES: [&str; 3] = ["low", "mid", "high"];
/// Quality used when the caller names none.
pub const DEFAULT_TIER: &str = "mid";
/// Mesh primitives one specimen fighter draws (torso + head from the crowd kit).
pub const MESH_DRAWS_PER_CARD: u32 = 2;

/// Distance where the far tier starts, from the crate's own table.
pub const CARD_THRESHOLD_M: f32 = engine_anim::lod::FAR_DIST_M;

/// One card quality: its name, the card LOD it draws, and its bytes per card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardPreset {
    /// `low`, `mid` or `high`.
    pub name: &'static str,
    /// Card mesh LOD index (0 is finest); the specimen chain has four.
    pub lod: u8,
    /// Bytes one card costs at this quality (documented estimate constant).
    pub bytes_per_card: u64,
}

/// The three card presets, in [`TIER_NAMES`] order.
pub const CARD_PRESETS: [CardPreset; 3] = [
    CardPreset {
        name: "low",
        lod: 3,
        bytes_per_card: 1_024,
    },
    CardPreset {
        name: "mid",
        lod: 2,
        bytes_per_card: 2_048,
    },
    CardPreset {
        name: "high",
        lod: 1,
        bytes_per_card: 4_096,
    },
];

/// A card quality by name.
fn preset(name: &str) -> Option<CardPreset> {
    let name = name.to_lowercase();
    CARD_PRESETS.iter().copied().find(|p| p.name == name)
}

/// Args for `asset.impostor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ImpostorArgs {
    /// Fighters in the legion, 1..=512.
    pub fighters: u16,
    /// Distance to the legion in metres, 0..=200 (0 is on top of the camera).
    pub distance_m: u16,
    /// Card quality: `low`, `mid` or `high` (default `mid`).
    #[serde(default)]
    pub tier: Option<String>,
}

/// One tier's agent count.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TierRowsOut {
    /// Agents at full rate (every tick).
    pub full: u32,
    /// Agents at half rate.
    pub half: u32,
    /// Agents at quarter rate.
    pub quarter: u32,
    /// Agents at the far tier: these become cards.
    pub far: u32,
}

/// What the legion costs at the door.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ImpostorOutput {
    /// Fighters asked about.
    pub fighters: u16,
    /// Distance used, in metres.
    pub distance_m: u16,
    /// Card quality used.
    pub tier: String,
    /// Card mesh LOD the tier draws (0 is finest).
    pub card_lod: u8,
    /// Fighters that turn to cards (the far tier).
    pub cards: u32,
    /// Fighters that keep a mesh (full, half and quarter).
    pub meshes: u32,
    /// Distance where cards start, in metres.
    pub threshold_m: f32,
    /// One count per tier.
    pub tiers: TierRowsOut,
    /// Bytes the cards cost at this quality.
    pub card_bytes: u64,
    /// Where `card_bytes` came from (tier estimate constant).
    pub card_bytes_source: String,
    /// Draw calls saved, fixed point hundredths (100 = one draw per card).
    pub saves_draws_x100: u32,
    /// One warning per surprise, in a fixed order.
    pub warnings: Vec<String>,
}

fn bad(message: String) -> McpToolError {
    McpToolError::new("bad_input", message)
}

/// Price one legion at one distance.
fn impostor(args: &ImpostorArgs) -> Result<ImpostorOutput, McpToolError> {
    if !(MIN_LEGION..=MAX_LEGION).contains(&args.fighters) {
        return Err(bad(format!(
            "fighters {} is outside {MIN_LEGION}..={MAX_LEGION}; send a legion in that band",
            args.fighters
        )));
    }
    if !(MIN_DISTANCE_M..=MAX_DISTANCE_M).contains(&args.distance_m) {
        return Err(bad(format!(
            "distance_m {} is outside {MIN_DISTANCE_M}..={MAX_DISTANCE_M}; send metres in that band",
            args.distance_m
        )));
    }
    let wanted = args.tier.clone().unwrap_or_else(|| DEFAULT_TIER.to_owned());
    let Some(preset) = preset(&wanted) else {
        return Err(bad(format!(
            "tier `{wanted}` is not one of {}; send one of them",
            TIER_NAMES.join(", ")
        )));
    };
    let fighters = u32::try_from(args.fighters).unwrap_or(u32::MAX);
    let distance = f32::from(args.distance_m);
    // The tier table's own fighter cap is smaller than a legion, so ask it for the
    // base tier at this distance and apply its nearest-full rule here.
    let base = engine_anim::lod::tier_for(distance, 0.0).map_err(|e| {
        bad(format!(
            "the tier table refused {distance} m: {e}; send a distance in {MIN_DISTANCE_M}..={MAX_DISTANCE_M} m"
        ))
    })?;
    let (mut full, mut half, mut quarter, mut far) = match base {
        engine_anim::lod::Tier::Full => (fighters, 0, 0, 0),
        engine_anim::lod::Tier::Half => (0, fighters, 0, 0),
        engine_anim::lod::Tier::Quarter => (0, 0, fighters, 0),
        engine_anim::lod::Tier::Far => (0, 0, 0, fighters),
    };
    let near = u32::try_from(engine_anim::lod::FULL_RATE_NEAREST)
        .unwrap_or(u32::MAX)
        .min(fighters);
    match base {
        engine_anim::lod::Tier::Full => {}
        engine_anim::lod::Tier::Half => {
            full += near;
            half -= near;
        }
        engine_anim::lod::Tier::Quarter => {
            full += near;
            quarter -= near;
        }
        engine_anim::lod::Tier::Far => {
            full += near;
            far -= near;
        }
    }
    let cards = far;
    let meshes = fighters.saturating_sub(cards);
    let mut warnings = Vec::new();
    if cards == 0 && distance > CARD_THRESHOLD_M {
        warnings.push(format!(
            "cards 0 at {distance} m though cards start at {CARD_THRESHOLD_M} m; the nearest {} fighters keep full rate, so ask about a bigger legion",
            engine_anim::lod::FULL_RATE_NEAREST
        ));
    }
    let card_bytes = u64::from(cards).saturating_mul(preset.bytes_per_card);
    let saved = cards.saturating_mul(MESH_DRAWS_PER_CARD.saturating_sub(1));
    Ok(ImpostorOutput {
        fighters: args.fighters,
        distance_m: args.distance_m,
        tier: preset.name.to_owned(),
        card_lod: preset.lod,
        cards,
        meshes,
        threshold_m: CARD_THRESHOLD_M,
        tiers: TierRowsOut {
            full,
            half,
            quarter,
            far,
        },
        card_bytes,
        card_bytes_source: format!(
            "estimate constant {} B/card at tier {} (LOD{}); the crowd specimen cook is not in the assets registry yet",
            preset.bytes_per_card, preset.name, preset.lod
        ),
        saves_draws_x100: saved.saturating_mul(100),
        warnings,
    })
}

fn impostor_verb(_server: &McpServer, args: &ImpostorArgs) -> Result<ImpostorOutput, McpToolError> {
    impostor(args)
}

crate::e2_tool!(
    /// `asset.impostor`: when a legion turns to cards and what it costs.
    ImpostorTool,
    name = TOOL_ASSET_IMPOSTOR,
    description = "Price a legion of fighters at one distance: cards and meshes from the engine_anim tier table (card threshold FAR_DIST_M, nearest FULL_RATE_NEAREST kept at full rate), per-tier counts, card bytes at a low/mid/high card quality, draw calls saved and one warning when cards are 0 beyond the threshold. Read-only, deterministic, fighters 1..=512 and distance 0..=200 m.",
    readonly = true,
    args = ImpostorArgs,
    output = ImpostorOutput,
    invoke = impostor_verb
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServer;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(11, Capability::unrestricted(11))
    }

    fn run(fighters: u16, distance_m: u16, tier: Option<&str>) -> ImpostorOutput {
        ImpostorTool::invoke(
            &server(),
            ImpostorArgs {
                fighters,
                distance_m,
                tier: tier.map(str::to_owned),
            },
        )
        .expect("prices")
    }

    #[test]
    fn a_legion_past_the_threshold_turns_to_cards_and_the_counts_add_up() {
        let out = run(120, 80, None);
        assert_eq!(out.fighters, 120);
        assert_eq!(out.tier, DEFAULT_TIER);
        assert_eq!(out.threshold_m, CARD_THRESHOLD_M);
        assert!(f32::from(out.distance_m) > out.threshold_m);
        assert!(out.cards > 0, "80 m is past the card threshold");
        assert_eq!(
            out.meshes + out.cards,
            u32::from(out.fighters),
            "every fighter is a card or a mesh"
        );
        assert_eq!(
            out.meshes,
            u32::try_from(engine_anim::lod::FULL_RATE_NEAREST).unwrap_or(u32::MAX),
            "the nearest fighters keep full rate"
        );
        assert_eq!(
            out.cards + out.tiers.full + out.tiers.half + out.tiers.quarter,
            u32::from(out.fighters)
        );
        assert_eq!(out.saves_draws_x100, out.cards * 100);
        assert_eq!(out.card_bytes, u64::from(out.cards) * 2_048);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out, run(120, 80, None), "same input, same answer twice");
    }

    #[test]
    fn a_legion_at_zero_distance_keeps_every_mesh() {
        let out = run(120, 0, None);
        assert_eq!(out.cards, 0);
        assert_eq!(out.meshes, 120);
        assert_eq!(out.tiers.full, 120);
        assert_eq!(out.card_bytes, 0);
        assert_eq!(out.saves_draws_x100, 0);
        assert!(
            out.warnings.is_empty(),
            "below the threshold no warning: {:?}",
            out.warnings
        );
    }

    #[test]
    fn a_small_legion_past_the_threshold_warns_that_no_card_was_taken() {
        let out = run(6, 200, None);
        assert_eq!(out.cards, 0, "all six are inside the nearest-full ring");
        assert_eq!(out.meshes, 6);
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("cards 0 at 200 m"));
    }

    #[test]
    fn a_tier_picks_the_card_lod_and_its_bytes() {
        let low = run(100, 90, Some("low"));
        let high = run(100, 90, Some("high"));
        assert_eq!(low.card_lod, 3);
        assert_eq!(high.card_lod, 1);
        assert_eq!(low.card_bytes, u64::from(low.cards) * 1_024);
        assert_eq!(high.card_bytes, u64::from(high.cards) * 4_096);
        assert!(low.card_bytes < high.card_bytes);
    }

    #[test]
    fn out_of_band_numbers_and_an_unknown_tier_are_refused_with_the_band() {
        let over = ImpostorTool::invoke(
            &server(),
            ImpostorArgs {
                fighters: 600,
                distance_m: 80,
                tier: None,
            },
        )
        .expect_err("600 fighters");
        assert_eq!(over.code, "bad_input");
        assert!(over.message.contains("1..=512"), "{}", over.message);
        let far = ImpostorTool::invoke(
            &server(),
            ImpostorArgs {
                fighters: 10,
                distance_m: 201,
                tier: None,
            },
        )
        .expect_err("distance over 200");
        assert!(far.message.contains("0..=200"), "{}", far.message);
        let tier = ImpostorTool::invoke(
            &server(),
            ImpostorArgs {
                fighters: 10,
                distance_m: 10,
                tier: Some("ultra".to_owned()),
            },
        )
        .expect_err("unknown tier");
        assert!(tier.message.contains("low, mid, high"), "{}", tier.message);
    }
}
