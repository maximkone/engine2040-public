//! adapter: choose the GPU adapter on purpose (GS6).
//!
//! Owner: render lane. `presenter.rs` takes `PowerPreference::HighPerformance`
//! at three call sites, which hands an Optimus laptop the Intel adapter and
//! can land on the CPU rasterizer unnoticed. This module is the pure half of
//! the fix: an env-driven [`AdapterChoice`] ranked over enumerated
//! [`AdapterSummary`] rows. Software adapters never win unless
//! `E2040_ALLOW_SOFTWARE=1`.
//! Units: list indices for picks, u32 PCI ids for vendors. Determinism: pure
//! over (list, choice, software flag); the env is read once on the cold
//! startup path, never per frame. Presentation only, never sim state.
//! Consumer: `presenter_pick` lifts wgpu adapters into summaries and picks.

use std::error::Error;
use std::fmt;

/// Env override naming the wanted adapter (`auto`, `discrete`,
/// `integrated`, or a name fragment). Unset or empty means auto.
pub const ADAPTER_ENV: &str = "E2040_ADAPTER";
/// Env switch letting a software adapter win. Only the exact value `1` counts.
pub const ALLOW_SOFTWARE_ENV: &str = "E2040_ALLOW_SOFTWARE";

/// Which adapter to bring up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AdapterChoice {
    /// Best hardware adapter: discrete first, then backend order.
    Auto,
    /// A discrete GPU; fails loudly when none is present.
    Discrete,
    /// An integrated GPU; fails loudly when none is present.
    Integrated,
    /// First adapter whose name contains this fragment (case-insensitive).
    Name(String),
}

/// Parse one `E2040_ADAPTER` value. Empty means [`AdapterChoice::Auto`];
/// `auto`, `discrete` and `integrated` are keywords (case-insensitive),
/// anything else is a [`AdapterChoice::Name`] fragment as written.
#[must_use]
pub fn parse_choice(raw: &str) -> AdapterChoice {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => AdapterChoice::Auto,
        "discrete" => AdapterChoice::Discrete,
        "integrated" => AdapterChoice::Integrated,
        _ => AdapterChoice::Name(raw.trim().to_string()),
    }
}

/// Read [`AdapterChoice`] from the live process env. Cold startup path only.
#[must_use]
pub fn choice_from_env() -> AdapterChoice {
    parse_choice(&std::env::var(ADAPTER_ENV).unwrap_or_default())
}

/// True only when `E2040_ALLOW_SOFTWARE=1`. Cold startup path only.
#[must_use]
pub fn allow_software() -> bool {
    matches!(
        std::env::var(ALLOW_SOFTWARE_ENV).ok().as_deref(),
        Some(value) if value.trim() == "1"
    )
}

/// Adapter kind in pick order: discrete beats integrated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterKind {
    /// Discrete GPU with its own memory.
    Discrete,
    /// Integrated GPU sharing CPU memory.
    Integrated,
    /// CPU/software rasterizer. Refused unless opted in.
    Cpu,
    /// Anything else (virtual, hosted, unknown). Hardware, never refused.
    Other,
}

impl AdapterKind {
    /// Short label quoted in [`AdapterSummary::describe`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Discrete => "discrete",
            Self::Integrated => "integrated",
            Self::Cpu => "cpu",
            Self::Other => "other",
        }
    }

    /// Rank for [`AdapterChoice::Auto`]: lower wins.
    fn rank(self) -> u8 {
        match self {
            Self::Discrete => 0,
            Self::Integrated => 1,
            Self::Other => 2,
            Self::Cpu => 3,
        }
    }
}

/// Backend in pick order: Dx12, Vulkan, Gl.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterBackend {
    /// DirectX 12 (Windows reference path).
    Dx12,
    /// Vulkan (portable reference path).
    Vulkan,
    /// OpenGL (fallback only).
    Gl,
    /// Anything else. Ranked last, never refused.
    Other,
}

