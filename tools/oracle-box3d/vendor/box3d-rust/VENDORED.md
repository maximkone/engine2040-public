# Vendored donor: box3d-rust

Source: https://github.com/larsbrubaker/box3d-rust
Commit: `f2ca61e39b653d1b13945d13ebd15478d5fa4661` (HEAD on 2026-10-23)
License: MIT (`LICENSE` copied next to this file).

Vendored for row PX45's determinism spike. The tree is the crate source only:
`src/` and `LICENSE`. What was dropped and why:

| Donor path | Why dropped |
| --- | --- |
| `demo/`, `benches/`, `Tests/`, `box3d-cpp-reference/`, `docs/` | not needed to build the library; keeps the spike small |
| `[[bench]]` targets and `criterion` dev-dependency | the spike never benches the Rust crate; dropping them keeps the build offline |

What was edited in `Cargo.toml`:

- `[workspace] members = ["."]` (the donor listed `demo/wasm`, which is not vendored).
- `[profile.release]` moved to `tools/oracle-box3d/Cargo.toml`, because a path
  dependency's profile section is ignored anyway.
- `exclude` list shortened to the paths that still exist.

No source file under `src/` was modified. Any behavioural diff against the
donor commit is therefore a diff in the scene's own use of the library, not in
the library.
