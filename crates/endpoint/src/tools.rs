//! tools: frozen registry mirroring the MCP router, plus the batch cap.
//! Slim-8 for `search` by default; the door advertises `mcp::default_tool_names`
//! (core plus default packs), full set via search.
//!
//! The registry is version-pinned for replays: tapes pin
//! [`REGISTRY_VERSION`] beside the tool names, and replay verifies the pin
//! before running a single call, so a schema drift fails closed with a
//! receipt instead of replaying silently. Tool schemas stay frozen: the pin
//! lives beside [`TOOLS`], never inside [`ToolDef`].

use engine_core::Fnv64;

/// One tool definition. Schemas generate from this — never hand-written twice.
pub struct ToolDef {
    pub name: &'static str,
    pub readonly: bool,
}

/// The frozen registry: the twenty-six replay-pinned tools in contract order.
/// It is a subset of the reachable door (`mcp::tool_names`: thirteen core
/// tools plus every pack verb), pinned by `registry_matches_router` under
/// the `mcp` feature and by the router-count pin in
/// `tests/it/tool_count_pin.rs`; retired names resolve to `None`, so a
/// renamed tool fails closed until it is classified here. `readonly` follows the router annotations: the eight
/// writers (`tx.apply`, `tx.undo`, `scene.place`, `level.build`, `play.run`,
/// `play.step`, `api.call`, `api.load`) stage or move state, everything else
/// only reads.
pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "scene.read",
        readonly: true,
    },
    ToolDef {
        name: "scene.brief",
        readonly: true,
    },
    ToolDef {
        name: "scene.spatial",
        readonly: true,
    },
    ToolDef {
        name: "tx.preview",
        readonly: true,
    },
    ToolDef {
        name: "tx.apply",
        readonly: false,
    },
    ToolDef {
        name: "tx.undo",
        readonly: false,
    },
    ToolDef {
        name: "scene.place",
        readonly: false,
    },
    ToolDef {
        name: "level.build",
        readonly: false,
    },
    ToolDef {
        name: "scene.query",
        readonly: true,
    },
    ToolDef {
        name: "sim.plan_goals",
        readonly: true,
    },
    ToolDef {
        name: "play.run",
        readonly: false,
    },
    ToolDef {
        name: "play.step",
        readonly: false,
    },
    ToolDef {
        name: "capture",
        readonly: true,
    },
    ToolDef {
        name: "log.digest",
        readonly: true,
    },
    ToolDef {
        name: "log.query",
        readonly: true,
    },
    ToolDef {
        name: "api.search",
        readonly: true,
    },
    ToolDef {
        name: "api.describe",
        readonly: true,
    },
    ToolDef {
        name: "api.call",
        readonly: false,
    },
    ToolDef {
        name: "api.load",
        readonly: false,
    },
    ToolDef {
        name: "code.callers",
        readonly: true,
    },
    ToolDef {
        name: "asset.refs",
        readonly: true,
    },
    ToolDef {
        name: "rpc.discover",
        readonly: true,
    },
    ToolDef {
        name: "scene.watch",
        readonly: true,
    },
    ToolDef {
        name: "test.run",
        readonly: true,
    },
    ToolDef {
        name: "sim.build_encounter",
        readonly: true,
    },
    ToolDef {
        name: "game.build",
        readonly: true,
    },
];

/// Find one registry tool by name; `None` when the name is unknown.
/// Units are name chars in, one definition out. No errors: misses return
/// `None`. Budget is one scan of the registry.
pub fn lookup(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Frozen registry version tapes pin beside tool names. Bump only with a
/// frozen schema change (new tool, rename, readonly flip); the digest below
/// then changes with it, so old tapes refuse instead of replaying drifted.
pub const REGISTRY_VERSION: u32 = 1;

/// Bound for one `lookup`: 0.1 ms in nanoseconds. The registry is a small
/// frozen table, so one scan is a few dozen compares, far inside this budget.
pub const LOOKUP_P95_NANOS: u128 = 100_000;

/// Version-drift refusal: the pinned tape header versus the live registry.
/// Pure data; [`check_pinned_registry`] mints it, [`RegistryDrift::receipt`]
/// renders the replay receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryDrift {
    /// Registry version the tape pinned.
    pub pinned_version: u32,
    /// Live [`REGISTRY_VERSION`].
    pub current_version: u32,
    /// Digest the tape pinned.
    pub pinned_digest: u64,
    /// Live [`registry_digest`].
    pub current_digest: u64,
}

