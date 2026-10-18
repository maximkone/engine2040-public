//! report: which adapter drew a frame, and whether that was a fallback.
//!
//! Owner: render lane. The pure half of render-proof honesty: a receipt that
//! names an adapter also says who asked for it ([`SelectionSource`]), what was
//! asked ([`AdapterChoice`]), what won, whether the win was a fallback, why,
//! and what to do about it. [`select`] ranks the enumerated
//! [`AdapterSummary`] rows exactly like `choose_with` and `choose_compatible`
//! and wraps the winner in a [`BackendSelection`].
//! A fallback is a software adapter winning (only with `E2040_ALLOW_SOFTWARE=1`)
//! or the best adapter being unable to present to the window surface.
//! Two policies: [`SelectionPolicy::Prefer`] returns the report and names the
//! fallback; [`SelectionPolicy::Strict`] (`E2040_REQUIRE_GPU=1`, the name the
//! GPU tests already use) turns a fallback into a typed [`SelectionError`]
//! carrying the remedy, so a proof job cannot pass on the wrong adapter.
//! Text form: [`BackendSelection::line`], one stable `backend:` line.
//! Units: none. Determinism: pure over (list, mask, request); the env is read
//! once on the cold startup path. Presentation only, never sim state.
//! Consumers: `presenter_pick` (offscreen bring-up) and
//! `Presenter::init_windowed` (surface bring-up), read back through
//! `Presenter::backend_selection` and the editor's snapshot envelope.

use std::error::Error;
use std::fmt;

use super::{
    allow_software, choice_from_env, choose_compatible, choose_with, AdapterChoice, AdapterError,
    AdapterKind, AdapterSummary, ADAPTER_ENV,
};

const REASON_BEST: &str = "best hardware adapter (discrete first, then dx12, vulkan, gl)";
const REASON_REQUESTED: &str = "the requested adapter was found";
const REASON_SOFTWARE: &str = "only software adapters exist and E2040_ALLOW_SOFTWARE=1 allows one";
const REASON_SURFACE: &str = "the preferred adapter cannot present to this surface";
const REASON_CALLER: &str = "the caller opened the adapter itself";
const REMEDY_NONE: &str = "none";
const REMEDY_SOFTWARE: &str =
    "run on a machine with a GPU driver, or unset E2040_ALLOW_SOFTWARE to refuse software";
const REMEDY_SURFACE: &str =
    "set E2040_ADAPTER to an adapter that can present, or update the display driver";
const REMEDY_EMPTY: &str = "install a GPU driver; no adapter is enumerated";
const REMEDY_ONLY_SOFTWARE: &str =
    "install a GPU driver, or set E2040_ALLOW_SOFTWARE=1 to accept the software adapter";
const REMEDY_NO_MATCH: &str = "unset E2040_ADAPTER or name an adapter this machine has";

/// Where the request came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionSource {
    /// `E2040_ADAPTER` was set to a non-empty value.
    Env,
    /// Nothing asked: the engine default (auto).
    Default,
    /// The caller opened the adapter itself (browser shell, tests).
    Caller,
}

impl SelectionSource {
    /// Label quoted in [`BackendSelection::line`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Default => "default",
            Self::Caller => "caller",
        }
    }

    /// `Env` for a non-empty `E2040_ADAPTER` value, `Default` otherwise.
    #[must_use]
    pub fn from_raw(raw: Option<&str>) -> Self {
        match raw {
            Some(value) if !value.trim().is_empty() => Self::Env,
            _ => Self::Default,
        }
    }

    /// [`Self::from_raw`] over the live process env. Cold startup path only.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_raw(std::env::var(ADAPTER_ENV).ok().as_deref())
    }
}

/// What a fallback does to the run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionPolicy {
    /// Return the report; a fallback is allowed and named.
    Prefer,
    /// A fallback is a typed [`SelectionError`]. Proof jobs use this.
    Strict,
}

impl SelectionPolicy {
    /// `Strict` when the requirement flag is set, `Prefer` otherwise.
    #[must_use]
    pub fn from_required(required: bool) -> Self {
        if required {
            Self::Strict
        } else {
            Self::Prefer
        }
    }

    /// `Strict` under `E2040_REQUIRE_GPU=1` (the GPU tests' switch, read by
    /// `test_gpu::required_by_env`). Cold startup path only.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_required(crate::test_gpu::required_by_env())
    }
}

/// Everything [`select`] needs besides the adapter rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SelectionRequest {
    /// The adapter asked for.
    pub choice: AdapterChoice,
    /// Where `choice` came from.
    pub source: SelectionSource,
    /// What a fallback does.
    pub policy: SelectionPolicy,
    /// Whether a software adapter may win (`E2040_ALLOW_SOFTWARE=1`).
    pub allow_software: bool,
}

impl SelectionRequest {
    /// The request the process env describes. Cold startup path only.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            choice: choice_from_env(),
            source: SelectionSource::from_env(),
            policy: SelectionPolicy::from_env(),
            allow_software: allow_software(),
        }
    }
}

/// The report: who picked which adapter, and whether it was a fallback.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackendSelection {
    /// Where the request came from.
    pub source: SelectionSource,
    /// What was asked; `None` when the caller opened the adapter itself.
    pub requested: Option<AdapterChoice>,
    /// The adapter that drew (name, backend, kind, vendor).
    pub selected: AdapterSummary,
    /// True when a software adapter won or the best adapter could not present.
    pub fallback_used: bool,
    /// Why this adapter won.
    pub reason: &'static str,
    /// What to do about a fallback; `none` when there was none.
    pub remedy: &'static str,
}

