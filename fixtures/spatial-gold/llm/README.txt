SPR4 spatial-llm cassettes. One <model-slug>.tsv per recorded model.
Format: crates/playtest/src/spatial_eval/llm.rs (v1 header, then one row per arm and question).
Record with a live agent loop (docs/plan/spatial-llm-ab.md); `cargo test -p engine-playtest --lib spatial_llm_replay` replays every .tsv here offline.
