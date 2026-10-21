//! D16: the five scenario prompts.
//!
//! Consumer: an MCP client that asks "what do I do now". The 33 shipped
//! skills are procedures; these five are *situations* the door is asked about
//! most, each answered with the exact door verbs to call, in the order they
//! have to be called.
//!
//! Data owner: this module. It names the door's own verbs and nothing else,
//! so a scenario never drifts into describing a tool the door does not have.
//! Units: chars for the bodies, counts for the prompts. Determinism: the same
//! build always publishes the same five prompts, in contract order; no clock,
//! no network, no float.
//!
//! Shape: a prompt is a name, a one-line description and a body. The body
//! is plain text an agent can follow: the verb to call, what to pass, and the
//! check that says the step landed.

use rmcp::model::Prompt;

/// The five scenario prompts, in contract order.
pub const SCENARIO_NAMES: [&str; 5] = [
    "new-duel",
    "add-a-break",
    "tune-a-hit",
    "fix-a-red-capture",
    "make-the-web-build",
];

/// One scenario prompt: name, one-line description and body.
struct Scenario {
    name: &'static str,
    description: &'static str,
    body: &'static str,
}

/// The scenario table. Every body names only verbs the door serves.
const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "new-duel",
        description: "Build a fresh duel from a brief, then read back what landed.",
        body: "\
# New duel

A new duel is one brief, one build verb, one read back.

1. Read the document so you know which revision you are building on:
   call `scene.read`. It answers the revision and the ids of the objects
   that already exist.
2. Build the duel from the brief. Write the brief in the level grammar
   (`prop` lines name a prop, `want` lines name what the fight must have)
   and call `level.build` with `brief`, a `seed`, the `expected_revision`
   you just read and an `idempotency_key` of your own choosing. The key
   makes a retry safe: the same key twice changes nothing twice.
3. Read what landed. The output carries `revision`, `solved`, the
   `placements` with their live ids, and `verdict` (`pass` or `refused`).
   On a refusal it carries `conflicts` and `findings` instead of
   placements: fix the named line in the brief and build again.
4. Check it plays. Call `play.run` with the same `seed` the build used.
   The answer carries the final digest. Two runs with the same seed give the
   same digest; a different digest means the build is not deterministic and
   the duel is not ready.

The undo token in the build output reverses the whole build in one call
(`tx.undo`). Nothing else is needed to undo it.",
    },
    Scenario {
        name: "add-a-break",
        description: "Add a break to the scene, then see exactly what changed.",
        body: "\
# Add a break

A break is a change to the document, so it goes through the same preview
and apply path as every other edit.

1. Read the document: call `scene.read` for the revision and the live ids.
2. See the change before you make it. Call `tx.preview` with the batch you
   intend (the ops that add the break). The answer carries the diff, the
   findings and a single-use approval bound to this session, this revision
   and the exact bytes of the batch.
3. Apply it with that approval: call `tx.apply` with the same batch and
   the approval token from the preview. The answer carries the new
   revision, the change list and the evidence receipt.
4. Read the diff back. Call `scene.diff` with the revision from step 1 and
   the revision from step 3: it answers only the added, removed and
   changed rows between the two, never the whole scene.

A stale approval (the document moved) is refused, and the refusal names
the revision to re-read. `tx.undo` with the build's undo token reverses
the whole batch in one call.",
    },
    Scenario {
        name: "tune-a-hit",
        description: "Sweep one combat number over a range and read the win-rate curve.",
        body: "\
# Tune a hit

Tuning is a sweep: one knob, a range, a fixed set of seeds, and a curve.

1. Pick the knob and the range. Every combat number has a legal range;
   `settings.get` answers the current value and the range for the knob you
   name. A value outside the range is refused, and the refusal prints the
   range.
2. Run the sweep. Call `play.sweep` with the seed and the number of bouts
   to play. The answer carries the outcome envelope for the sweep, and the
   curve is written to a CSV beside it: one row per bout with the win
   share, the kill share and the stuck count.
3. Read the curve. The win rate is a permille per row, so two runs of the
   same seed give the same curve; a curve that moves between runs means
   the sweep is not seeded and its number is not a number.
4. Land the value you chose, undoably. Set it with `settings.set` (or the
   combat knob's own verb) and keep the undo token: the change lands as
   one undoable step, so a bad number is one `settings.undo` away.

Do not tune two knobs at once: the curve then cannot say which one moved
the win rate.",
    },
    Scenario {
        name: "fix-a-red-capture",
        description: "A capture came out red. Find the rule that refused it and fix that.",
        body: "\
# Fix a red capture

A red capture is a refusal with a reason. The reason names the rule and
the legal value, so the fix is in the message, not in a guess.

1. Read what the capture call refused. The refusal carries a code and a
   message. The code is in the error catalog, which the door serves as
   the resource `e2040://error-catalog`; read it with `resources/read`.
2. Read the scene the capture ran against: call `scene.read`. A capture
   that goes red usually means the object moved, was never placed, or is
   behind another object, and the scene shows which.
3. Ask the spatial questions. Call `scene.query` with the object id and
   the relation you need (`reachable`, `visible`, `on_top_of` and the
   rest). The answer names the object and the rule that decided.
4. Fix the named thing, not the symptom. Change the object the rule
   named, or change the brief that placed it, and go through
   `tx.preview` then `tx.apply` again. A capture that is refused for a
   missing placement is not fixed by re-taking the capture.
5. Prove it. Re-take the capture. It must come back with the pixels of
   the frame, not a refusal.",
    },
    Scenario {
        name: "make-the-web-build",
        description: "Build the browser shell and read the receipt that says it works.",
        body: "\
# Make the web build

The web pack is off by default and stays off until the host allows it.

1. Load the pack: call `api.load` with the pack name `web`. Until the
   pack is loaded its verbs are not advertised, and until the host allows
   the web root the verbs refuse with the reason.
2. Build the browser shell: call `build.web`. The answer carries the
   build directory, the file list and the byte sizes. A build that fails
   prints the last lines of the tool's output and the cause.
3. Serve it: call `serve.web`. The answer names the port and the command
   that serves the same folder.
4. Look at it: call `browser.capture` for a frame and `browser.input`
   for the input. The capture answers a PNG plus the manifest that says
   which build and which engine it came from.
5. Check the numbers. The build receipt carries the size of the bundle
   and the sizes of the parts; a web build that grew past the asset
   budget is refused by the receipt, not by a crash.

The web pack carries its own undo stack, so a build that lands and then
turns out wrong is reversed by the pack's undo, not by the document
journal.",
    },
];

