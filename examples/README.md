# examples — index

Five runnable demos.

| file | run | speed | used in |
|---|---|---|---|
| `quick_start.rs` | `cargo run --example quick_start` | fast | `docs/book/quick-start.md` |
| `headless.rs` | `cargo run --example headless` | fast (120 ticks) | `docs/book/quick-start.md` |
| `steals_proof.rs` | `cargo run --example steals_proof` | fast (asserts x2) | nothing yet |
| `window.rs` | `cargo run --release --example window --features window -- 10000 5` | slow (GPU, 5 s) | `README.md`, `docs/VISUAL-TARGET.md` |
| `ai_builds_arena.rs` | build the door first, then `cargo run --example ai_builds_arena` | slowest (two door passes + PNG) | `docs/FINISH-RICH.md` (P7) |

Slowest first to trim: `ai_builds_arena.rs`.
