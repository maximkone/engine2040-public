//! modes: display mode list plus validated swap keeping last-good.
//!
//! Consumer: the `app` windowed leg (`windowed.rs`) and the settings
//! screen stage validated swaps here (frame boundaries only).
//! Owner: this module owns ranking plus validation; the operator owns real
//! adapter enumeration (winit/wgpu calls stay operator-side, never headless).
//! Units: pixels (`w_px`, `h_px`), hertz (`refresh_hz`), frames (`u64`).
//! Determinism: ranking is a stable sort over plain data, so the same
//! adapter list gives the same ranked list twice. Errors: [`ModesError`];
//! unsupported modes fail loudly, never fall back silently.

/// Cap for human-readable notices carried in errors and reports.
pub const MAX_NOTICE_CHARS: usize = 256;
/// Smallest windowed extent accepted (pixels); below this fails loudly.
pub const MIN_WINDOWED_PX: u32 = 320;
/// Largest extent accepted per axis (pixels); above this fails loudly.
pub const MAX_EXTENT_PX: u32 = 16_384;
/// Largest refresh accepted (Hz); above this fails loudly.
pub const MAX_REFRESH_HZ: u32 = 1000;

/// One adapter-reported mode as plain data. Enumeration stays operator-side
/// (winit/wgpu); this struct is the headless-testable boundary: adapter
/// modes in, ranked list out. Units: pixels, Hz.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AdapterMode {
    /// Width in pixels, nonzero.
    pub w_px: u32,
    /// Height in pixels, nonzero.
    pub h_px: u32,
    /// Refresh in Hz, nonzero.
    pub refresh_hz: u32,
}

impl AdapterMode {
    /// Pixel area as `u64` (no overflow on either axis).
    #[must_use]
    pub fn area_px(self) -> u64 {
        u64::from(self.w_px) * u64::from(self.h_px)
    }

    /// True when every field is within its accepted range.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.w_px >= MIN_WINDOWED_PX
            && self.w_px <= MAX_EXTENT_PX
            && self.h_px >= MIN_WINDOWED_PX
            && self.h_px <= MAX_EXTENT_PX
            && self.refresh_hz > 0
            && self.refresh_hz <= MAX_REFRESH_HZ
    }
}

/// Requested display state. `Fullscreen` names its exact adapter mode;
/// nothing here probes a device, so unlisted modes are refused, not probed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DisplayMode {
    /// Windowed at `w_px` x `h_px` pixels.
    Windowed {
        /// Width in pixels.
        w_px: u32,
        /// Height in pixels.
        h_px: u32,
    },
    /// Borderless fullscreen at the desktop size (no resolution claimed).
    Borderless,
    /// Exclusive fullscreen at exactly this adapter mode (pixels, Hz).
    Fullscreen {
        /// Width in pixels.
        w_px: u32,
        /// Height in pixels.
        h_px: u32,
        /// Refresh in Hz.
        refresh_hz: u32,
    },
}

/// One validated swap: mode applied at a frame boundary.
/// Resolution and refresh travel together; mid-frame
/// application is refused by [`ModeManager::apply_pending`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapRequest {
    /// Requested display state.
    pub mode: DisplayMode,
}

/// TEMPORARY local mirror of the `engine-observe` supervisor heal shapes
/// (`HealKind::{LastGood, Reconnect, Degrade, Quarantine}`) for the
/// display-swap path. Zero crate imports by packet contract, so the shapes
/// are copied here; the orchestrator replaces them with a documented
/// `game -> observe` edge. Do not extend: new recoveries go through the
/// supervisor, not this enum.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ModeHeal {
    /// Hold the last-good mode; the failed swap changed nothing.
    KeepLastGood,
    /// Reopen/reconfigure the surface with backoff, last-good held meanwhile.
    Reconnect,
    /// Lower presentation quality within a floor, last-good mode held.
    Degrade,
    /// Budget spent: quarantine loudly with an operator notice, never guess.
    Quarantine,
}