impl AdapterBackend {
    /// Short label quoted in [`AdapterSummary::describe`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Dx12 => "dx12",
            Self::Vulkan => "vulkan",
            Self::Gl => "gl",
            Self::Other => "other",
        }
    }

    /// Rank for the pick: lower wins.
    fn rank(self) -> u8 {
        match self {
            Self::Dx12 => 0,
            Self::Vulkan => 1,
            Self::Gl => 2,
            Self::Other => 3,
        }
    }
}

/// One enumerated adapter: everything the pure pick needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AdapterSummary {
    /// Driver-reported name (e.g. `NVIDIA GeForce RTX 3050 Laptop GPU`).
    pub name: String,
    /// Backend this row was enumerated on.
    pub backend: AdapterBackend,
    /// Kind of device.
    pub kind: AdapterKind,
    /// Backend vendor id (low 16 bits are the PCI id).
    pub vendor: u32,
}

impl AdapterSummary {
    /// One-line startup log: `adapter: <name> <backend> <type> vendor=0x<id>`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "adapter: {} {} {} vendor=0x{:04x}",
            self.name,
            self.backend.name(),
            self.kind.name(),
            self.vendor
        )
    }
}

/// Adapter pick failures; every variant names the culprit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AdapterError {
    /// Nothing was enumerated.
    Empty,
    /// Only software adapters exist and `E2040_ALLOW_SOFTWARE=1` is unset.
    /// Carries the refused adapter's name.
    OnlySoftware(String),
    /// No enumerated adapter fits the choice. Carries the want.
    NoMatch(String),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "no GPU adapters enumerated"),
            Self::OnlySoftware(name) => write!(
                f,
                "only software adapter present ({name}); set E2040_ALLOW_SOFTWARE=1 to use it"
            ),
            Self::NoMatch(want) => write!(f, "no adapter matches {want:?}"),
        }
    }
}

impl Error for AdapterError {}

/// Pick the winning index of `list` for `choice`. Software rows never win
/// unless `E2040_ALLOW_SOFTWARE=1`. Cold startup path only.
pub fn choose(list: &[AdapterSummary], choice: &AdapterChoice) -> Result<usize, AdapterError> {
    choose_with(list, choice, allow_software())
}

/// [`choose`] with the software flag as an input, so tests drive it exactly.
pub fn choose_with(
    list: &[AdapterSummary],
    choice: &AdapterChoice,
    allow_cpu: bool,
) -> Result<usize, AdapterError> {
    if list.is_empty() {
        return Err(AdapterError::Empty);
    }
    let mut pool: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind != AdapterKind::Cpu)
        .map(|(index, _)| index)
        .collect();
    if pool.is_empty() {
        if !allow_cpu {
            return Err(AdapterError::OnlySoftware(list[0].name.clone()));
        }
        pool = (0..list.len()).collect();
    }
    match choice {
        AdapterChoice::Auto => Ok(best_of(list, &pool)),
        AdapterChoice::Discrete => pool
            .iter()
            .copied()
            .filter(|&index| list[index].kind == AdapterKind::Discrete)
            .min_by_key(|&index| list[index].backend.rank())
            .ok_or_else(|| AdapterError::NoMatch("discrete".to_string())),
        AdapterChoice::Integrated => pool
            .iter()
            .copied()
            .filter(|&index| list[index].kind == AdapterKind::Integrated)
            .min_by_key(|&index| list[index].backend.rank())
            .ok_or_else(|| AdapterError::NoMatch("integrated".to_string())),
        AdapterChoice::Name(want) => {
            let needle = want.to_ascii_lowercase();
            let mut best: Option<usize> = None;
            for &index in &pool {
                if !list[index].name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                let replace = match best {
                    None => true,
                    Some(held) => rank_of(&list[index]) < rank_of(&list[held]),
                };
                if replace {
                    best = Some(index);
                }
            }
            best.ok_or_else(|| AdapterError::NoMatch(want.clone()))
        }
    }
}

