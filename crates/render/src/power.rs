//! power: battery-aware quality cap (GS40).
//!
//! Purpose: a laptop on battery should not draw full frames and cook its fan.
//! [`read_power`] is a thin OS leaf beside `backend::read_vram`
//! (`GetSystemPowerStatus`); [`power_cap`] is the pure rule that steps one
//! tier down when the machine runs on battery and is either low or in
//! battery saver.
//! Owner: render lane. Units: battery percent (%). Determinism: pure integer
//! data, no clock, no float, and never an input to `state_hash`; a consumer
//! reads it at startup or off the tick, never per frame. Budget: one Win32
//! call on the cold path; the rule is O(1).

use crate::quality::Quality;

/// Windows sentinel for a value it does not know (`BatteryLifePercent`,
/// `ACLineStatus`). Never read as a real battery level.
pub const UNKNOWN_PERCENT: u8 = 255;

/// Battery percent at or below which the cap steps in (%).
pub const LOW_BATTERY_PERCENT: u8 = 30;

/// Reason carried by a reading no leaf can take here: not Windows, or a build
/// without the `window` feature that links the `windows` crate.
pub const NO_LEAF: &str = "no power leaf on this target";

/// One OS power reading, or why there is none. Never a guessed value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerReading {
    /// Read from the OS.
    Known {
        /// Mains power is connected.
        ac_online: bool,
        /// Battery charge (%), [`UNKNOWN_PERCENT`] when the OS does not say.
        percent: u8,
        /// The OS battery saver is on.
        saver: bool,
    },
    /// Not readable here; a short static reason.
    Unknown(&'static str),
}

/// Read the OS power state (Win32 `GetSystemPowerStatus`). Cold path: call it
/// at startup or off the tick, never per frame. Other targets, and the
/// headless build that links no `windows` crate, return
/// [`PowerReading::Unknown`] with a reason.
#[must_use]
pub fn read_power() -> PowerReading {
    #[cfg(all(windows, feature = "window"))]
    {
        win_power::read()
    }
    #[cfg(not(all(windows, feature = "window")))]
    {
        PowerReading::Unknown(NO_LEAF)
    }
}

/// Cap a tier for battery: one tier down when the machine runs on battery and
/// sits at or below [`LOW_BATTERY_PERCENT`] or has battery saver on. Mains
/// power, a healthy battery, an unknown level and an unreadable reading all
/// keep the tier; `Potato` and `Web` are the floor and stay put.
#[must_use]
pub fn power_cap(reading: &PowerReading, tier: Quality) -> Quality {
    match *reading {
        PowerReading::Known {
            ac_online: true, ..
        } => tier,
        PowerReading::Known { percent, saver, .. } => {
            let low = percent != UNKNOWN_PERCENT && percent <= LOW_BATTERY_PERCENT;
            if low || saver {
                step_down(tier)
            } else {
                tier
            }
        }
        PowerReading::Unknown(_) => tier,
    }
}

/// One tier down, and never onto `Web` (a browser tab, not a battery step) or
/// `Mobile` (a device class, not a battery step).
const fn step_down(tier: Quality) -> Quality {
    match tier {
        Quality::High => Quality::Comfort,
        Quality::Comfort => Quality::Potato,
        Quality::Mobile => Quality::Potato,
        Quality::Potato | Quality::Web => tier,
    }
}

#[cfg(all(windows, feature = "window"))]
mod win_power {
    use super::{PowerReading, UNKNOWN_PERCENT};
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    /// `ACLineStatus`: mains power is off.
    const AC_OFFLINE: u8 = 0;
    /// `ACLineStatus`: mains power is on.
    const AC_ONLINE: u8 = 1;
    /// `SystemStatusFlag`: battery saver is on.
    const SAVER_ON: u8 = 1;
    /// Percent values above a full battery are the API's "unknown".
    const FULL_PERCENT: u8 = 100;

