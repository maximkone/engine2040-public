//! announcer: bout voice lines as a data table with a last-played throttle.
//!
//! Consumer: [`audio_mix`](crate::audio_mix) queues the name a cue earns and
//! plays it on the voice bus. Data owner: this module owns the rule table and
//! one last-play stamp per rule; the clip behind a name comes from a content
//! row, and a missing clip falls back to the cue bank's synth take
//! (`engine_audio::CueBank`). Units: `tick` is the fixed-tick clock (60 Hz),
//! `min_gap_ticks` a minimum gap in ticks. Line names are keys, never paths.
//! Determinism: no clock, no RNG and no heap per cue; the same cue script at
//! the same ticks gives the same lines. Presentation only, never hashed.
//! Errors: none; an unmatched cue kind and a held gap both answer `None`.
//! Budget: `O(rules)` per cue, at most one line per call, and the per-rule
//! stamp vector is allocated once at construction.

use engine_audio::FightCue;

/// Line names: the keys a content row maps to recorded announcer clips.
///
/// They are deliberately distinct from the impact cue names (`hit`, `parry`,
/// `knockdown`) so a missing voice clip never resolves to an impact take.
pub mod lines {
    /// A fighter drops to the floor.
    pub const DOWN: &str = "voice_down";
    /// A block inside the parry window: blade on blade.
    pub const PARRY: &str = "voice_parry";
    /// An attack or dodge pressed without enough stamina.
    pub const TIRED: &str = "voice_tired";
    /// Dodge i-frames negated the hit.
    pub const DODGE: &str = "voice_dodge";
    /// Full damage landed.
    pub const HIT: &str = "voice_hit";
    /// A held block soaked the hit down to chip damage.
    pub const BLOCK: &str = "voice_block";
}

/// Gap between two `down` lines (ticks, 3 s at 60 Hz).
pub const DOWN_GAP_TICKS: u64 = 180;
/// Gap between two `parry` lines (ticks).
pub const PARRY_GAP_TICKS: u64 = 90;
/// Gap between two `tired` lines (ticks).
pub const TIRED_GAP_TICKS: u64 = 240;
/// Gap between two `dodge` lines (ticks).
pub const DODGE_GAP_TICKS: u64 = 120;
/// Gap between two `hit` lines (ticks).
pub const HIT_GAP_TICKS: u64 = 45;
/// Gap between two `block` lines (ticks).
pub const BLOCK_GAP_TICKS: u64 = 120;

/// One announcer rule: answer a cue kind with `line`, no closer than
/// `min_gap_ticks` after this same rule last spoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Cue kind this rule answers. Only the variant matters: the payload
    /// fields (`attacker`, `damage`, ...) are ignored, so one rule covers
    /// every hit. The values below are a placeholder key, not a filter.
    pub cue: FightCue,
    /// Line name returned while the rule speaks; a content-row key.
    pub line: &'static str,
    /// Minimum ticks between two plays of this rule (`0` speaks every call).
    pub min_gap_ticks: u64,
}

impl Rule {
    /// True when `cue` is the same variant as [`Self::cue`] (payload ignored).
    #[must_use]
    pub fn matches(&self, cue: FightCue) -> bool {
        core::mem::discriminant(&self.cue) == core::mem::discriminant(&cue)
    }
}

/// The shipped bout table, most dramatic first: knockdown, parry, stamina
/// out, dodge, hit, block. List order is priority order; the cue kinds do not
/// overlap here, so the order is the shape a bout work adds loud-hit or
/// final-blow rules to.
pub const DEFAULT_RULES: [Rule; 6] = [
    Rule {
        cue: FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        },
        line: lines::DOWN,
        min_gap_ticks: DOWN_GAP_TICKS,
    },
    Rule {
        cue: FightCue::Parried {
            attacker: 0,
            victim: 1,
        },
        line: lines::PARRY,
        min_gap_ticks: PARRY_GAP_TICKS,
    },
    Rule {
        cue: FightCue::StaminaOut { fighter: 0 },
        line: lines::TIRED,
        min_gap_ticks: TIRED_GAP_TICKS,
    },
    Rule {
        cue: FightCue::Dodged {
            attacker: 0,
            victim: 1,
        },
        line: lines::DODGE,
        min_gap_ticks: DODGE_GAP_TICKS,
    },
    Rule {
        cue: FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 0,
        },
        line: lines::HIT,
        min_gap_ticks: HIT_GAP_TICKS,
    },
    Rule {
        cue: FightCue::Blocked {
            attacker: 0,
            victim: 1,
            chip: 0,
        },
        line: lines::BLOCK,
        min_gap_ticks: BLOCK_GAP_TICKS,
    },
];