/// TEMPORARY local mirror of the `engine-render` surface fault shapes
/// (`SurfaceEvent::{Outdated, Lost}`) reported by the swapchain path. Same
/// packet rule as [`ModeHeal`]: copied here until the orchestrator wires
/// the documented `game -> render` edge. Do not extend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SurfaceFault {
    /// Surface is outdated (resize): reconfigure then retry.
    Outdated,
    /// Surface or device is lost: reconnect with backoff.
    Lost,
}

/// Map one surface fault to its heal advice. Pure.
#[must_use]
pub fn heal_for(fault: SurfaceFault) -> ModeHeal {
    match fault {
        SurfaceFault::Outdated | SurfaceFault::Lost => ModeHeal::Reconnect,
    }
}

/// Typed display-mode failures. Every refusal is loud: no silent fallback
/// to another mode anywhere in this module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ModesError {
    /// Adapter list was empty (or held no valid mode) while the request
    /// needs one; `Borderless` never reports this.
    EmptyAdapterList,
    /// Zero or out-of-range extent; culprit carried in pixels.
    InvalidSize {
        /// Rejected width in pixels.
        w_px: u32,
        /// Rejected height in pixels.
        h_px: u32,
    },
    /// Zero or out-of-range refresh; culprit carried in Hz.
    InvalidRefresh {
        /// Rejected refresh in Hz.
        refresh_hz: u32,
    },
    /// Mode is well-formed but no adapter reported it; request refused,
    /// caller keeps last-good. Never a silent fallback.
    Unsupported {
        /// Requested width in pixels.
        w_px: u32,
        /// Requested height in pixels.
        h_px: u32,
        /// Requested refresh in Hz (0 for windowed refusals).
        refresh_hz: u32,
    },
    /// `apply_pending` called before the staged frame boundary.
    BoundaryNotReached {
        /// Frame the caller is on.
        frame: u64,
        /// Frame the staged swap applies at.
        applies_at_frame: u64,
    },
    /// `apply_pending` called with nothing staged.
    NothingStaged,
    /// The device refused the validated swap; the manager kept last-good.
    /// The notice is bounded ([`MAX_NOTICE_CHARS`]) and names the fault.
    ApplyFailed {
        /// Heal advice for the fault.
        heal: ModeHeal,
        /// Bounded notice naming the fault and the kept mode.
        notice: String,
    },
}

impl core::fmt::Display for ModesError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyAdapterList => write!(f, "no adapter modes reported"),
            Self::InvalidSize { w_px, h_px } => {
                write!(f, "invalid mode size ({w_px}x{h_px}px)")
            }
            Self::InvalidRefresh { refresh_hz } => {
                write!(f, "invalid refresh ({refresh_hz}Hz)")
            }
            Self::Unsupported {
                w_px,
                h_px,
                refresh_hz,
            } => write!(
                f,
                "unsupported mode ({w_px}x{h_px}px at {refresh_hz}Hz); keeping last-good"
            ),
            Self::BoundaryNotReached {
                frame,
                applies_at_frame,
            } => write!(
                f,
                "swap staged for frame {applies_at_frame}, now at {frame}"
            ),
            Self::NothingStaged => write!(f, "no display swap staged"),
            Self::ApplyFailed { notice, .. } => {
                write!(f, "display swap failed, last-good kept ({notice})")
            }
        }
    }
}

impl std::error::Error for ModesError {}

/// Bound a notice so errors and reports stay small and printable.
fn cap_notice(note: &str) -> String {
    let out: String = note.chars().take(MAX_NOTICE_CHARS).collect();
    if out.len() < note.len() {
        format!("{out}...")
    } else {
        out
    }
}