/// Pick the winning index of `list` for `choice` over the rows `compatible`
/// marks surface-compatible. A short mask refuses the tail, a long mask's
/// tail is ignored, so a caller that enumerates and tests support in one
/// pass can hand both slices over directly. Empty enumerate is
/// [`AdapterError::Empty`]; adapters present but none compatible is
/// [`AdapterError::NoMatch`] naming `surface-compatible`. Pure over
/// (list, mask, choice, software flag); cold startup path only.
/// Consumer: the GS6 surface sites in `present/gpu_present/presenter.rs`
/// (`init_windowed` plus `compatible_adapter` through `pick_surface_adapter`).
pub fn choose_compatible(
    list: &[AdapterSummary],
    compatible: &[bool],
    choice: &AdapterChoice,
    allow_cpu: bool,
) -> Result<usize, AdapterError> {
    if list.is_empty() {
        return Err(AdapterError::Empty);
    }
    let pool: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(index, _)| compatible.get(*index).copied().unwrap_or(false))
        .map(|(index, _)| index)
        .collect();
    if pool.is_empty() {
        return Err(AdapterError::NoMatch("surface-compatible".to_string()));
    }
    let rows: Vec<AdapterSummary> = pool.iter().map(|&index| list[index].clone()).collect();
    Ok(pool[choose_with(&rows, choice, allow_cpu)?])
}

/// Rank pair for one row: kind first, then backend. Lower wins.
fn rank_of(row: &AdapterSummary) -> (u8, u8) {
    (row.kind.rank(), row.backend.rank())
}