/// Fixed-tick announcer: a rule table in priority order plus one last-play
/// stamp per rule. Same cue script at the same ticks gives the same lines; a
/// fresh bout builds a fresh announcer.
#[derive(Clone, Debug)]
pub struct Announcer {
    rules: &'static [Rule],
    last_played: Vec<Option<u64>>,
}

impl Announcer {
    /// The shipped table ([`DEFAULT_RULES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_rules(&DEFAULT_RULES)
    }

    /// Announcer over `rules`, first listed first. The table is borrowed for
    /// the whole bout; only the per-rule stamps are allocated, once.
    #[must_use]
    pub fn with_rules(rules: &'static [Rule]) -> Self {
        Self {
            rules,
            last_played: vec![None; rules.len()],
        }
    }

    /// The rule table, in priority order.
    #[must_use]
    pub fn rules(&self) -> &'static [Rule] {
        self.rules
    }

    /// Answer one fight cue at `tick`: the first rule that matches the cue's
    /// kind and whose own gap since its last play has passed. A matching rule
    /// still inside its gap is skipped (a later matching rule may speak), and
    /// a rule that speaks stamps `tick` before its line returns. At most one
    /// line per call; `None` when no rule speaks. A tick behind a stamp counts
    /// as zero gap, so a rewind never re-fires a rule early. Never panics.
    pub fn on_cue(&mut self, tick: u64, cue: FightCue) -> Option<&'static str> {
        for (index, rule) in self.rules.iter().enumerate() {
            if !rule.matches(cue) {
                continue;
            }
            if let Some(at) = self.last_played[index] {
                if tick.saturating_sub(at) < rule.min_gap_ticks {
                    continue;
                }
            }
            self.last_played[index] = Some(tick);
            return Some(rule.line);
        }
        None
    }
}

