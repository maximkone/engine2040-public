//! Pure integer VRAM budget math (presentation pool only).
//! Consumer: the perf harness `--vram-plan` report via [`VramPlan`]'s
//! `Display` plus the later orchestrator step that feeds a Windows
//! video-memory reading into [`plan`]. Units: bytes as `u64`.
//! Determinism: integer ops only, no clock, no float, no OS query;
//! same inputs give the same plan. Never enters the simulation hash.

use std::fmt::{self, Display, Formatter};

/// Bytes in one GiB. All inputs and outputs in this module are bytes as `u64`.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// Video-memory reading fed into [`plan`]. All fields are bytes as `u64`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VramInfo {
    /// Total video memory available, bytes as `u64`.
    pub budget_bytes: u64,
    /// Currently used video memory, bytes as `u64`.
    pub current_bytes: u64,
    /// Bytes the waiter wants reserved, bytes as `u64`.
    pub requested_bytes: u64,
}

/// Planned reservation plus the adjusted budget and usage. All bytes as `u64`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VramPlan {
    /// Reserved bytes held back for the waiter, bytes as `u64`.
    pub reservation_bytes: u64,
    /// Usable budget after reservation and a 5% headroom cut, bytes as `u64`.
    pub budget_bytes: u64,
    /// Current usage minus the reservation, bytes as `u64`.
    pub usage_bytes: u64,
}

impl Display for VramPlan {
    /// One-line report for the perf harness: reservation, budget, usage
    /// in bytes plus integer pressure percent. Calls [`pressure_pct`],
    /// so the harness line and the gate read the same number.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let pressure = pressure_pct(self.usage_bytes, self.budget_bytes);
        write!(
            f,
            "reservation={} budget={} usage={} pressure={}",
            self.reservation_bytes, self.budget_bytes, self.usage_bytes, pressure
        )
    }
}

/// Reserved bytes held back for the waiter: `min(budget / 2, requested)`.
/// Single owner for the reservation rule; [`plan`] reads it. Integer
/// division floors, so an odd budget reserves the floor half. Pure.
#[must_use]
pub fn reservation_for(budget_bytes: u64, requested_bytes: u64) -> u64 {
    (budget_bytes / 2).min(requested_bytes)
}

/// Plan a reservation from a video-memory reading with integer math only.
///
/// Reservation is [`reservation_for`]; the adjusted budget keeps
/// 95% of what remains (via `u128` then saturated to `u64`); usage is
/// `current.saturating_sub(reservation)`. Same inputs give the same plan.
/// Caller: `plan` (same file) is the production wire for [`reservation_for`].
pub fn plan(info: VramInfo) -> VramPlan {
    let reservation_bytes = reservation_for(info.budget_bytes, info.requested_bytes);
    let remaining = info.budget_bytes.saturating_sub(reservation_bytes);
    let budget_bytes = u64::try_from(u128::from(remaining) * 95 / 100).unwrap_or(u64::MAX);
    let usage_bytes = info.current_bytes.saturating_sub(reservation_bytes);
    VramPlan {
        reservation_bytes,
        budget_bytes,
        usage_bytes,
    }
}

/// Usage pressure as an integer percent. Zero budget returns 0 for zero
/// usage else 101; otherwise `usage * 100 / budget` via `u128` saturated
/// to `u64`. Any usage above budget reads at least 101, so a just-over
/// budget frame never hides at 100. No panic, no float.
pub fn pressure_pct(usage_bytes: u64, budget_bytes: u64) -> u64 {
    if budget_bytes == 0 {
        if usage_bytes == 0 {
            0
        } else {
            101
        }
    } else {
        let pct = u64::try_from(u128::from(usage_bytes) * 100 / u128::from(budget_bytes))
            .unwrap_or(u64::MAX);
        if usage_bytes > budget_bytes && pct <= 100 {
            101
        } else {
            pct
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept_info() -> VramInfo {
        VramInfo {
            budget_bytes: 6 * GIB,
            current_bytes: 2 * GIB,
            requested_bytes: GIB,
        }
    }

    #[test]
    fn budget_vram_accept_numbers() {
        let got = plan(accept_info());
        assert_eq!(got.reservation_bytes, GIB);
        assert_eq!(got.budget_bytes, 5_100_273_664);
        assert_eq!(got.usage_bytes, GIB);
    }

    #[test]
    fn budget_vram_pressure_over_100() {
        let got = plan(accept_info());
        let accept = pressure_pct(got.usage_bytes, got.budget_bytes);
        assert!(accept < 100, "accept pressure {accept} must stay under 100");
        let over = pressure_pct(got.budget_bytes.saturating_mul(2), got.budget_bytes);
        assert!(over > 100, "over-budget pressure {over} must read over 100");
        assert_eq!(pressure_pct(0, 0), 0);
        assert_eq!(pressure_pct(1, 0), 101);
    }

    #[test]
    fn budget_vram_stable_x2() {
        let info = accept_info();
        let first = plan(info);
        let second = plan(info);
        assert_eq!(first, second);
        assert_eq!(
            pressure_pct(first.usage_bytes, first.budget_bytes),
            pressure_pct(second.usage_bytes, second.budget_bytes)
        );
    }

    #[test]
    fn vram_display_reports_accept_plan() {
        let got = plan(accept_info());
        let text = format!("{got}");
        assert!(text.contains("reservation=1073741824"), "{text}");
        assert!(text.contains("budget=5100273664"), "{text}");
        assert!(text.contains("usage=1073741824"), "{text}");
        assert!(text.contains("pressure="), "{text}");
    }

    #[test]
    fn vram_display_zero_budget_names_pressure() {
        let empty = VramPlan {
            reservation_bytes: 0,
            budget_bytes: 0,
            usage_bytes: 0,
        };
        assert!(
            format!("{empty}").contains("pressure=0"),
            "zero on zero reads 0"
        );
        let stranded = VramPlan {
            reservation_bytes: 0,
            budget_bytes: 0,
            usage_bytes: 1,
        };
        assert!(
            format!("{stranded}").contains("pressure=101"),
            "any usage on zero budget reads 101"
        );
    }

    #[test]
    fn budget_vram_pressure_just_over_reads_over_100() {
        // Any usage above budget must read over 100, even by one byte.
        // Floor division alone gives 100 for budget + 1, so the gate
        // would miss a just-over-budget frame.
        let budget = 5_100_273_664u64;
        assert_eq!(pressure_pct(budget, budget), 100);
        assert!(
            pressure_pct(budget + 1, budget) > 100,
            "one byte over budget must read over 100"
        );
    }

    #[test]
    fn reservation_for_caps_at_half_budget() {
        // Single owner for `min(budget / 2, requested)`; `plan` reads it.
        assert_eq!(reservation_for(6 * GIB, GIB), GIB);
        assert_eq!(reservation_for(7, 10), 3);
        assert_eq!(reservation_for(0, 100), 0);
        assert_eq!(reservation_for(u64::MAX, u64::MAX), u64::MAX / 2);
    }
}
