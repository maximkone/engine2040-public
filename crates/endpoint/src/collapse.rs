//! API6 collapse guardrail: a pinned retain list plus an adaptive budget.
//!
//! API4 cut the advertised surface to the core tools and left every pack verb
//! to `api.search` -> `api.describe` -> `api.call`, while the door answered a
//! growing pack set by raising [`TOOL_LIST_BUDGET_CHARS`] each time. This
//! module guards the other end: when the tools a session would advertise cost
//! more than the ledger allows, the door advertises a deterministic subset
//! instead of a list its own gate refuses.
//!
//! The selection is protocol-aware and never model summarization: no tool
//! definition is rewritten, shortened or summarized, and no model is called.
//! The retain list stays advertised in its pinned order: the retained set
//! leads with the [`RETAIN_TOOL_NAMES`] spine, and the rest of the budget
//! goes to the remaining tools in the order the door already lists them
//! (name sorted). Priority follows the protocol, not the alphabet:
//!
//! 1. the [`RETAIN_TOOL_NAMES`] spine (`api.search`, `api.describe`, `api.call`,
//!    `api.load` and the core read, write, play and build tools), always;
//! 2. the packs the door loads by default (`scene` today), whole, because a
//!    default session must keep advertising what it always advertised;
//! 3. the packs this session loaded itself, newest load first, whole when the
//!    pack fits and verb by verb in listed order when it does not;
//! 4. anything else still listed, in listed order.
//!
//! A hidden tool keeps its exact definition and stays callable by exact name
//! (`api.call` or a plain `tools/call`), so hiding costs one discovery step and
//! never an answer.
//!
//! The budget is measured, not hand-edited: the session's default surface (core
//! plus its default packs) earns [`PACK_SHARE_PCT`] of itself on top, never
//! less than [`MIN_PACK_CHARS`], and is capped at [`MAX_ADVERTISED_CHARS`], the
//! ledger ceiling. Costs use the ledger's own measure
//! ([`crate::mcp::tool_cost_chars`]), so an advertised list passes the ledger
//! gate by construction; [`tool_chars`] is the serialized measure the API4 and
//! API6 token reports print.
//!
//! Determinism: the same tool list and load order give the same retained set,
//! the same chars and the same budget. Integer math only.

use rmcp::model::Tool;

use crate::mcp::{tool_cost_chars, TOOL_LIST_BUDGET_CHARS, TOOL_LIST_MIN_HEADROOM_CHARS};
use crate::packs::PackRegistry;

/// Tool names the door always advertises: the discovery spine (`api.search`,
/// `api.describe`, `api.call`, `api.load`) plus the core read, write, play and
/// build tools, in the [`crate::mcp::CORE_TOOL_NAMES`] contract order. Pinned
/// by name so a pack change can never hide a protocol path.
pub const RETAIN_TOOL_NAMES: [&str; 13] = [
    "scene.read",
    "tx.preview",
    "tx.apply",
    "tx.undo",
    "play.run",
    "capture",
    "api.search",
    "api.describe",
    "api.call",
    "api.load",
    "test.run",
    "sim.build_encounter",
    "game.build",
];

/// Pack allowance as a share of the session's default surface, in percent.
pub const PACK_SHARE_PCT: usize = 50;

/// Floor for the pack allowance in chars, so a small default surface still
/// advertises a pack on top of its spine.
pub const MIN_PACK_CHARS: usize = 8_192;

/// Highest advertised cost the door may show, in chars of the ledger's own
/// measure: [`TOOL_LIST_BUDGET_CHARS`] minus its minimum headroom. A list at
/// or under this passes the ledger gate with its floor intact.
pub const MAX_ADVERTISED_CHARS: usize = TOOL_LIST_BUDGET_CHARS - TOOL_LIST_MIN_HEADROOM_CHARS;