impl core::fmt::Display for RegistryDrift {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "registry_drift: tape pins v{} digest {:016x}, registry is v{} digest {:016x}",
            self.pinned_version, self.pinned_digest, self.current_version, self.current_digest
        )
    }
}

impl RegistryDrift {
    /// Replay receipt: names both pins plus one recovery, so a runner can
    /// re-record against the live registry without a second round trip.
    #[must_use]
    pub fn receipt(self) -> String {
        format!(
            "{self}; recovery: re-record the tape against v{} (digest {:016x}) \
             or re-run with a v{} tape",
            self.current_version, self.current_digest, self.current_version
        )
    }
}

/// Replay refusal: version drift, or a tape naming an unknown tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// Tape header drifts from the live registry.
    Drift(RegistryDrift),
    /// Tape names a tool the registry does not serve; fail closed.
    Unknown {
        /// First unknown name in the tape, in order.
        name: String,
    },
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Drift(drift) => write!(f, "{drift}"),
            Self::Unknown { name } => write!(
                f,
                "unknown_tool: tape names `{name}`; the registry serves exactly {} tools",
                TOOLS.len()
            ),
        }
    }
}

impl ReplayError {
    /// Replay receipt: drift carries both pins, unknown names the legal set.
    #[must_use]
    pub fn receipt(&self) -> String {
        match self {
            Self::Drift(drift) => drift.receipt(),
            Self::Unknown { name } => format!(
                "{self}; recovery: re-run with one of [{}] (tape names `{name}`)",
                TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

/// Length prefix for the canonical digests below, in `u32` units.
/// The registry holds a few dozen short names, so every length fits;
/// a larger value saturates instead of truncating.
pub(crate) fn digest_len(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// Canonical digest over the frozen registry: version, count, then contract
/// order names with readonly bits. Length-prefixes keep `("ab", "c")` from
/// colliding with `("a", "bc")`. Determinism: integer math only, same digest
/// on every machine for the same registry.
///
/// Lengths prefix in `u32` through [`digest_len`]: the registry is a small
/// frozen table of short names, so every length fits in `u32`.
#[must_use]
pub fn registry_digest() -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(REGISTRY_VERSION);
    hasher.write_u32(digest_len(TOOLS.len()));
    for tool in TOOLS {
        hasher.write_u32(digest_len(tool.name.len()));
        hasher.write_str(tool.name);
        hasher.write_bool(tool.readonly);
    }
    hasher.finish()
}

/// Gate one pinned tape header against the live registry. `Ok(version)`
/// when both pin fields match; `Err(drift)` with the receipt fields
/// otherwise. Pure: reads the table only, writes nothing.
pub fn check_pinned_registry(
    pinned_version: u32,
    pinned_digest: u64,
) -> Result<u32, RegistryDrift> {
    let current_digest = registry_digest();
    if pinned_version == REGISTRY_VERSION && pinned_digest == current_digest {
        Ok(REGISTRY_VERSION)
    } else {
        Err(RegistryDrift {
            pinned_version,
            current_version: REGISTRY_VERSION,
            pinned_digest,
            current_digest,
        })
    }
}

/// Replay one pinned call list: verify the tape header, fail closed on the
/// first unknown name, then seal a deterministic digest over the version
/// plus the call names in order. Same tape replays to the same digest on
/// every machine; drift refuses with a receipt and runs nothing. Pure:
/// reads the table only, writes nothing.
pub fn replay_pinned_calls(
    pinned_version: u32,
    pinned_digest: u64,
    calls: &[&str],
) -> Result<u64, ReplayError> {
    check_pinned_registry(pinned_version, pinned_digest).map_err(ReplayError::Drift)?;
    for name in calls {
        if lookup(name).is_none() {
            return Err(ReplayError::Unknown {
                name: (*name).to_owned(),
            });
        }
    }
    let mut hasher = Fnv64::new();
    hasher.write_u32(pinned_version);
    hasher.write_u32(digest_len(calls.len()));
    for name in calls {
        hasher.write_u32(digest_len(name.len()));
        hasher.write_str(name);
    }
    Ok(hasher.finish())
}

/// Batch cap: command batches refuse more than 50 calls per request.
/// Atomic path (journal→apply→re-read→rollback) lands separately.
pub const BATCH_MAX: usize = 50;

/// Validate a batch size. `Ok(n)` when admissible, `Err(BATCH_MAX)` when over.
pub fn check_batch(n: usize) -> Result<usize, usize> {
    if n <= BATCH_MAX {
        Ok(n)
    } else {
        Err(BATCH_MAX)
    }
}
/// Slim-8 default surface: progressive disclosure, never a 300-tool dump.
/// Full set only via `search`. All eight are read-only, so a weak model can
/// discover the door without ever touching a writer.
pub const SLIM8: &[&str] = &[
    "rpc.discover",
    "api.search",
    "scene.read",
    "scene.brief",
    "scene.spatial",
    "scene.query",
    "sim.plan_goals",
    "log.digest",
];

/// Default listing: slim-8 only.
pub fn slim() -> Vec<&'static ToolDef> {
    SLIM8.iter().filter_map(|n| lookup(n)).collect()
}

/// Ranked search over the frozen registry: case-insensitive substring match,
/// readonly-first, name-ascending tiebreak, capped at 8. Empty query → slim.
pub fn search(query: &str) -> Vec<&'static ToolDef> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return slim();
    }
    let mut hits: Vec<&ToolDef> = TOOLS
        .iter()
        .filter(|t| t.name.to_lowercase().contains(&q))
        .collect();
    hits.sort_by_key(|t| (!t.readonly, t.name));
    hits.truncate(8);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_surface_is_slim_8() {
        assert_eq!(slim().len(), 8);
        assert_eq!(TOOLS.len(), 26);
        for name in SLIM8 {
            let def = lookup(name).expect("slim-8 names resolve in the registry");
            assert!(def.readonly, "slim-8 stays read-only, got writer {name}");
        }
    }

    #[test]
    fn search_scene_returns_all_six_scene_tools_readonly_first() {
        let names: Vec<&str> = search("scene").iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "scene.brief",
                "scene.query",
                "scene.read",
                "scene.spatial",
                "scene.watch",
                "scene.place",
            ]
        );
    }

    #[test]
    fn search_caps_at_8_and_empty_means_slim() {
        assert!(search("e").len() <= 8);
        let names: Vec<&str> = search("").iter().map(|t| t.name).collect();
        let slim_names: Vec<&str> = slim().iter().map(|t| t.name).collect();
        assert_eq!(names, slim_names);
        assert!(search("no-such-verb-xyz").is_empty());
    }

    #[test]
    fn batch_cap_is_50() {
        assert_eq!(check_batch(50), Ok(50));
        assert_eq!(check_batch(51), Err(BATCH_MAX));
    }

    #[test]
    fn mutating_set_is_exactly_the_eight_writers() {
        let mut writers: Vec<&str> = TOOLS
            .iter()
            .filter(|t| !t.readonly)
            .map(|t| t.name)
            .collect();
        writers.sort_unstable();
        assert_eq!(
            writers,
            vec![
                "api.call",
                "api.load",
                "level.build",
                "play.run",
                "play.step",
                "scene.place",
                "tx.apply",
                "tx.undo",
            ]
        );
    }

    #[test]
    fn retired_names_resolve_to_none() {
        for name in [
            "health.summary",
            "tools.search",
            "tools.list",
            "state.read",
            "state.patch",
            "sim.brief",
            "sim.patch",
            "sim.run",
            "asset.health",
            "asset.acquire",
            "proof.run",
            "batch.execute",
        ] {
            assert!(
                lookup(name).is_none(),
                "retired `{name}` must fail closed, never resolve"
            );
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn registry_matches_router_exactly() {
        // Pack owners add verbs through `packs::VerbSpec` without touching
        // this frozen list, so the contract is: no phantom names (every
        // registry name is reachable) and every core tool is classified.
        let router: Vec<&str> = crate::mcp::tool_names();
        for tool in TOOLS {
            assert!(
                router.contains(&tool.name),
                "registry names a tool the door does not serve: {}",
                tool.name
            );
        }
        for core in crate::mcp::CORE_TOOL_NAMES {
            assert!(
                lookup(core).is_some(),
                "core tool `{core}` is in the registry"
            );
        }
    }

    #[test]
    fn pinned_v1_replay_is_identical_twice() {
        let digest = registry_digest();
        let calls = ["scene.read", "tx.preview", "tx.apply", "log.digest"];
        let first = replay_pinned_calls(REGISTRY_VERSION, digest, &calls).expect("v1 tape replays");
        let second =
            replay_pinned_calls(REGISTRY_VERSION, digest, &calls).expect("v1 tape replays twice");
        assert_eq!(first, second, "same tape replays identically");
        assert_eq!(
            check_pinned_registry(REGISTRY_VERSION, digest),
            Ok(REGISTRY_VERSION)
        );
        // Call order matters: a reordered tape seals a different digest.
        let reordered = replay_pinned_calls(
            REGISTRY_VERSION,
            digest,
            &["tx.preview", "scene.read", "tx.apply", "log.digest"],
        )
        .expect("reordered tape still pins v1");
        assert_ne!(first, reordered, "order is part of the replay digest");
    }

    #[test]
    fn drifted_pin_refuses_with_receipt() {
        let live = registry_digest();
        let drifted_version = REGISTRY_VERSION.wrapping_add(1);
        let by_version =
            check_pinned_registry(drifted_version, live).expect_err("v2 version drifts");
        assert_eq!(by_version.pinned_version, drifted_version);
        assert_eq!(by_version.current_version, REGISTRY_VERSION);
        let receipt = by_version.receipt();
        assert!(receipt.contains("registry_drift"), "receipt names the code");
        assert!(receipt.contains(&REGISTRY_VERSION.to_string()));
        assert!(receipt.contains("re-record"), "receipt carries a recovery");
        // Wrong digest with the right version refuses too.
        let bad_digest = live.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let by_digest = check_pinned_registry(REGISTRY_VERSION, bad_digest)
            .expect_err("drifted digest refuses");
        assert_eq!(by_digest.pinned_digest, bad_digest);
        // Replay refuses before running a single call, with the same receipt.
        let calls = ["scene.read", "tx.preview"];
        let refused =
            replay_pinned_calls(drifted_version, live, &calls).expect_err("drifted replay refuses");
        match &refused {
            ReplayError::Drift(drift) => assert_eq!(*drift, by_version),
            ReplayError::Unknown { name } => {
                panic!("drift must win over lookup, got unknown `{name}`")
            }
        }
        assert_eq!(refused.receipt(), by_version.receipt());
        // A renamed tool fails closed even under a good pin.
        let unknown = replay_pinned_calls(REGISTRY_VERSION, live, &["shell.exec"])
            .expect_err("unknown tool refuses");
        match unknown {
            ReplayError::Unknown { name } => assert_eq!(name, "shell.exec"),
            ReplayError::Drift(drift) => {
                panic!("good pin must not drift: {drift}")
            }
        }
    }

    #[test]
    fn lookup_p95_stays_under_point_one_ms() {
        let mut samples = Vec::with_capacity(1_000);
        for index in 0..1_000 {
            let name = TOOLS[index % TOOLS.len()].name;
            let start = std::time::Instant::now();
            let found = lookup(name).expect("registry names resolve");
            std::hint::black_box(found.name);
            samples.push(start.elapsed().as_nanos());
        }
        samples.sort_unstable();
        let p95 = samples[950];
        assert!(
            p95 <= LOOKUP_P95_NANOS,
            "lookup p95 {p95}ns exceeds {LOOKUP_P95_NANOS}ns; one scan of the registry must stay under 0.1 ms"
        );
    }
}
