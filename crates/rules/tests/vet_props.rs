//! AD13: vet never panics on a token stream, and every refusal is a typed
//! `RulesError`. Fixed seed, std only: no property-test dependency.
//!
//! Consumer: the `rules.set` vet path (`rules/src/rules.rs::Rules::set`), which
//! runs compile plus the smoke worlds before anything installs. Determinism:
//! the same seed builds the same stream and the same answer twice, so a
//! failure here reproduces without a model. Units: none (text and codes).
//! Budget: 64 seeds x 32 cases, each a short script.

use engine_rules::{Limits, Rules, RulesError};

/// Fixed-seed xorshift64: deterministic streams with no rand crate.
struct Seeds(u64);

impl Seeds {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// One token from the alphabet, uniform enough for a fuzz stream.
    fn pick<'a>(&mut self, alphabet: &[&'a str]) -> &'a str {
        let len = u64::try_from(alphabet.len()).unwrap_or(1).max(1);
        let at = usize::try_from(self.next() % len).unwrap_or(0);
        alphabet[at.min(alphabet.len() - 1)]
    }
}

/// The token alphabet: legal host calls, legal syntax, and the shapes the vet
/// exists to refuse (floats, strings, unbounded loops, huge integers).
const TOKENS: [&str; 26] = [
    "let ",
    "x",
    " = ",
    "1",
    ";",
    "fn ",
    "on_tick",
    "(",
    ")",
    "{",
    "}",
    "press(1);",
    "walk(100, 0);",
    "loop ",
    "while true ",
    "if x > 0 ",
    "return ",
    "me",
    "\"text\"",
    "1.5",
    "// comment\n",
    "a b",
    "999999999999999999999999",
    "mem_set(0, 1);",
    "damage(1, 10);",
    "\n",
];

/// One script from `count` tokens.
fn script(seed: u64, count: usize) -> String {
    let mut seeds = Seeds(seed | 1);
    let mut text = String::with_capacity(count * 8);
    for _ in 0..count {
        text.push_str(seeds.pick(&TOKENS));
    }
    text
}

/// Accept a rules-crate answer: an install or a typed refusal, never a panic
/// and never empty text.
fn accepted(answer: Result<(), RulesError>, text: &str) {
    if let Err(error) = answer {
        let shown = format!("{error}");
        assert!(!shown.is_empty(), "a refusal names itself: {text:?}");
        assert!(shown.len() < 400, "a refusal text stays bounded: {shown:?}");
    }
}

#[test]
fn vet_props_never_panics_and_every_failure_is_typed() {
    for seed in 0..64u64 {
        for case in 0..32usize {
            let case = u64::try_from(case).unwrap_or(0);
            let text = script(seed.wrapping_mul(1_000_003).wrapping_add(case), 24);
            assert!(text.len() < 64 * 1024, "a stream stays a short script");
            let mut rules = Rules::new(Limits::default());
            let answer = rules.set(&text, 0).map(drop);
            accepted(answer, &text);
            // A refusal installs nothing and is remembered as typed data.
            if rules.script_hash().is_none() {
                assert_eq!(rules.history().len(), 0, "no revision without a script");
            }
        }
    }
    // Same seed twice: the same stream gives the same refusals, twice.
    let assert_bounded = |seed: u64| {
        let mut rules = Rules::new(Limits::default());
        let answer = rules.set(&script(seed, 24), 0).map(drop);
        answer.map_err(|error| format!("{error}"))
    };
    assert_eq!(assert_bounded(11), assert_bounded(11), "a replay drifts");
}

#[test]
fn vet_props_text_limits_refuse_with_the_typed_error() {
    // A source over the byte cap is `TooLarge`, not a panic.
    let cap = Limits::default().max_source_bytes;
    let long = "let x = 1;".repeat(cap / 10 + 1);
    let mut rules = Rules::new(Limits::default());
    let refused = rules.set(&long, 0).expect_err("over the cap refuses");
    assert!(
        matches!(refused, RulesError::TooLarge { .. }),
        "a huge source is TooLarge: {refused:?}"
    );
    // Raw punctuation chaos also refuses inside the type.
    for chaos in ["{{{{", "))))", "fn(", "\n\n\n", "x = = = ;"] {
        let mut rules = Rules::new(Limits::default());
        accepted(rules.set(chaos, 0).map(drop), chaos);
    }
}