/// Best index of `pool` by [`rank_of`]; ties keep the earlier index.
fn best_of(list: &[AdapterSummary], pool: &[usize]) -> usize {
    let mut best = pool[0];
    for &index in &pool[1..] {
        if rank_of(&list[index]) < rank_of(&list[best]) {
            best = index;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, backend: AdapterBackend, kind: AdapterKind, vendor: u32) -> AdapterSummary {
        AdapterSummary {
            name: name.to_string(),
            backend,
            kind,
            vendor,
        }
    }

    /// The GS6 accept list: Optimus laptop plus the CPU rasterizer.
    fn canned() -> Vec<AdapterSummary> {
        vec![
            row(
                "Intel UHD Graphics",
                AdapterBackend::Dx12,
                AdapterKind::Integrated,
                0x8086,
            ),
            row(
                "NVIDIA GeForce RTX 3050 Laptop GPU",
                AdapterBackend::Dx12,
                AdapterKind::Discrete,
                0x10de,
            ),
            row(
                "Basic Render Driver",
                AdapterBackend::Dx12,
                AdapterKind::Cpu,
                0x1414,
            ),
        ]
    }

    #[test]
    fn auto_picks_the_discrete_gpu() {
        assert_eq!(choose_with(&canned(), &AdapterChoice::Auto, false), Ok(1));
    }

    #[test]
    fn integrated_picks_the_uhd() {
        assert_eq!(
            choose_with(&canned(), &AdapterChoice::Integrated, false),
            Ok(0)
        );
    }

    #[test]
    fn name_fragment_picks_the_uhd() {
        assert_eq!(
            choose_with(&canned(), &AdapterChoice::Name("uhd".to_string()), false),
            Ok(0)
        );
        assert_eq!(
            choose_with(
                &canned(),
                &AdapterChoice::Name("RTX 3050".to_string()),
                false
            ),
            Ok(1)
        );
    }

    #[test]
    fn cpu_only_list_is_only_software_naming_it() {
        let only = vec![row(
            "Basic Render Driver",
            AdapterBackend::Dx12,
            AdapterKind::Cpu,
            0x1414,
        )];
        assert_eq!(
            choose_with(&only, &AdapterChoice::Auto, false),
            Err(AdapterError::OnlySoftware(
                "Basic Render Driver".to_string()
            ))
        );
        // Opt-in uses it instead of failing.
        assert_eq!(choose_with(&only, &AdapterChoice::Auto, true), Ok(0));
    }

    #[test]
    fn backend_order_is_dx12_then_vulkan_then_gl() {
        let list = vec![
            row("Card A", AdapterBackend::Gl, AdapterKind::Discrete, 0x10de),
            row(
                "Card B",
                AdapterBackend::Vulkan,
                AdapterKind::Discrete,
                0x10de,
            ),
            row(
                "Card C",
                AdapterBackend::Dx12,
                AdapterKind::Discrete,
                0x10de,
            ),
        ];
        assert_eq!(choose_with(&list, &AdapterChoice::Auto, false), Ok(2));
        assert_eq!(choose_with(&list, &AdapterChoice::Discrete, false), Ok(2));
    }

    #[test]
    fn missing_kind_and_name_fail_loudly() {
        let list = canned();
        assert_eq!(
            choose_with(&list, &AdapterChoice::Name("radeon".to_string()), false),
            Err(AdapterError::NoMatch("radeon".to_string()))
        );
        let integrated_only = vec![row(
            "UHD",
            AdapterBackend::Dx12,
            AdapterKind::Integrated,
            0x8086,
        )];
        assert_eq!(
            choose_with(&integrated_only, &AdapterChoice::Discrete, false),
            Err(AdapterError::NoMatch("discrete".to_string()))
        );
        let empty: Vec<AdapterSummary> = vec![];
        assert_eq!(
            choose_with(&empty, &AdapterChoice::Auto, false),
            Err(AdapterError::Empty)
        );
    }

    #[test]
    fn describe_prints_the_startup_line() {
        let rtx = &canned()[1];
        assert_eq!(
            rtx.describe(),
            "adapter: NVIDIA GeForce RTX 3050 Laptop GPU dx12 discrete vendor=0x10de"
        );
    }

    #[test]
    fn env_parses_keywords_and_fragments() {
        assert_eq!(parse_choice(""), AdapterChoice::Auto);
        assert_eq!(parse_choice("auto"), AdapterChoice::Auto);
        assert_eq!(parse_choice(" Discrete "), AdapterChoice::Discrete);
        assert_eq!(parse_choice("INTEGRATED"), AdapterChoice::Integrated);
        assert_eq!(parse_choice("uhd"), AdapterChoice::Name("uhd".to_string()));
    }

    #[test]
    fn errors_name_the_culprit() {
        assert_eq!(
            AdapterError::Empty.to_string(),
            "no GPU adapters enumerated"
        );
        assert!(
            AdapterError::OnlySoftware("Basic Render Driver".to_string())
                .to_string()
                .contains("Basic Render Driver")
        );
        assert!(AdapterError::NoMatch("radeon".to_string())
            .to_string()
            .contains("radeon"));
    }

    #[test]
    fn compatible_prefers_discrete_when_all_compatible() {
        assert_eq!(
            choose_compatible(&canned(), &[true, true, true], &AdapterChoice::Auto, false),
            Ok(1)
        );
    }

    #[test]
    fn compatible_skips_an_incompatible_discrete_gpu() {
        assert_eq!(
            choose_compatible(&canned(), &[true, false, true], &AdapterChoice::Auto, false),
            Ok(0)
        );
    }

    #[test]
    fn compatible_none_is_no_match() {
        assert_eq!(
            choose_compatible(
                &canned(),
                &[false, false, false],
                &AdapterChoice::Auto,
                false
            ),
            Err(AdapterError::NoMatch("surface-compatible".to_string()))
        );
        let empty: Vec<AdapterSummary> = vec![];
        assert_eq!(
            choose_compatible(&empty, &[], &AdapterChoice::Auto, false),
            Err(AdapterError::Empty)
        );
    }

    #[test]
    fn compatible_cpu_only_is_refused_without_opt_in() {
        let only = vec![row(
            "Basic Render Driver",
            AdapterBackend::Dx12,
            AdapterKind::Cpu,
            0x1414,
        )];
        assert_eq!(
            choose_compatible(&only, &[true], &AdapterChoice::Auto, false),
            Err(AdapterError::OnlySoftware(
                "Basic Render Driver".to_string()
            ))
        );
        assert_eq!(
            choose_compatible(&only, &[true], &AdapterChoice::Auto, true),
            Ok(0)
        );
    }

    #[test]
    fn compatible_short_mask_refuses_the_tail() {
        assert_eq!(
            choose_compatible(&canned(), &[true], &AdapterChoice::Auto, false),
            Ok(0)
        );
    }
}