impl Default for Announcer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(damage: u16) -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage,
        }
    }

    fn parried() -> FightCue {
        FightCue::Parried {
            attacker: 0,
            victim: 1,
        }
    }

    fn down() -> FightCue {
        FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }
    }

    /// Two rules that both answer a hit, first listed first.
    const TWO_HIT_RULES: [Rule; 2] = [
        Rule {
            cue: FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 0,
            },
            line: "voice_first_hit",
            min_gap_ticks: 30,
        },
        Rule {
            cue: FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 0,
            },
            line: "voice_second_hit",
            min_gap_ticks: 30,
        },
    ];

    #[test]
    fn announcer_holds_the_line_inside_its_gap() {
        let mut announcer = Announcer::new();
        assert_eq!(announcer.on_cue(0, hit(40)), Some(lines::HIT));
        let index = announcer
            .rules()
            .iter()
            .position(|rule| rule.line == lines::HIT)
            .expect("the shipped table holds a hit rule");
        assert_eq!(announcer.last_played[index], Some(0));
        // The same cue again inside the gap: one line, then silence.
        assert_eq!(announcer.on_cue(1, hit(40)), None);
        assert_eq!(
            announcer.on_cue(HIT_GAP_TICKS - 1, hit(40)),
            None,
            "the tick before the gap ends still holds"
        );
        assert_eq!(
            announcer.last_played[index],
            Some(0),
            "a held cue moves no stamp"
        );
    }

    #[test]
    fn announcer_speaks_again_after_the_gap() {
        let mut announcer = Announcer::new();
        assert_eq!(announcer.on_cue(0, hit(40)), Some(lines::HIT));
        assert_eq!(
            announcer.on_cue(HIT_GAP_TICKS, hit(40)),
            Some(lines::HIT),
            "the gap's own tick speaks"
        );
        assert_eq!(
            announcer.on_cue(HIT_GAP_TICKS * 2, hit(40)),
            Some(lines::HIT)
        );
    }

    #[test]
    fn announcer_first_listed_matching_rule_wins() {
        let mut announcer = Announcer::with_rules(&TWO_HIT_RULES);
        // The rule key carries damage 0 and the cue damage 80: the payload is
        // ignored, so both rules match and the first listed answers.
        assert_eq!(announcer.on_cue(0, hit(80)), Some("voice_first_hit"));
        // Inside the first rule's gap the scan reaches the second listed rule.
        assert_eq!(announcer.on_cue(5, hit(80)), Some("voice_second_hit"));
        // Both gaps now held: no line.
        assert_eq!(announcer.on_cue(10, hit(80)), None);
        // Past the first rule's own gap it is first listed again.
        assert_eq!(announcer.on_cue(35, hit(80)), Some("voice_first_hit"));
    }

    #[test]
    fn announcer_same_cue_script_same_lines() {
        let script: [(u64, FightCue); 8] = [
            (0, hit(130)),
            (12, parried()),
            (40, hit(20)),
            (95, hit(20)),
            (200, parried()),
            (260, down()),
            (300, hit(60)),
            (
                470,
                FightCue::Dodged {
                    attacker: 0,
                    victim: 1,
                },
            ),
        ];
        let run = || {
            let mut announcer = Announcer::new();
            script
                .iter()
                .map(|&(tick, cue)| announcer.on_cue(tick, cue))
                .collect::<Vec<_>>()
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "the same script speaks the same lines");
        assert!(first.iter().any(Option::is_some), "the script speaks");
        assert!(
            first.iter().any(Option::is_none),
            "the script is also throttled"
        );
    }

    #[test]
    fn announcer_gaps_are_per_rule_not_global() {
        let mut announcer = Announcer::new();
        assert_eq!(announcer.on_cue(0, hit(40)), Some(lines::HIT));
        // A different rule speaks inside the hit gap: the throttle is its own.
        assert_eq!(announcer.on_cue(2, parried()), Some(lines::PARRY));
        assert_eq!(
            announcer.on_cue(4, hit(40)),
            None,
            "the hit gap still holds"
        );
        // Down holds the longest gap.
        assert_eq!(announcer.on_cue(10, down()), Some(lines::DOWN));
        assert_eq!(announcer.on_cue(10 + DOWN_GAP_TICKS - 1, down()), None);
        assert_eq!(
            announcer.on_cue(10 + DOWN_GAP_TICKS, down()),
            Some(lines::DOWN)
        );
    }

    #[test]
    fn announcer_rewind_never_refires_a_rule_early() {
        let mut announcer = Announcer::new();
        assert_eq!(announcer.on_cue(100, hit(40)), Some(lines::HIT));
        // A tick behind the stamp counts as zero gap, not a wrapped huge one.
        assert_eq!(announcer.on_cue(0, hit(40)), None);
        assert_eq!(
            announcer.on_cue(100 + HIT_GAP_TICKS, hit(40)),
            Some(lines::HIT)
        );
    }

    #[test]
    fn announcer_default_table_answers_every_cue_kind_once() {
        let cues = [
            hit(1),
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 1,
            },
            parried(),
            FightCue::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 0 },
            down(),
        ];
        let mut announcer = Announcer::new();
        for cue in cues {
            assert!(announcer.on_cue(0, cue).is_some(), "{cue:?} earns a line");
        }
        // One rule per kind, one kind per rule, and every gap is real.
        assert_eq!(DEFAULT_RULES.len(), cues.len());
        for (index, rule) in DEFAULT_RULES.iter().enumerate() {
            assert!(
                DEFAULT_RULES[..index]
                    .iter()
                    .all(|earlier| !earlier.matches(rule.cue)),
                "{} repeats a cue kind",
                rule.line
            );
            assert!(rule.min_gap_ticks > 0, "{} holds a gap", rule.line);
            assert!(
                rule.line.starts_with("voice_"),
                "{} is a voice key",
                rule.line
            );
        }
        // A cue kind with no rule stays silent.
        let mut unknown = Announcer::with_rules(&DEFAULT_RULES[..1]);
        assert_eq!(unknown.on_cue(0, parried()), None);
    }

    #[test]
    fn announcer_line_names_are_distinct_voice_keys() {
        let names = [
            lines::DOWN,
            lines::PARRY,
            lines::TIRED,
            lines::DODGE,
            lines::HIT,
            lines::BLOCK,
        ];
        for (index, name) in names.iter().enumerate() {
            assert!(name.starts_with("voice_"), "{name} is a voice key");
            assert!(
                names[..index].iter().all(|earlier| earlier != name),
                "{name} is unique"
            );
        }
    }
}
