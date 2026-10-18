# SP20 / SPR4: how to get the real-model spatial number (2026-10-18)

Status: harness and cassette format are done (`crates/playtest/src/spatial_eval/llm.rs`). No model figure exists
yet. The echo run proves plumbing only; its table says `NO MODEL FIGURE`.

## What is measured

50 seeded questions on the three eval rooms (15 left_of, 15 can_see, 10 where_fits, 10 why_failed). Gold comes from
the integer-cm oracle in `world.rs` with the convention stated in the question text (x east, z north). Two arms:
`raw` gets only today's `scene.brief` text, `verb` gets the door tools (`scene.query`, `scene.place`). Score is exact
match on the answer set; an empty answer is wrong. The table prints exact, per family, calls and tokens per arm and a
verdict: `KEEP` when verb beats raw by 20 points or more, otherwise `KILL` (stop adding verbs on this evidence).

## How a real model is run (no paid call was made from here)

1. Pick the model slug (for example `muse-spark-1.3-free`). Write a small recorder that implements `Model`
   (`ask(arm, question) -> Reply`) over `engine-provider`: for the `raw` arm one completion with `prompt(Raw, q)`; for
   the `verb` arm an agent loop with the real door MCP tools (`scene.query`, `scene.place`) staged on the matching
   room, counting tool calls and summing provider usage. The recorder must put `source` = `model`.
2. `record("<slug>", &recorder, &questions())`, then write `Cassette::to_text()` to
   `fixtures/spatial-gold/llm/<slug>.tsv`. Only answers the model gave are recorded (no timeouts or quota errors).
3. `cargo test -p engine-playtest --lib spatial_llm_replay` replays every `.tsv` there offline and prints one table per
   cassette, byte-identical twice. A changed question or brief makes the row stale and the replay refuses it.

Budget guide: 100 calls (50 questions, 2 arms); the raw arm prompt is about 300-600 estimated tokens, the verb arm
about 150 plus tool results. Use the free model first (Forge measured 12.9k-47k tokens per scene task for the same
class of model), then one paid model for the ceiling.

## What to do next

- Land SPR1 (real labels and a position row in the brief) and SPR2 (pin left/right in the door) first. Until then the
  verb arm's `left_of` answer comes from a door whose left/right convention is unverified, so a bad verb score may be
  the door and not the model. After SPR1 regenerate the questions (labels change) and re-record.
- Read the misses before any training talk: wrong id sets mean name grounding (SP67 candidate), wrong yes/no or a wrong
  side means verb text or convention, an empty answer means the loop gave up (budget).
- If `KILL`: stop SPR5-SPR10 verb work and spend on schema text and refusals (AD26) instead. If `KEEP`: the verbs are
  worth the lane; train only if the verb arm stays under 90 percent after the text fixes.
