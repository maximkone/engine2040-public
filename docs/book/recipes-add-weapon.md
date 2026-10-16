# Recipe: add a weapon

Add a new weapon as data, not as combat code: a versioned move-table
row plus a TPL1 weapon template item plus the staged-fight replay
that proves it. Combat rules stay integer-only on the fixed tick;
gore is presentation-only. Full signatures live in rustdoc:
`cargo doc -p engine-combat --no-deps` and
`cargo doc -p engine-editor --no-deps`.

## 1. Tune the row in `engine-combat` tables

Rows live in `crates/combat/src/fight.rs` (`Tables`, `TABLES_V1`)
and travel versioned through `crates/combat/src/tables.rs`
(`MoveTableSet::new`, `validate_tables`, `SwapInput`). Units:
damage in permille (`0..=1000`), stamina in points
(`0..=MAX_STAMINA`), all timing in ticks at 60 Hz. Version 1 is the
shipped CB1 row; tag 0 replays legacy tapes through V1; every new
set needs a nonzero version and must pass validation (damage within
health, startup within a second, `parry_window < startup`, chip
`num <= den <= 16`, costs within stamina).

Example shape (link, do not copy-paste blind — read the types):

- `MoveTableSet::v1()` is the shipped rules.
- `MoveTableSet::new(version, tables)` plus `validate_tables`
  builds a candidate; only a validated set becomes a `SwapInput`.
- The arena consumes the `SwapInput` at match build and applies the
  swap only at a recorded authority tick.

## 2. Create the weapon item through the TPL1 template

The weapon starter goes through the editor command service, never
around it: preview first, then apply, and it must stay spatially
clean. In code that is `TemplateService::preview` then
`Shell::template_apply(TemplateKind::Weapon, name)` in
`crates/editor/src/templates.rs` and `crates/editor/src/shell.rs`;
over MCP it is `tx.preview` then `tx.apply` (see
[Drive it from MCP](recipes-drive-from-mcp.md)). Taken names refuse
with a suggestion and change nothing.

## 3. Verify headlessly

Validate the tables plus the templates (both verified this wave):

```powershell
cargo test --locked -p engine-combat --lib tables
cargo test --locked -p engine-editor --lib templates
```

Expected: combat `21 passed`, editor `8 passed`. Then replay the
scripted 4-fighter match from the [quick start](quick-start.md)
— same manifest plus seed plus script gives the same fight:

```powershell
cargo run --locked -p engine-game --features app --bin staged_fight -- --ticks 600 --out-dir target/weapon-check
```

Expected: `staged_fight done: ticks=600 ... captures=4` with a
manifest and receipt under `target/weapon-check`. A seeded
overpowered row must trip the tournament spotlight (see the
[verifiers page](verifiers.md)) — that is the balance gate doing
its job, not a green weapon.

Gate notes: this recipe is fully headless. Windowed damage numbers,
muzzle flash, and blood decals submit on a real device (see the
[presentation page](systems-presentation.md)) and are not claimed
here. Board row HR3 owns versioned move tables; TPL1 owns the
weapon template.

## Reference

- `crates/combat/src/fight.rs` — `Tables`, `TABLES_V1`, step
- `crates/combat/src/tables.rs` — `MoveTableSet`, `validate_tables`, `SwapInput`
- `crates/editor/src/templates.rs` — `TemplateKind::Weapon`
- `content/fighters/manifest.json`, `crates/game/src/fighter_bind.rs`
