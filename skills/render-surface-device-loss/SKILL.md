---
name: render-surface-device-loss
description: "Use when changing crates/render/src/present surface, resize, minimize or device-loss code (Outdated, Lost, zero size, mark_device_lost): gives the recovery map and the tests that prove it."
---

# Surface resize and device loss in render present

The presenter keeps the last good pipeline and targets through resize, minimize, surface loss and device loss, then recovers.
The one danger: a GPU test with no adapter returns early and passes, so a green run can test nothing.

## Rules

1. Zero size suspends and never configures the surface. Do: `crates/render/src/present/surface.rs::SizeClass::classify`, then `crates/render/src/present/gpu_present/presenter.rs::Presenter::resize`, which calls `SwapState::recreate` in `crates/render/src/v1.rs`.
2. Map each acquire outcome to one action. Outdated and Lost reconfigure and return an error for a retry. Timeout and Occluded skip the frame. Validation keeps last-good and does not present. Do: `crates/render/src/present/gpu_present/submit.rs::Presenter::render_surface` and `crates/render/src/present/surface.rs::surface_action`.
3. Reconfigure with the stored last-good config, never a new size. Do: `crates/render/src/present/gpu_present/submit.rs::Presenter::reconfigure_surface`.
4. Device loss is a flag the caller must set. Frames then fail with `DeviceLost`. Do: `crates/render/src/present/gpu_present/presenter.rs::Presenter::mark_device_lost` and `Presenter::check_device`, then `crates/render/src/heal.rs::RenderHeal::report_device_lost` for backoff. Grep finds no wgpu device-lost callback in `crates/render/src`, so nothing sets the flag by itself.
5. Map each error once. `PresentError::recovery` says what the caller does, and `PresentError::fault_code` says what the supervisor reports. Do: `crates/render/src/present/surface.rs::PresentError::recovery`, checked by `crates/render/src/present/tests_unit.rs::every_error_names_its_recovery`.
6. Call the staging belt's reset on every resize, surface loss and device loss. Do: `crates/render/src/uploads.rs::StagingBelt::invalidate_all`. Grep finds no caller outside its own test, so this hook is not wired yet.

## Check
- `cargo test -p engine-render --lib every_error_names_its_recovery`: no GPU needed, `test result: ok`.
- `cargo test -p engine-render --lib surface_events_map_to_recovery_actions`: ok.
- `cargo test -p engine-render --lib size_class_covers_minimize_and_reject`: ok.
- `cargo test -p engine-render --lib verified_recovery_restores_full_quality_and_resets`: ok.
- `$env:E2040_REQUIRE_GPU = "1"; cargo test --locked -p engine-render --all-features --lib resize_and_minimize_survive_with_last_good`: ok. Repeat with `device_loss_fails_loudly_until_rebuilt`. With the variable set, a missing adapter fails instead of skipping (`crates/render/src/test_gpu.rs::require_or_skip`).
- `node tools/proof.mjs`: the engine-render GPU skip count it prints must be 0 on a GPU machine.

## Don't
- Don't configure the surface or the targets at zero size. Zero area keeps the last good size.
- Don't drop the pipeline or the targets on Outdated, Lost or Validation.
- Don't count a skipped GPU test as green. Run it with `E2040_REQUIRE_GPU=1`.