impl BackendSelection {
    /// The report for an adapter the caller opened itself: nothing was
    /// requested and nothing was ranked, so it is never a fallback.
    #[must_use]
    pub fn caller_opened(selected: AdapterSummary) -> Self {
        Self {
            source: SelectionSource::Caller,
            requested: None,
            selected,
            fallback_used: false,
            reason: REASON_CALLER,
            remedy: REMEDY_NONE,
        }
    }

    /// The one stable receipt line:
    /// `backend: source=.. requested=.. selected=".." api=.. kind=.. fallback=no|yes reason=".." remedy=".."`.
    /// Free text is quoted with `"` mapped to `'`, so the line never breaks.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "backend: source={} requested={} selected={} api={} kind={} fallback={} reason={} remedy={}",
            self.source.name(),
            requested_text(self.requested.as_ref()),
            quote(&self.selected.name),
            self.selected.backend.name(),
            self.selected.kind.name(),
            if self.fallback_used { "yes" } else { "no" },
            quote(self.reason),
            quote(self.remedy),
        )
    }
}

/// Why [`select`] refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SelectionError {
    /// The adapter pick itself failed (nothing enumerated, only software, no match).
    Pick(AdapterError),
    /// [`SelectionPolicy::Strict`] refused a fallback.
    Fallback {
        /// The adapter the fallback would have used.
        selected: String,
        /// Why it would have won.
        reason: &'static str,
        /// What to do instead.
        remedy: &'static str,
    },
}

impl SelectionError {
    /// What to do about this failure.
    #[must_use]
    pub fn remedy(&self) -> &'static str {
        match self {
            Self::Pick(AdapterError::Empty) => REMEDY_EMPTY,
            Self::Pick(AdapterError::OnlySoftware(_)) => REMEDY_ONLY_SOFTWARE,
            Self::Pick(AdapterError::NoMatch(_)) => REMEDY_NO_MATCH,
            Self::Fallback { remedy, .. } => remedy,
        }
    }
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pick(error) => write!(f, "{error}; remedy: {}", self.remedy()),
            Self::Fallback {
                selected,
                reason,
                remedy,
            } => write!(
                f,
                "strict backend selection refuses the fallback to {selected} ({reason}); remedy: {remedy}"
            ),
        }
    }
}

impl Error for SelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Pick(error) => Some(error),
            Self::Fallback { .. } => None,
        }
    }
}

/// Pick the winning row of `list` for `request` and report it. `compatible`
/// marks the surface-compatible rows (`None`: no surface, every row counts).
/// Returns the winning index with its [`BackendSelection`]. A pick failure is
/// [`SelectionError::Pick`]; a fallback under [`SelectionPolicy::Strict`] is
/// [`SelectionError::Fallback`]. Pure; cold startup path only.
pub fn select(
    list: &[AdapterSummary],
    compatible: Option<&[bool]>,
    request: &SelectionRequest,
) -> Result<(usize, BackendSelection), SelectionError> {
    let pick = |mask: Option<&[bool]>| match mask {
        Some(mask) => choose_compatible(list, mask, &request.choice, request.allow_software),
        None => choose_with(list, &request.choice, request.allow_software),
    };
    let index = pick(compatible).map_err(SelectionError::Pick)?;
    let selected = list
        .get(index)
        .cloned()
        .ok_or(SelectionError::Pick(AdapterError::Empty))?;
    let software = selected.kind == AdapterKind::Cpu;
    // Without a surface the masked and unmasked picks are the same pick.
    let cannot_present = compatible.is_some() && pick(None).is_ok_and(|best| best != index);
    let (fallback_used, reason, remedy) = if software {
        (true, REASON_SOFTWARE, REMEDY_SOFTWARE)
    } else if cannot_present {
        (true, REASON_SURFACE, REMEDY_SURFACE)
    } else if request.choice == AdapterChoice::Auto {
        (false, REASON_BEST, REMEDY_NONE)
    } else {
        (false, REASON_REQUESTED, REMEDY_NONE)
    };
    if fallback_used && request.policy == SelectionPolicy::Strict {
        return Err(SelectionError::Fallback {
            selected: selected.name,
            reason,
            remedy,
        });
    }
    let report = BackendSelection {
        source: request.source,
        requested: Some(request.choice.clone()),
        selected,
        fallback_used,
        reason,
        remedy,
    };
    Ok((index, report))
}

/// `auto`, `discrete`, `integrated`, a quoted `name:<fragment>`, or `none`.
fn requested_text(requested: Option<&AdapterChoice>) -> String {
    match requested {
        None => "none".to_string(),
        Some(AdapterChoice::Auto) => "auto".to_string(),
        Some(AdapterChoice::Discrete) => "discrete".to_string(),
        Some(AdapterChoice::Integrated) => "integrated".to_string(),
        Some(AdapterChoice::Name(fragment)) => quote(&format!("name:{fragment}")),
    }
}

/// `text` in double quotes with `"` mapped to `'` and control characters to
/// a space, so a driver-supplied name cannot split or forge a receipt line.
fn quote(text: &str) -> String {
    let body: String = text
        .chars()
        .map(|c| match c {
            '"' => '\'',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    format!("\"{body}\"")
}

#[cfg(test)]
mod tests;