/// Rank adapter modes for the settings list: drops invalid entries, then
/// sorts by pixel area descending, refresh descending, width descending,
/// height descending. Stable and deterministic: the same input list gives
/// the same ranked list twice. Pure; adapter enumeration stays
/// operator-side (this only orders plain data).
#[must_use]
pub fn rank_modes(modes: &[AdapterMode]) -> Vec<AdapterMode> {
    let mut ranked: Vec<AdapterMode> = modes
        .iter()
        .copied()
        .filter(|mode| mode.is_valid())
        .collect();
    ranked.sort_by(|a, b| {
        b.area_px()
            .cmp(&a.area_px())
            .then_with(|| b.refresh_hz.cmp(&a.refresh_hz))
            .then_with(|| b.w_px.cmp(&a.w_px))
            .then_with(|| b.h_px.cmp(&a.h_px))
    });
    ranked.dedup();
    ranked
}

/// True when the ranked list contains `mode` exactly (pixels and Hz).
#[must_use]
pub fn contains_mode(ranked: &[AdapterMode], mode: &AdapterMode) -> bool {
    ranked.contains(mode)
}

/// Largest reported extent per axis (pixels); `None` on an empty list.
fn max_extent(modes: &[AdapterMode]) -> Option<(u32, u32)> {
    let mut extent: Option<(u32, u32)> = None;
    for mode in modes {
        extent = Some(match extent {
            None => (mode.w_px, mode.h_px),
            Some((w, h)) => (w.max(mode.w_px), h.max(mode.h_px)),
        });
    }
    extent
}

/// Validate one swap request against the ranked adapter list. `Borderless`
/// always passes (desktop size, no claim); `Windowed` must be nonzero and
/// fit the largest reported extent; `Fullscreen` must match a reported
/// mode exactly. Pure.
pub fn validate_request(request: &SwapRequest, ranked: &[AdapterMode]) -> Result<(), ModesError> {
    match request.mode {
        DisplayMode::Borderless => Ok(()),
        DisplayMode::Windowed { w_px, h_px } => {
            if w_px == 0 || h_px == 0 || w_px > MAX_EXTENT_PX || h_px > MAX_EXTENT_PX {
                return Err(ModesError::InvalidSize { w_px, h_px });
            }
            match max_extent(ranked) {
                None => Err(ModesError::EmptyAdapterList),
                Some((max_w, max_h)) if w_px <= max_w && h_px <= max_h => Ok(()),
                Some(_) => Err(ModesError::Unsupported {
                    w_px,
                    h_px,
                    refresh_hz: 0,
                }),
            }
        }
        DisplayMode::Fullscreen {
            w_px,
            h_px,
            refresh_hz,
        } => {
            if w_px == 0 || h_px == 0 || w_px > MAX_EXTENT_PX || h_px > MAX_EXTENT_PX {
                return Err(ModesError::InvalidSize { w_px, h_px });
            }
            if refresh_hz == 0 || refresh_hz > MAX_REFRESH_HZ {
                return Err(ModesError::InvalidRefresh { refresh_hz });
            }
            if ranked.is_empty() {
                return Err(ModesError::EmptyAdapterList);
            }
            let probe = AdapterMode {
                w_px,
                h_px,
                refresh_hz,
            };
            if contains_mode(ranked, &probe) {
                Ok(())
            } else {
                Err(ModesError::Unsupported {
                    w_px,
                    h_px,
                    refresh_hz,
                })
            }
        }
    }
}

/// One staged swap: the validated request plus the frame it applies at.
/// Mid-frame application is refused; see [`ModeManager::apply_pending`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StagedSwap {
    /// Validated request waiting for its boundary.
    pub request: SwapRequest,
    /// Frame the swap applies at.
    pub applies_at_frame: u64,
}

/// Report for an applied swap: what changed and what came before.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ApplyReport {
    /// Mode now active.
    pub applied: DisplayMode,
    /// Mode active before this swap.
    pub previous: DisplayMode,
    /// Heal advice carried for the log (`KeepLastGood` on success).
    pub heal: ModeHeal,
    /// Bounded human-readable note for the log.
    pub note: String,
}

/// Display-mode owner: current plus last-good plus one staged swap. A
/// failed device swap keeps last-good (surface fault paths): `current` never moves
/// unless the device confirms, and refusals never pick another mode.
/// Budget: swaps run on the UI thread at frame boundaries (apply within
/// one frame hitch); nothing here runs per simulation tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModeManager {
    /// Active mode; moves only after validation plus device confirm.
    current: DisplayMode,
    /// Last device-confirmed mode; restored (kept) on any failure.
    last_good: DisplayMode,
    /// One staged swap waiting for its frame boundary, if any.
    pending: Option<StagedSwap>,
}

