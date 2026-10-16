---
name: ai-door-rules
description: Use when you change an AI door verb, a model or provider call, an eval or a win-rate claim, an offline search, a small net in the tick, or a dead-reckoned position in crates ai, provider, nn, endpoint or playtest.
version: 0.1.0
author: skillworks
---

# AI door rules

Use when you change an AI door verb, a model or provider call, an eval, an offline search, a small net in the tick, or a dead-reckoned position. The model runs outside the tick. Only recorded actions go in.

1. Tick-stamp each model action and record it before it enters the tick. Replay reads the record, never the model. [dushkin 19.1]
2. Every door verb takes an idempotency key, can be undone, and states its cost before it runs. [dushkin 11.1-11.3]
3. Use three guard layers: input, action and output. A model line must pass the output rail before a player sees it. [dushkin 20.1]
4. No god-agent. Use fixed roles. Add a pattern only with a test and a metric. [dushkin 22.5, 23.2]
5. Do not trust a win rate from a few games. 53 wins in 100 games proves nothing (p = 0.62). Play about 1000 games, swap sides, and run a binomial test. [gogame 10.3.1-10.3.2]
6. Run search and training offline. The tick runs only a small int8 net distilled from that search. [gogame 6.2, 14.2]
7. Quantize every float input to i8 the same way everywhere, before `infer`. Never put a float in hashed state. [crates/nn/src/lib.rs header]
8. Dead reckoning restarts the curve on each packet and keeps velocity continuous (C1). It is render-only and never hashed. [gems2 18]

## Repo facts

- `crates/nn/src/lib.rs` header: weights and activations are integers, the math is integer-only, and there is no wall clock. Rule 7 follows from this.
- `crates/provider` is used by `crates/endpoint` and `crates/playtest` only. The game window does not call it. A library with no caller is not done (ai review).
- `crates/provider/src/pins.rs` pins a request: `RequestPins` with `PINNED_SEED` (2040), `with_system`, `with_temperature_milli` and `with_tools`, and `openai.rs` takes a `RequestPins`. `crates/provider/src/cassette.rs` keys a request (`CassetteKey`, `RequestImage`) and replays the stored answer, so a run replays without the model.

Search more: `node tools/books.mjs search "agent guardrails idempotency"`
