//! tier_rules: a GPU's facts to a quality tier through a first-match rule table (GS11).
//!
//! The shape is Chromium's `gpu_control_list`. A `Rule` names what it matches in a
//! `Cond` (vendor id, device kind, a VRAM range in MB, a driver version with an operator),
//! may list `except` conditions that void the match, and carries the result: a tier or a
//! refusal, plus named workarounds. The first rule that matches wins. The table is data an AI
//! can edit; `decide` stays a plain loop.
//!
//! Defaults, in table order: a CPU adapter is refused, an integrated GPU is Potato, a discrete
//! GPU under 4 GB is Potato, a discrete GPU with 4 GB or more is Comfort, anything else
//! (including a discrete GPU whose VRAM is unknown) falls to Potato. High is never picked by
//! rule: it is the player's choice.
//!
//! Units: VRAM in MB, driver versions as four numbers, vendor ids as PCI ids.
//! Determinism: a pure function of `GpuFacts` over a constant table; no clock, no float, no
//! tick. The tier is presentation and never enters a hash.
//! Caller: the adapter pick (GS6) reads `decide` after it chooses a GPU; its VRAM comes from
//! the OS query (GS13).

use crate::adapter::AdapterKind;
use crate::quality::Quality;

/// What the adapter pick knows about the chosen GPU.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GpuFacts {
    /// PCI vendor id (`0x10de` NVIDIA, `0x8086` Intel, `0x1002` AMD, `0x1414` Microsoft).
    pub vendor_id: u32,
    /// Discrete, integrated, CPU or other.
    pub kind: AdapterKind,
    /// Dedicated video memory in MB; `None` when the OS query could not say.
    pub vram_mb: Option<u32>,
    /// Driver version as four numbers, zeros when unknown.
    pub driver: [u32; 4],
}

/// Driver version comparison, as in Chromium's `driver_version.op`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// The GPU's driver is older than the version.
    Lt,
    /// Older than or equal to the version.
    Le,
    /// Equal to the version.
    Eq,
    /// Newer than or equal to the version.
    Ge,
    /// Newer than the version.
    Gt,
}

impl Op {
    /// True when `driver` stands in this relation to `version`.
    #[must_use]
    pub fn holds(self, driver: [u32; 4], version: [u32; 4]) -> bool {
        match self {
            Self::Lt => driver < version,
            Self::Le => driver <= version,
            Self::Eq => driver == version,
            Self::Ge => driver >= version,
            Self::Gt => driver > version,
        }
    }
}

/// What a rule (or one of its exceptions) matches. A field left `None` matches every GPU.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cond {
    /// PCI vendor id to match.
    pub vendor_id: Option<u32>,
    /// Device kind to match.
    pub kind: Option<AdapterKind>,
    /// Inclusive VRAM range in MB. A GPU with unknown VRAM never matches a range.
    pub vram_mb: Option<(u32, u32)>,
    /// Driver version test. A GPU that reports no driver (all zeros) is compared as version 0.
    pub driver: Option<(Op, [u32; 4])>,
}

impl Cond {
    /// Matches every GPU.
    pub const ANY: Self = Self {
        vendor_id: None,
        kind: None,
        vram_mb: None,
        driver: None,
    };

    /// True when every named field matches `facts`.
    #[must_use]
    pub fn matches(&self, facts: &GpuFacts) -> bool {
        self.vendor_id.is_none_or(|v| v == facts.vendor_id)
            && self.kind.is_none_or(|k| k == facts.kind)
            && self
                .vram_mb
                .is_none_or(|(lo, hi)| facts.vram_mb.is_some_and(|mb| (lo..=hi).contains(&mb)))
            && self
                .driver
                .is_none_or(|(op, version)| op.holds(facts.driver, version))
    }
}

/// What a matching rule decides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Do not run on this GPU (a CPU adapter draws a slideshow).
    Refuse,
    /// Run at this tier.
    Tier(Quality),
}