impl ModeManager {
    /// Start on `initial` after validating it against `ranked`. A bad
    /// initial fails loudly; there is no default to fall back to.
    pub fn new(initial: DisplayMode, ranked: &[AdapterMode]) -> Result<Self, ModesError> {
        validate_request(&SwapRequest { mode: initial }, ranked)?;
        Ok(Self {
            current: initial,
            last_good: initial,
            pending: None,
        })
    }

    /// Active mode.
    #[must_use]
    pub fn current(&self) -> DisplayMode {
        self.current
    }

    /// Last device-confirmed mode (equals `current` whenever no swap is
    /// in flight or failing).
    #[must_use]
    pub fn last_good(&self) -> DisplayMode {
        self.last_good
    }

    /// Stage a validated swap for `applies_at_frame`. Replaces any earlier
    /// staged swap (settings rows re-stage freely before the boundary).
    pub fn stage(
        &mut self,
        request: SwapRequest,
        ranked: &[AdapterMode],
        applies_at_frame: u64,
    ) -> Result<(), ModesError> {
        validate_request(&request, ranked)?;
        self.pending = Some(StagedSwap {
            request,
            applies_at_frame,
        });
        Ok(())
    }

    /// Apply the staged swap at its frame boundary. Before the boundary
    /// this fails with [`ModesError::BoundaryNotReached`] and stages
    /// nothing new; `apply` runs the device switch and reports the surface
    /// fault on failure. Success moves `current` and `last_good`
    /// together; failure keeps both and reports loudly.
    pub fn apply_pending(
        &mut self,
        frame: u64,
        apply: impl FnOnce(&DisplayMode) -> Result<(), SurfaceFault>,
    ) -> Result<ApplyReport, ModesError> {
        let staged = self.pending.ok_or(ModesError::NothingStaged)?;
        if frame < staged.applies_at_frame {
            return Err(ModesError::BoundaryNotReached {
                frame,
                applies_at_frame: staged.applies_at_frame,
            });
        }
        self.pending = None;
        match apply(&staged.request.mode) {
            Ok(()) => {
                let previous = self.current;
                self.current = staged.request.mode;
                self.last_good = staged.request.mode;
                Ok(ApplyReport {
                    applied: staged.request.mode,
                    previous,
                    heal: ModeHeal::KeepLastGood,
                    note: cap_notice("display swap applied at frame boundary"),
                })
            }
            Err(fault) => {
                let heal = heal_for(fault);
                let notice = cap_notice(&format!(
                    "display swap {fault:?} refused, last-good {self:?} kept",
                    self = self.last_good
                ));
                Err(ModesError::ApplyFailed { heal, notice })
            }
        }
    }

