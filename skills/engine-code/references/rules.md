# Rules for every new line

Full checklist behind `SKILL.md` Rules.

- **Reuse first.** `git grep` the behavior before you write it. Owners in
  `crates/core/src/lib.rs`:
  - hashing: `engine_core::{Fnv64, hash_bytes, hash_u64s, mix64, splitmix64}`;
  - rng: `Rng` (`fork`, `fork_named`), `CounterRng`;
  - clock: `time::IntAcc`, `tick` (`MAX_STEPS`);
  - math: `Fixed`, `det_math` (`det_sin`, `det_cos`, `det_exp`), `vec3`, `quat`;
  - conversions: `checked` (saturating and decimal parsers);
  - zero-alloc proofs: `alloc_probe::measure` (dev-dependency feature `test-util`).
  Any crate may depend on `engine-core`.
- **Errors.** Typed enums with `Display`. `?` over `unwrap`. `expect` names the proven
  invariant. Panics only for bugs. Binaries may add context.
- **Numbers.** `From`/`TryFrom` or a named tested conversion, not `as` (see
  `rust-safety`). `total_cmp` for float order. Units in names or newtypes. Never
  `mul_add` in sim code.
- **Hot paths.** No allocation, `format!`, lock or `dyn` per entity per tick. Buffers
  owned by the system and reused. Slices in, slices out. `sort_unstable` for keys.
  Prove with `alloc_probe::measure`.
- **Shape.** Functions under 100 lines, files under 1,500. A params struct over 7
  arguments. No free function that only forwards. `pub(crate)` unless a consumer needs
  it. Docs on every `pub` item.
- **Exceptions.** `#[expect(lint, reason = "...")]`, never `#[allow]`.
  `#[ignore = "<why>"]`. `// det-ok:` and `// alloc-ok:` waivers are reviewed.
- **Comments.** Why, not history. No board ids, sprint numbers or "was X". Git carries
  history.
- **Tests.** Each fails on a plausible defect: oracle, negative case, property over
  seeds. Never restate a constant.

"Ponytail" from the user: shortest diff that works, one check, mark deferred work
`ponytail: <path>`. Never cut validation, determinism or tests.