/// One row of the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    /// Short name, printed in the startup line so the reason for a tier is findable.
    pub id: &'static str,
    /// What the rule matches.
    pub when: Cond,
    /// A GPU that matches any of these is not covered by this rule.
    pub except: &'static [Cond],
    /// The result.
    pub verdict: Verdict,
    /// Named workarounds the renderer applies on this GPU.
    pub workarounds: &'static [&'static str],
}

/// The 4 GB line between Potato and Comfort for a discrete GPU, in MB.
pub const COMFORT_MIN_VRAM_MB: u32 = 4096;

/// The rule table. First match wins.
pub const RULES: [Rule; 4] = [
    Rule {
        id: "cpu-refuse",
        when: Cond {
            kind: Some(AdapterKind::Cpu),
            ..Cond::ANY
        },
        except: &[],
        verdict: Verdict::Refuse,
        workarounds: &[],
    },
    Rule {
        id: "integrated-potato",
        when: Cond {
            kind: Some(AdapterKind::Integrated),
            ..Cond::ANY
        },
        except: &[],
        verdict: Verdict::Tier(Quality::Potato),
        // Shares the name of the `E2040_LIMITS=downlevel` switch (GS9).
        workarounds: &["downlevel_limits"],
    },
    Rule {
        id: "discrete-small-potato",
        when: Cond {
            kind: Some(AdapterKind::Discrete),
            vram_mb: Some((0, COMFORT_MIN_VRAM_MB - 1)),
            ..Cond::ANY
        },
        except: &[],
        verdict: Verdict::Tier(Quality::Potato),
        workarounds: &[],
    },
    Rule {
        id: "discrete-comfort",
        when: Cond {
            kind: Some(AdapterKind::Discrete),
            vram_mb: Some((COMFORT_MIN_VRAM_MB, u32::MAX)),
            ..Cond::ANY
        },
        except: &[],
        verdict: Verdict::Tier(Quality::Comfort),
        workarounds: &[],
    },
];

/// The answer for a GPU no rule names: the weakest tier, because an unknown GPU is not trusted.
pub const FALLBACK: Rule = Rule {
    id: "unknown-potato",
    when: Cond::ANY,
    except: &[],
    verdict: Verdict::Tier(Quality::Potato),
    workarounds: &[],
};

/// The first rule of `rules` that matches `facts` and none of its exceptions.
#[must_use]
pub fn first_match<'a>(rules: &'a [Rule], facts: &GpuFacts) -> Option<&'a Rule> {
    rules
        .iter()
        .find(|r| r.when.matches(facts) && !r.except.iter().any(|e| e.matches(facts)))
}