    /// Validate plus immediately apply one swap (operator drill path; the
    /// settings screen prefers [`ModeManager::stage`] plus
    /// [`ModeManager::apply_pending`]). Failure keeps last-good loudly.
    pub fn try_swap(
        &mut self,
        request: SwapRequest,
        ranked: &[AdapterMode],
        apply: impl FnOnce(&DisplayMode) -> Result<(), SurfaceFault>,
    ) -> Result<ApplyReport, ModesError> {
        validate_request(&request, ranked)?;
        let previous = self.current;
        match apply(&request.mode) {
            Ok(()) => {
                self.current = request.mode;
                self.last_good = request.mode;
                Ok(ApplyReport {
                    applied: request.mode,
                    previous,
                    heal: heal_for(SurfaceFault::Outdated),
                    note: cap_notice("display swap applied"),
                })
            }
            Err(fault) => {
                let heal = heal_for(fault);
                let notice = cap_notice(&format!(
                    "display swap {fault:?} refused, last-good {:?} kept",
                    self.last_good
                ));
                Err(ModesError::ApplyFailed { heal, notice })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapters() -> Vec<AdapterMode> {
        vec![
            AdapterMode {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 60,
            },
            AdapterMode {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 144,
            },
            AdapterMode {
                w_px: 1280,
                h_px: 720,
                refresh_hz: 60,
            },
            AdapterMode {
                w_px: 3840,
                h_px: 2160,
                refresh_hz: 60,
            },
        ]
    }

    #[test]
    fn rank_orders_by_area_then_refresh_and_dedupes() {
        let mut modes = adapters();
        modes.push(AdapterMode {
            w_px: 1920,
            h_px: 1080,
            refresh_hz: 60,
        });
        modes.push(AdapterMode {
            w_px: 0,
            h_px: 1080,
            refresh_hz: 60,
        });
        let ranked = rank_modes(&modes);
        assert_eq!(
            ranked,
            vec![
                AdapterMode {
                    w_px: 3840,
                    h_px: 2160,
                    refresh_hz: 60,
                },
                AdapterMode {
                    w_px: 1920,
                    h_px: 1080,
                    refresh_hz: 144,
                },
                AdapterMode {
                    w_px: 1920,
                    h_px: 1080,
                    refresh_hz: 60,
                },
                AdapterMode {
                    w_px: 1280,
                    h_px: 720,
                    refresh_hz: 60,
                },
            ]
        );
    }

    #[test]
    fn rank_is_deterministic_on_reversed_input() {
        let mut modes = adapters();
        modes.reverse();
        assert_eq!(rank_modes(&adapters()), rank_modes(&modes));
    }

    #[test]
    fn fullscreen_exact_match_passes_wrong_refresh_refused() {
        let ranked = rank_modes(&adapters());
        let rate_144 = SwapRequest {
            mode: DisplayMode::Fullscreen {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 144,
            },
        };
        assert!(validate_request(&rate_144, &ranked).is_ok());
        let rate_120 = SwapRequest {
            mode: DisplayMode::Fullscreen {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 120,
            },
        };
        assert_eq!(
            validate_request(&rate_120, &ranked),
            Err(ModesError::Unsupported {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 120,
            })
        );
    }

    #[test]
    fn windowed_zero_refused_oversize_refused_borderless_passes() {
        let ranked = rank_modes(&adapters());
        let zero = SwapRequest {
            mode: DisplayMode::Windowed { w_px: 0, h_px: 720 },
        };
        assert_eq!(
            validate_request(&zero, &ranked),
            Err(ModesError::InvalidSize { w_px: 0, h_px: 720 })
        );
        let huge = SwapRequest {
            mode: DisplayMode::Windowed {
                w_px: 7680,
                h_px: 4320,
            },
        };
        assert_eq!(
            validate_request(&huge, &ranked),
            Err(ModesError::Unsupported {
                w_px: 7680,
                h_px: 4320,
                refresh_hz: 0,
            })
        );
        let borderless = SwapRequest {
            mode: DisplayMode::Borderless,
        };
        assert!(validate_request(&borderless, &ranked).is_ok());
        assert!(validate_request(&borderless, &[]).is_ok());
    }

    #[test]
    fn empty_adapter_list_refuses_ranked_requests() {
        let ranked: Vec<AdapterMode> = Vec::new();
        let full = SwapRequest {
            mode: DisplayMode::Fullscreen {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 60,
            },
        };
        assert_eq!(
            validate_request(&full, &ranked),
            Err(ModesError::EmptyAdapterList)
        );
        assert!(rank_modes(&[]).is_empty());
    }

    #[test]
    fn staged_swap_applies_only_at_its_frame() {
        let ranked = rank_modes(&adapters());
        let mut manager = ModeManager::new(
            DisplayMode::Windowed {
                w_px: 1280,
                h_px: 720,
            },
            &ranked,
        )
        .expect("valid initial");
        manager
            .stage(
                SwapRequest {
                    mode: DisplayMode::Borderless,
                },
                &ranked,
                10,
            )
            .expect("valid stage");
        assert_eq!(
            manager.apply_pending(9, |_| Ok(())),
            Err(ModesError::BoundaryNotReached {
                frame: 9,
                applies_at_frame: 10,
            })
        );
        assert_eq!(
            manager.current(),
            DisplayMode::Windowed {
                w_px: 1280,
                h_px: 720
            }
        );
        let report = manager
            .apply_pending(10, |_| Ok(()))
            .expect("boundary reached");
        assert_eq!(report.applied, DisplayMode::Borderless);
        assert_eq!(
            report.previous,
            DisplayMode::Windowed {
                w_px: 1280,
                h_px: 720
            }
        );
        assert_eq!(manager.last_good(), DisplayMode::Borderless);
    }

    #[test]
    fn failed_device_swap_keeps_last_good_loudly() {
        let ranked = rank_modes(&adapters());
        let start = DisplayMode::Windowed {
            w_px: 1280,
            h_px: 720,
        };
        let mut manager = ModeManager::new(start, &ranked).expect("valid initial");
        let target = SwapRequest {
            mode: DisplayMode::Fullscreen {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 144,
            },
        };
        let err = manager
            .try_swap(target, &ranked, |_| Err(SurfaceFault::Lost))
            .expect_err("lost surface must fail");
        match err {
            ModesError::ApplyFailed { heal, notice } => {
                assert_eq!(heal, ModeHeal::Reconnect);
                assert!(
                    notice.contains("kept"),
                    "notice names kept last-good: {notice}"
                );
            }
            other => panic!("wrong error: {other:?}"),
        }
        assert_eq!(manager.current(), start);
        assert_eq!(manager.last_good(), start);
        // Outdated maps to reconnect advice as well.
        assert_eq!(heal_for(SurfaceFault::Outdated), ModeHeal::Reconnect);
    }

    #[test]
    fn successful_swap_moves_current_and_last_good_together() {
        let ranked = rank_modes(&adapters());
        let mut manager =
            ModeManager::new(DisplayMode::Borderless, &ranked).expect("valid initial");
        let target = SwapRequest {
            mode: DisplayMode::Fullscreen {
                w_px: 1920,
                h_px: 1080,
                refresh_hz: 60,
            },
        };
        let report = manager
            .try_swap(target, &ranked, |_| Ok(()))
            .expect("exact mode applies");
        assert_eq!(report.applied, target.mode);
        assert_eq!(report.previous, DisplayMode::Borderless);
        assert_eq!(manager.current(), target.mode);
        assert_eq!(manager.last_good(), target.mode);
    }

    #[test]
    fn bad_initial_has_no_silent_default() {
        let ranked = rank_modes(&adapters());
        assert_eq!(
            ModeManager::new(
                DisplayMode::Fullscreen {
                    w_px: 800,
                    h_px: 600,
                    refresh_hz: 60,
                },
                &ranked
            ),
            Err(ModesError::Unsupported {
                w_px: 800,
                h_px: 600,
                refresh_hz: 60,
            })
        );
        assert!(ModeManager::new(DisplayMode::Borderless, &[]).is_ok());
    }

    #[test]
    fn ladder_light_gate_covers_display_modes() {
        const LADDER_SRC: &str = include_str!("ladder.rs");
        const MODES_SRC: &str = include_str!("modes.rs");
        for src in [LADDER_SRC, MODES_SRC] {
            let head = src
                .split("#[cfg(test)]")
                .next()
                .expect("owned file keeps its cfg(test) marker");
            assert!(
                crate::ladder::is_light_clean(head),
                "display layer must stay LIGHT: {:?}",
                crate::ladder::first_cut_token(head)
            );
        }
        // Locked ladder content stays readable from the display layer: the
        // start-to-rank fixture ends on Legend with two owed repairs.
        let ladder = crate::ladder::start_to_rank();
        assert_eq!(ladder.rank(), 4);
        assert_eq!(crate::ladder::Ladder::rank_name(ladder.rank()), "Legend");
        assert_eq!(ladder.pending_repairs(), 2);
    }
}