    /// Read one `SYSTEM_POWER_STATUS`; never panics.
    #[expect(
        unsafe_code,
        reason = "GetSystemPowerStatus is a Win32 call in the windows crate"
    )]
    pub(super) fn read() -> PowerReading {
        let mut status = SYSTEM_POWER_STATUS::default();
        // SAFETY: `status` is a local, fully sized out-parameter that lives
        // across the call; the API writes only that struct and no pointer
        // escapes this function.
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return PowerReading::Unknown("GetSystemPowerStatus failed");
        }
        let ac_online = match status.ACLineStatus {
            AC_ONLINE => true,
            AC_OFFLINE => false,
            _ => return PowerReading::Unknown("ac line status unknown"),
        };
        let percent = if status.BatteryLifePercent > FULL_PERCENT {
            UNKNOWN_PERCENT
        } else {
            status.BatteryLifePercent
        };
        PowerReading::Known {
            ac_online,
            percent,
            saver: status.SystemStatusFlag == SAVER_ON,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mains(percent: u8, saver: bool) -> PowerReading {
        PowerReading::Known {
            ac_online: true,
            percent,
            saver,
        }
    }

    fn battery(percent: u8, saver: bool) -> PowerReading {
        PowerReading::Known {
            ac_online: false,
            percent,
            saver,
        }
    }

    #[test]
    fn power_ac_full_battery_keeps_high() {
        assert_eq!(power_cap(&mains(100, false), Quality::High), Quality::High);
    }

    #[test]
    fn power_ac_keeps_every_tier() {
        for tier in Quality::ALL {
            assert_eq!(power_cap(&mains(5, true), tier), tier, "tier={tier:?}");
        }
    }

    #[test]
    fn power_low_battery_steps_one_tier() {
        assert_eq!(
            power_cap(&battery(25, false), Quality::High),
            Quality::Comfort
        );
        assert_eq!(
            power_cap(&battery(25, false), Quality::Comfort),
            Quality::Potato
        );
    }

    #[test]
    fn power_saver_steps_one_tier() {
        assert_eq!(
            power_cap(&battery(80, true), Quality::High),
            Quality::Comfort
        );
    }

    #[test]
    fn power_healthy_battery_keeps_tier() {
        assert_eq!(power_cap(&battery(31, false), Quality::High), Quality::High);
        assert_eq!(
            power_cap(&battery(LOW_BATTERY_PERCENT + 1, false), Quality::High),
            Quality::High
        );
    }

    #[test]
    fn power_potato_stays_potato() {
        assert_eq!(
            power_cap(&battery(10, true), Quality::Potato),
            Quality::Potato
        );
        assert_eq!(
            power_cap(&battery(10, false), Quality::Potato),
            Quality::Potato
        );
    }

    #[test]
    fn power_never_steps_onto_web() {
        assert_eq!(
            power_cap(&battery(10, true), Quality::Comfort),
            Quality::Potato
        );
        assert_eq!(power_cap(&battery(10, true), Quality::Web), Quality::Web);
    }

    #[test]
    fn power_never_steps_onto_the_mobile_tier() {
        // The mobile row is a device class, not a battery step: a desktop on
        // battery drops to the floor, a phone drops to the floor too.
        assert_eq!(
            power_cap(&battery(10, true), Quality::Mobile),
            Quality::Potato
        );
        assert_eq!(
            power_cap(&battery(90, false), Quality::Mobile),
            Quality::Mobile
        );
    }

    #[test]
    fn power_unknown_keeps_tier() {
        let reading = PowerReading::Unknown(NO_LEAF);
        for tier in Quality::ALL {
            assert_eq!(power_cap(&reading, tier), tier, "tier={tier:?}");
        }
    }

    #[test]
    fn power_unknown_percent_is_not_a_level() {
        // 255 is the API's "unknown", never the lowest battery: no cap.
        assert_eq!(
            power_cap(&battery(UNKNOWN_PERCENT, false), Quality::High),
            Quality::High
        );
        // Battery saver still caps even when the level is unknown.
        assert_eq!(
            power_cap(&battery(UNKNOWN_PERCENT, true), Quality::High),
            Quality::Comfort
        );
    }

    #[test]
    fn power_read_never_panics() {
        // The leaf may return Known or Unknown on this machine; only the
        // shape is asserted, never a measured number.
        match read_power() {
            PowerReading::Known { percent, .. } => {
                assert!(percent <= 100 || percent == UNKNOWN_PERCENT);
            }
            PowerReading::Unknown(why) => assert!(!why.is_empty()),
        }
    }

    #[test]
    fn power_leaf_reason_names_the_target() {
        #[cfg(not(all(windows, feature = "window")))]
        assert_eq!(read_power(), PowerReading::Unknown(NO_LEAF));
        #[cfg(all(windows, feature = "window"))]
        assert_eq!(NO_LEAF, "no power leaf on this target");
    }
}