/// Serialized cost of one tool definition in chars (name, description, input
/// and output schema). Same measure the API4 and API6 token reports print, so
/// both reports compare.
#[must_use]
pub fn tool_chars(tool: &Tool) -> usize {
    rmcp::serde_json::to_string(tool).map_or(0, |text| text.len())
}

/// Adaptive budget in chars for a surface: `base_chars` plus
/// [`PACK_SHARE_PCT`] of it (never less than [`MIN_PACK_CHARS`]), capped at
/// `ceiling_chars`. Grows with the surface it measures, so a wider default
/// contract gets a wider allowance without a hand-edited constant. Integer
/// math only.
#[must_use]
pub fn adaptive_budget_chars(base_chars: usize, ceiling_chars: usize) -> usize {
    let share = base_chars.saturating_mul(PACK_SHARE_PCT) / 100;
    base_chars
        .saturating_add(share.max(MIN_PACK_CHARS))
        .min(ceiling_chars)
}

/// One collapse decision: names kept advertised, names hidden, and the costs
/// that decided it. Units: chars of the ledger's measure, except
/// `total_serialized_chars`. Deterministic over its input list and load order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollapsePlan {
    /// Advertised names, in the input list's order.
    pub retained: Vec<String>,
    /// Names withheld from the advertisement. They stay callable by name.
    pub hidden: Vec<String>,
    /// Cost of `retained`, in chars.
    pub retained_chars: usize,
    /// Cost of the whole input list, in chars.
    pub total_chars: usize,
    /// Cost of the whole input list serialized, in chars: the API4/API6 token
    /// measure of the kind of list that was trimmed.
    pub total_serialized_chars: usize,
    /// The budget the plan followed, in chars.
    pub budget_chars: usize,
}

impl CollapsePlan {
    /// True when `name` stays advertised.
    #[must_use]
    pub fn keeps(&self, name: &str) -> bool {
        self.retained.iter().any(|kept| kept == name)
    }

    /// Share of the hidden cost in whole percent of the input list.
    #[must_use]
    pub fn hidden_share_pct(&self) -> usize {
        if self.total_chars == 0 {
            return 0;
        }
        let hidden = self.total_chars.saturating_sub(self.retained_chars);
        hidden.saturating_mul(100) / self.total_chars
    }
}

/// Index of `name` in `tools`, if listed.
fn position(tools: &[Tool], name: &str) -> Option<usize> {
    tools.iter().position(|tool| tool.name.as_ref() == name)
}