/// The scenario prompts, in contract order.
#[must_use]
pub fn prompts() -> Vec<Prompt> {
    SCENARIOS
        .iter()
        .map(|scenario| {
            let mut prompt = Prompt::new(
                scenario.name.to_owned(),
                Some(scenario.description.to_owned()),
                None,
            );
            prompt.description = Some(scenario.description.to_owned());
            prompt
        })
        .collect()
}

/// One scenario body; `None` for an unknown name.
#[must_use]
pub fn body(name: &str) -> Option<&'static str> {
    SCENARIOS
        .iter()
        .find(|scenario| scenario.name == name)
        .map(|scenario| scenario.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_scenarios_are_published_in_contract_order() {
        let prompts = prompts();
        assert_eq!(prompts.len(), 5, "the door publishes five scenarios");
        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_ref()).collect();
        assert_eq!(names, SCENARIO_NAMES.to_vec());
        for prompt in &prompts {
            assert!(
                prompt.description.is_some(),
                "{} explains itself",
                prompt.name
            );
        }
    }

    #[test]
    fn every_scenario_names_the_verbs_it_uses() {
        for scenario in SCENARIOS {
            assert!(
                !scenario.body.is_empty(),
                "{} carries a body",
                scenario.name
            );
            assert!(
                scenario.body.contains('`'),
                "{} names its verbs in backticks",
                scenario.name
            );
        }
    }

    #[test]
    fn a_scenario_body_is_readable_by_name() {
        assert!(body("new-duel").is_some());
        assert!(body("not-a-scenario").is_none());
    }

    #[test]
    fn the_scenarios_name_only_verbs_the_door_has() {
        // Every verb a scenario calls must be one the door serves. The list
        // is the core tools plus the pack verbs; a scenario that named a
        // verb the door does not have would send an agent down a dead end.
        let mut legal: Vec<String> = crate::mcp::CORE_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        for verb in crate::packs::PackRegistry::shipped().verb_names() {
            legal.push(verb.to_owned());
        }
        for scenario in SCENARIOS {
            for token in scenario.body.split('`') {
                if !token.contains('.') || token.contains(' ') || token.contains('(') {
                    continue;
                }
                let name = token.split_whitespace().next().unwrap_or(token);
                assert!(
                    legal.iter().any(|v| v == name),
                    "{} names the verb `{name}`, which the door does not serve",
                    scenario.name
                );
            }
        }
    }
}