/// The rule that decides for `facts`: the first match in [`RULES`], else [`FALLBACK`].
#[must_use]
pub fn decide(facts: &GpuFacts) -> &'static Rule {
    first_match(&RULES, facts).unwrap_or(&FALLBACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVIDIA: u32 = 0x10de;
    const INTEL: u32 = 0x8086;
    const MICROSOFT: u32 = 0x1414;

    fn facts(vendor_id: u32, kind: AdapterKind, vram_mb: Option<u32>) -> GpuFacts {
        GpuFacts {
            vendor_id,
            kind,
            vram_mb,
            driver: [0; 4],
        }
    }

    fn tier(f: &GpuFacts) -> Verdict {
        decide(f).verdict
    }

    #[test]
    fn tier_rules_name_the_reference_gpus() {
        let uhd = facts(INTEL, AdapterKind::Integrated, Some(128));
        let rtx3050 = facts(NVIDIA, AdapterKind::Discrete, Some(6144));
        let gtx1050 = facts(NVIDIA, AdapterKind::Discrete, Some(2048));
        let unknown_8gb = facts(0x1234, AdapterKind::Discrete, Some(8192));
        let warp = facts(MICROSOFT, AdapterKind::Cpu, Some(0));
        assert_eq!(tier(&uhd), Verdict::Tier(Quality::Potato));
        assert_eq!(tier(&rtx3050), Verdict::Tier(Quality::Comfort));
        assert_eq!(tier(&gtx1050), Verdict::Tier(Quality::Potato));
        assert_eq!(tier(&unknown_8gb), Verdict::Tier(Quality::Comfort));
        assert_eq!(tier(&warp), Verdict::Refuse);
        assert_eq!(decide(&uhd).workarounds, ["downlevel_limits"]);
    }

    #[test]
    fn tier_rules_vram_line_is_four_gb_and_unknown_is_not_trusted() {
        let at = |mb| facts(NVIDIA, AdapterKind::Discrete, mb);
        assert_eq!(tier(&at(Some(4095))), Verdict::Tier(Quality::Potato));
        assert_eq!(tier(&at(Some(4096))), Verdict::Tier(Quality::Comfort));
        assert_eq!(decide(&at(None)).id, "unknown-potato");
        assert_eq!(
            tier(&facts(0, AdapterKind::Other, Some(16384))),
            Verdict::Tier(Quality::Potato)
        );
    }

    #[test]
    fn tier_rules_same_facts_give_the_same_rule_twice() {
        for f in [
            facts(INTEL, AdapterKind::Integrated, Some(128)),
            facts(NVIDIA, AdapterKind::Discrete, Some(6144)),
            facts(MICROSOFT, AdapterKind::Cpu, None),
        ] {
            assert_eq!(decide(&f), decide(&f));
        }
    }

    #[test]
    fn tier_rules_every_default_rule_has_a_fact_that_reaches_it() {
        let probes = [
            facts(MICROSOFT, AdapterKind::Cpu, None),
            facts(INTEL, AdapterKind::Integrated, None),
            facts(NVIDIA, AdapterKind::Discrete, Some(1)),
            facts(NVIDIA, AdapterKind::Discrete, Some(COMFORT_MIN_VRAM_MB)),
        ];
        for (rule, probe) in RULES.iter().zip(&probes) {
            assert_eq!(decide(probe).id, rule.id);
        }
    }

    #[test]
    fn tier_rules_driver_op_and_exception_steer_a_custom_table() {
        const TABLE: [Rule; 2] = [
            Rule {
                id: "nvidia-old-driver",
                when: Cond {
                    vendor_id: Some(NVIDIA),
                    driver: Some((Op::Lt, [31, 0, 15, 4000])),
                    ..Cond::ANY
                },
                except: &[Cond {
                    vram_mb: Some((8192, u32::MAX)),
                    ..Cond::ANY
                }],
                verdict: Verdict::Tier(Quality::Potato),
                workarounds: &["old_driver_blit"],
            },
            Rule {
                id: "rest",
                when: Cond::ANY,
                except: &[],
                verdict: Verdict::Tier(Quality::High),
                workarounds: &[],
            },
        ];
        let mut f = facts(NVIDIA, AdapterKind::Discrete, Some(6144));
        f.driver = [31, 0, 15, 3999];
        assert_eq!(
            first_match(&TABLE, &f).map(|r| r.id),
            Some("nvidia-old-driver")
        );
        f.driver = [31, 0, 15, 4000];
        assert_eq!(first_match(&TABLE, &f).map(|r| r.id), Some("rest"));
        f.driver = [31, 0, 15, 3999];
        f.vram_mb = Some(8192);
        assert_eq!(first_match(&TABLE, &f).map(|r| r.id), Some("rest"));
        assert!(Op::Eq.holds([1, 2, 3, 4], [1, 2, 3, 4]));
        assert!(Op::Le.holds([1, 2, 3, 4], [1, 2, 3, 4]));
        assert!(Op::Ge.holds([1, 2, 3, 4], [1, 2, 3, 4]));
        assert!(!Op::Gt.holds([1, 2, 3, 4], [1, 2, 3, 4]));
        assert!(Op::Gt.holds([2, 0, 0, 0], [1, 9, 9, 9]));
    }
}