/// Indices of one pack's listed verbs, in the list's order (name sorted).
fn pack_group(tools: &[Tool], verbs: &[&'static str]) -> Vec<usize> {
    let mut group: Vec<usize> = verbs
        .iter()
        .filter_map(|name| position(tools, name))
        .collect();
    group.sort_unstable();
    group.dedup();
    group
}

/// Groups of listed tools, in priority order: the spine, then the packs the
/// door loads by default, then this session's own loads newest first. Each
/// group holds indices in the list's order.
fn priority_groups(
    tools: &[Tool],
    load_order: &[&'static str],
    registry: &PackRegistry,
) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![pack_group(tools, &RETAIN_TOOL_NAMES)];
    let mut grouped: Vec<&'static str> = Vec::new();
    for pack in registry.packs() {
        if pack.default_loaded() {
            groups.push(pack_group(tools, &pack.verb_names()));
            grouped.push(pack.name());
        }
    }
    for pack in load_order.iter().rev() {
        if grouped.contains(pack) {
            continue;
        }
        if let Some(def) = registry.pack(pack) {
            groups.push(pack_group(tools, &def.verb_names()));
            grouped.push(def.name());
        }
    }
    groups
}

/// Cost of the session's default surface: the spine plus every pack the door
/// loads by default, the first `default_groups` groups. The budget never falls
/// below it, so a default session keeps advertising what it always advertised.
fn default_surface_chars(groups: &[Vec<usize>], costs: &[usize], default_groups: usize) -> usize {
    groups
        .iter()
        .take(default_groups)
        .map(|group| group.iter().map(|&index| costs[index]).sum::<usize>())
        .fold(0usize, usize::saturating_add)
}

/// Which tools the budget keeps: the spine whole (it is the protocol path),
/// then every group whole when it fits and verb by verb in listed order when
/// it does not, then whatever still fits in the list's order.
fn place(groups: &[Vec<usize>], costs: &[usize], budget_chars: usize) -> Vec<bool> {
    let mut placed = vec![false; costs.len()];
    let mut remaining = budget_chars;
    // The spine is the protocol path: placed whole, even over budget, because
    // a door without `api.call` could never reach a hidden tool.
    for &index in &groups[0] {
        placed[index] = true;
        remaining = remaining.saturating_sub(costs[index]);
    }
    for group in groups.iter().skip(1) {
        let whole: usize = group.iter().map(|&index| costs[index]).sum();
        if whole <= remaining {
            for &index in group {
                placed[index] = true;
            }
            remaining -= whole;
            continue;
        }
        // The pack does not fit whole: keep what fits, in listed order, so a
        // wide verb never hides a narrow sibling of the same pack.
        for &index in group {
            if costs[index] <= remaining {
                placed[index] = true;
                remaining -= costs[index];
            }
        }
    }
    for (index, &cost) in costs.iter().enumerate() {
        if !placed[index] && cost <= remaining {
            placed[index] = true;
            remaining -= cost;
        }
    }
    placed
}

/// Plan one collapsed advertisement over `tools`, honouring the session's
/// explicit `load_order` (oldest load first, newest last). Pure; never
/// rewrites a definition.
#[must_use]
pub fn plan(tools: &[Tool], load_order: &[&'static str]) -> CollapsePlan {
    let registry = PackRegistry::shipped();
    let costs: Vec<usize> = tools.iter().map(tool_cost_chars).collect();
    let total_chars: usize = costs.iter().sum();
    let total_serialized_chars: usize = tools.iter().map(tool_chars).sum();

    let groups = priority_groups(tools, load_order, registry);
    let default_groups = 1 + registry
        .packs()
        .iter()
        .filter(|p| p.default_loaded())
        .count();
    let budget_chars = adaptive_budget_chars(
        default_surface_chars(&groups, &costs, default_groups),
        MAX_ADVERTISED_CHARS,
    );
    let placed = place(&groups, &costs, budget_chars);

    // The retained set leads with the pinned spine in its pinned order, then
    // the rest of the list in the list's order, so the protocol path always
    // reads first and the promise in the header holds.
    let mut retained = Vec::with_capacity(tools.len());
    for pinned in RETAIN_TOOL_NAMES {
        if let Some(index) = position(tools, pinned) {
            if placed[index] {
                retained.push((*pinned).to_owned());
            }
        }
    }
    let mut hidden = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let name = tool.name.to_string();
        if !placed[index] {
            hidden.push(name);
        } else if !RETAIN_TOOL_NAMES.contains(&tool.name.as_ref()) {
            retained.push(name);
        }
    }
    CollapsePlan {
        retained,
        hidden,
        retained_chars: total_chars.saturating_sub(
            costs
                .iter()
                .enumerate()
                .filter(|(index, _)| !placed[*index])
                .map(|(_, cost)| *cost)
                .sum::<usize>(),
        ),
        total_chars,
        total_serialized_chars,
        budget_chars,
    }
}

/// The list the door may advertise: [`plan`] over `listed`, keeping the
/// input's order. A list that fits the budget comes back untouched.
#[must_use]
pub fn guard(listed: Vec<Tool>, load_order: &[&'static str]) -> Vec<Tool> {
    let planned = plan(&listed, load_order);
    if planned.hidden.is_empty() {
        return listed;
    }
    listed
        .into_iter()
        .filter(|tool| planned.keeps(tool.name.as_ref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::CORE_TOOL_NAMES;
    use engine_editor::capability::Capability;

    fn server() -> crate::mcp::McpServer {
        crate::mcp::McpServer::new(7, Capability::unrestricted(7))
    }

    fn load(server: &crate::mcp::McpServer, pack: &str) {
        server
            .lock()
            .expect("test lock holds")
            .load_pack(pack, false)
            .expect("pack loads");
    }

    fn cost(tools: &[Tool]) -> usize {
        tools.iter().map(tool_cost_chars).sum()
    }

    /// The pinned list covers the core contract and names only reachable tools.
    #[test]
    fn retain_list_covers_the_core_and_is_reachable() {
        let reachable = crate::mcp::tool_names();
        for pinned in RETAIN_TOOL_NAMES {
            assert!(
                reachable.contains(&pinned),
                "retain name `{pinned}` must stay a reachable tool"
            );
        }
        for core in CORE_TOOL_NAMES {
            assert!(
                RETAIN_TOOL_NAMES.contains(&core),
                "core tool `{core}` must stay in the retain list: the guardrail \
                 may hide a pack verb, never a core tool"
            );
        }
        assert_eq!(
            RETAIN_TOOL_NAMES.len(),
            CORE_TOOL_NAMES.len(),
            "the retain list is the core spine and nothing else"
        );
    }

    #[test]
    fn adaptive_budget_follows_the_surface_and_never_exceeds_the_ceiling() {
        assert_eq!(adaptive_budget_chars(0, 0), 0);
        assert_eq!(
            adaptive_budget_chars(1_000, 100_000),
            1_000 + MIN_PACK_CHARS,
            "a small surface still gets the pack floor"
        );
        assert_eq!(
            adaptive_budget_chars(40_000, 100_000),
            40_000 + 20_000,
            "a wide surface gets half of itself on top"
        );
        assert_eq!(
            adaptive_budget_chars(40_000, 45_000),
            45_000,
            "the budget never exceeds its ceiling"
        );
        assert_eq!(
            adaptive_budget_chars(56_737, MAX_ADVERTISED_CHARS),
            MAX_ADVERTISED_CHARS,
            "the ledger ceiling binds before the share does on today's surface"
        );
    }

    /// A default session advertises exactly what it always did: the guardrail
    /// only trims a surface that loaded packs pushed over the ceiling.
    #[test]
    fn default_session_list_is_untouched() {
        let probe = server();
        let listed = probe
            .lock()
            .expect("test lock holds")
            .loaded_packs()
            .clone();
        let tools = probe.tools();
        let planned = plan(&tools, &[]);
        assert!(planned.hidden.is_empty(), "default surface fits the budget");
        assert_eq!(planned.retained.len(), tools.len());
        assert_eq!(planned.retained_chars, planned.total_chars);
        assert!(
            planned.total_chars <= planned.budget_chars,
            "default surface {} chars over budget {}",
            planned.total_chars,
            planned.budget_chars
        );
        assert_eq!(
            planned.budget_chars, MAX_ADVERTISED_CHARS,
            "a default session already spends the ledger ceiling"
        );
        assert!(
            listed.len() <= 1,
            "only the scene pack loads by default, saw {listed:?}"
        );
    }

    #[test]
    fn plan_is_deterministic_and_keeps_the_spine() {
        let probe = server();
        let all = probe.all_tools();
        let order: Vec<&'static str> = PackRegistry::shipped()
            .packs()
            .iter()
            .map(|pack| pack.name())
            .collect();
        let first = plan(&all, &order);
        let second = plan(&all, &order);
        assert_eq!(first, second, "same list and load order, same plan");
        for pinned in RETAIN_TOOL_NAMES {
            assert!(first.keeps(pinned), "retain tool `{pinned}` is advertised");
        }
        // The spine leads the retained set in its pinned order.
        let spine: Vec<&str> = first
            .retained
            .iter()
            .take(RETAIN_TOOL_NAMES.len())
            .map(|name| name.as_str())
            .collect();
        assert_eq!(
            spine,
            RETAIN_TOOL_NAMES.to_vec(),
            "the retained set leads with the pinned spine in its pinned order"
        );
        assert!(
            !first.hidden.is_empty(),
            "the flat {} tool surface is trimmed, not advertised whole",
            all.len()
        );
        assert!(
            first.retained_chars <= first.budget_chars,
            "kept {} chars over a {} char budget",
            first.retained_chars,
            first.budget_chars
        );
        assert!(
            first.budget_chars <= MAX_ADVERTISED_CHARS,
            "budget {} stays under the ledger ceiling",
            first.budget_chars
        );
        assert_eq!(
            first.retained.len() + first.hidden.len(),
            all.len(),
            "every input tool is kept or hidden"
        );
        assert!(
            first.hidden_share_pct() >= 50,
            "guardrail hides {}% of the flat surface, bar is 50%",
            first.hidden_share_pct()
        );
    }

    /// A hidden tool is still a reachable contract name: hiding withholds the
    /// advertisement only.
    #[test]
    fn hidden_tools_stay_reachable_names() {
        let probe = server();
        let planned = plan(&probe.all_tools(), &[]);
        let reachable = crate::mcp::tool_names();
        for name in &planned.hidden {
            assert!(
                reachable.iter().any(|known| known == name),
                "hidden tool `{name}` stays callable by exact name"
            );
        }
    }

    /// An on-demand load grows the advertised list: the pack's narrow verbs are
    /// advertised even when the pack does not fit whole, and unloading hides
    /// them again.
    #[test]
    fn a_loaded_pack_grows_the_list_and_unload_restores_it() {
        let probe = server();
        // The full default surface plus gym is over the hard tool budget, so
        // the documented flow unloads the default packs first; the pack then
        // fits the ledger's ceiling whole.
        crate::packs::hide_default_packs(&probe);
        let before = probe.tools();
        load(&probe, "gym");
        let after = probe.tools();
        assert!(
            after.len() > before.len(),
            "loading gym advertises more tools: {} vs {}",
            after.len(),
            before.len()
        );
        let names: Vec<&str> = after.iter().map(|tool| tool.name.as_ref()).collect();
        assert!(
            names.contains(&"sim.reset"),
            "the loaded pack's verbs reach the list"
        );
        assert!(names.contains(&"sim.observe"));
        assert!(
            cost(&after) <= MAX_ADVERTISED_CHARS,
            "advertised {} chars stay under the ceiling {MAX_ADVERTISED_CHARS}",
            cost(&after)
        );
        assert!(
            cost(&before) <= cost(&after),
            "an older list never shrinks when a pack loads"
        );
        probe
            .lock()
            .expect("test lock holds")
            .load_pack("gym", true)
            .expect("gym unloads");
        let restored: Vec<String> = probe
            .tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let original: Vec<String> = before.iter().map(|tool| tool.name.to_string()).collect();
        assert_eq!(restored, original, "unload restores the exact list");
    }

    /// A list under the budget is returned untouched; the flat surface is not.
    #[test]
    fn guard_leaves_a_list_under_the_ledger_alone() {
        let probe = server();
        let listed = probe.all_tools();
        let small: Vec<Tool> = listed
            .iter()
            .filter(|tool| RETAIN_TOOL_NAMES.contains(&tool.name.as_ref()))
            .cloned()
            .collect();
        let guarded = guard(small.clone(), &[]);
        assert_eq!(
            guarded.iter().map(tool_chars).collect::<Vec<_>>(),
            small.iter().map(tool_chars).collect::<Vec<_>>(),
            "under the budget the list stands untouched"
        );
        let wide = guard(listed.clone(), &[]);
        assert!(
            wide.len() < listed.len(),
            "the flat surface is trimmed to {} tools",
            wide.len()
        );
        assert!(cost(&wide) <= MAX_ADVERTISED_CHARS);
    }
}
