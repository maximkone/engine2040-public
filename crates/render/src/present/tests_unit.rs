//! tests_unit: headless presenter tests (no `window` needed).
// ---------------------------------------------------------------------------
// Headless tests: errors, keys, cache policy, sizes, surface mapping. These
// run on the default (std-only) build; GPU tests live behind `window` below.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod headless_tests {
    use super::super::*;

    #[test]
    fn mip_chain_box_filters_down_to_one_texel() {
        // 4x2 checker of black and white columns: every level averages
        // to mid gray, sizes halve (odd edges clamp) down to 1x1.
        let mut rgba = Vec::new();
        for _row in 0..2 {
            for col in 0..4u8 {
                let v = if col % 2 == 0 { 0 } else { 254 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let upload = TextureUpload {
            width: 4,
            height: 2,
            rgba,
        };
        let chain = upload.mip_chain();
        let sizes: Vec<(u32, u32)> = chain.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, vec![(2, 1), (1, 1)]);
        for (_, _, level) in &chain {
            assert!(
                level.chunks(4).all(|px| px == [127, 127, 127, 255]),
                "{level:?}"
            );
        }
        let single = TextureUpload {
            width: 1,
            height: 1,
            rgba: vec![9, 9, 9, 9],
        };
        assert!(single.mip_chain().is_empty());
    }

    #[test]
    fn every_error_names_its_recovery() {
        let cases: Vec<(PresentError, RecoveryAction)> = vec![
            (PresentError::NoAdapter, RecoveryAction::KeepLastGood),
            (
                PresentError::Device("x".into()),
                RecoveryAction::ReacquireDevice,
            ),
            (
                PresentError::DeviceLost("x".into()),
                RecoveryAction::ReacquireDevice,
            ),
            (
                PresentError::SurfaceLost { generation: 3 },
                RecoveryAction::ReconfigureSurface,
            ),
            (
                PresentError::SurfaceOutdated { generation: 3 },
                RecoveryAction::ReconfigureSurface,
            ),
            (PresentError::SurfaceTimeout, RecoveryAction::SkipFrame),
            (PresentError::SurfaceOccluded, RecoveryAction::KeepLastGood),
            (
                PresentError::OutOfMemory { generation: 3 },
                RecoveryAction::KeepLastGood,
            ),
            (
                PresentError::Validation("x".into()),
                RecoveryAction::KeepLastGood,
            ),
            (
                PresentError::Readback("x".into()),
                RecoveryAction::RetryFrame,
            ),
            (
                PresentError::InvalidSize {
                    width: 9999,
                    height: 1,
                },
                RecoveryAction::FixCallerInput,
            ),
            (
                PresentError::InvalidDraw {
                    index: 0,
                    reason: "x".into(),
                },
                RecoveryAction::FixCallerInput,
            ),
            (
                PresentError::InvalidMesh("x".into()),
                RecoveryAction::FixCallerInput,
            ),
            (
                PresentError::InvalidTexture("x".into()),
                RecoveryAction::FixCallerInput,
            ),
            (PresentError::NoSurface, RecoveryAction::FixCallerInput),
            (
                PresentError::BadDigest("x".into()),
                RecoveryAction::FixCallerInput,
            ),
        ];
        for (err, recovery) in cases {
            assert_eq!(err.recovery(), recovery, "recovery for {err}");
            assert!(!format!("{err}").is_empty(), "display for {err:?}");
        }
    }

    #[test]
    fn digest_hex_roundtrip_and_rejects() {
        let digest = [0xABu8; 32];
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let key = MeshKey::from_hex(7, &hex).expect("valid hex parses");
        assert_eq!((key.id, key.digest), (7, digest));
        let upper = hex.to_uppercase();
        assert_eq!(
            TextureKey::from_hex(9, &upper)
                .expect("uppercase hex parses")
                .digest,
            digest
        );
        for bad in [
            "",
            "ab",
            &"zz".repeat(32),
            &"ab".repeat(31),
            &"ab".repeat(33),
        ] {
            assert!(
                MeshKey::from_hex(1, bad).is_err(),
                "bad hex refused: {bad:?}"
            );
        }
    }

    #[test]
    fn cache_evicts_oldest_first_and_reinsert_keeps() {
        let mut cache: CpuCache<u64, &str> = CpuCache::new(2);
        assert!(cache.is_empty());
        assert_eq!(cache.insert(1, "a"), None);
        assert_eq!(cache.insert(2, "b"), None);
        assert_eq!(cache.len(), 2);
        // Reinserting a live key replaces without evicting.
        assert_eq!(cache.insert(1, "a2"), None);
        assert_eq!(cache.get(&1), Some(&"a2"));
        // Oldest insert (key 1) still evicts first: FIFO, not LRU.
        assert_eq!(cache.insert(3, "c"), Some(1));
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.sorted_keys(), vec![2, 3]);
        assert!(cache.evict(&2));
        assert!(!cache.evict(&2));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn zero_capacity_cache_keeps_nothing() {
        let mut cache: CpuCache<u64, u8> = CpuCache::new(0);
        assert_eq!(cache.insert(1, 9), Some(1));
        assert!(cache.is_empty());
        assert_eq!((cache.inserted(), cache.evicted()), (1, 1));
        assert_eq!(cache.last_evicted(), Some(&1));
    }

    #[test]
    fn cache_budget_counters_track_high_water_and_name_evicted() {
        let mut cache: CpuCache<u64, &str> = CpuCache::new(2);
        assert_eq!(cache.capacity(), 2);
        assert_eq!(cache.insert(1, "a"), None);
        assert_eq!(cache.insert(2, "b"), None);
        assert_eq!((cache.len(), cache.high_water()), (2, 2));
        assert_eq!((cache.inserted(), cache.evicted()), (2, 0));
        assert_eq!(cache.last_evicted(), None);
        // Reinserting a live key replaces without counting or evicting.
        assert_eq!(cache.insert(1, "a2"), None);
        assert_eq!((cache.inserted(), cache.evicted()), (2, 0));
        assert_eq!(cache.high_water(), 2);
        // Capacity eviction names the oldest id.
        assert_eq!(cache.insert(3, "c"), Some(1));
        assert_eq!((cache.inserted(), cache.evicted()), (3, 1));
        assert_eq!(cache.last_evicted(), Some(&1));
        assert_eq!(cache.high_water(), 2);
        // Manual drops count toward the same telemetry.
        assert!(cache.evict(&2));
        assert_eq!(cache.evicted(), 2);
        assert_eq!(cache.last_evicted(), Some(&2));
        assert!(!cache.evict(&2));
        assert_eq!(cache.evicted(), 2);
    }

    #[test]
    fn overflow_fixture_at_cache_bounds_evicts_without_error() {
        // Mesh-scale overflow: MAX_CACHED_MESHES + 1 inserts evict exactly
        // the oldest id, keep the bound, and name the evicted asset.
        let mut meshes: CpuCache<MeshKey, u8> = CpuCache::new(MAX_CACHED_MESHES);
        let mut evicted: Option<MeshKey> = None;
        for id in 0..=(MAX_CACHED_MESHES as u64) {
            let key = MeshKey::new(id, [0xABu8; 32]);
            if let Some(out) = meshes.insert(key, 0) {
                assert!(evicted.is_none(), "exactly one eviction expected");
                evicted = Some(out);
            }
        }
        let first = evicted.expect("overflow must evict");
        assert_eq!(first.id, 0, "insertion order evicts the oldest first");
        assert_eq!(meshes.len(), MAX_CACHED_MESHES);
        assert_eq!(meshes.high_water(), MAX_CACHED_MESHES);
        assert_eq!(meshes.evicted(), 1);
        assert_eq!(meshes.last_evicted().map(|k| k.id), Some(0));
        // Texture-scale overflow proves the same bound on the other cache.
        let mut textures: CpuCache<TextureKey, u8> = CpuCache::new(MAX_CACHED_TEXTURES);
        let mut evicted_tex: Option<TextureKey> = None;
        for id in 0..=(MAX_CACHED_TEXTURES as u64) {
            let key = TextureKey::new(id, [0xCDu8; 32]);
            if let Some(out) = textures.insert(key, 0) {
                assert!(evicted_tex.is_none(), "exactly one eviction expected");
                evicted_tex = Some(out);
            }
        }
        assert_eq!(
            evicted_tex.map(|k| k.id),
            Some(0),
            "texture eviction names the oldest asset id"
        );
        assert_eq!(textures.len(), MAX_CACHED_TEXTURES);
        assert_eq!(textures.high_water(), MAX_CACHED_TEXTURES);
        assert_eq!(textures.evicted(), 1);
    }

    #[test]
    fn missing_texture_refuses_loudly_with_its_asset_id() {
        use engine_observe::EventCode;
        let key = TextureKey::new(42, [0xEFu8; 32]);
        let resident = vec![TextureKey::new(7, [0x11u8; 32])];
        assert!(check_texture_resident(&resident, &resident[0]).is_ok());
        match check_texture_resident(&resident, &key) {
            Err(PresentError::MissingTexture { id, reason }) => {
                assert_eq!(id, 42);
                assert!(reason.contains("42"), "reason names the id: {reason}");
            }
            other => panic!("absent texture must refuse loudly, got {other:?}"),
        }
        let err = missing_texture_error(42);
        assert_eq!(err.recovery(), RecoveryAction::FixCallerInput);
        assert_eq!(err.fault_code(), Some(EventCode::ASSETS_MISSING));
        assert!(format!("{err}").contains("42"));
    }

    #[test]
    fn size_class_covers_minimize_and_reject() {
        assert_eq!(SizeClass::classify(0, 0), SizeClass::Suspend);
        assert_eq!(SizeClass::classify(0, 600), SizeClass::Suspend);
        assert_eq!(SizeClass::classify(800, 0), SizeClass::Suspend);
        assert_eq!(SizeClass::classify(1, 1), SizeClass::Sizes);
        assert_eq!(
            SizeClass::classify(MAX_FRAME_DIM, MAX_FRAME_DIM),
            SizeClass::Sizes
        );
        assert_eq!(SizeClass::classify(MAX_FRAME_DIM + 1, 1), SizeClass::Reject);
        assert_eq!(SizeClass::classify(1, MAX_FRAME_DIM + 1), SizeClass::Reject);
    }

    #[test]
    fn texture_upload_validation_catches_bad_inputs() {
        let ok = TextureUpload {
            width: 2,
            height: 2,
            rgba: vec![128u8; 16],
        };
        ok.validate().expect("2x2 RGBA8 is valid");
        for bad in [
            TextureUpload {
                width: 0,
                height: 2,
                rgba: vec![],
            },
            TextureUpload {
                width: 2,
                height: 2,
                rgba: vec![0u8; 15],
            },
            TextureUpload {
                width: 2,
                height: 2,
                rgba: vec![0u8; 17],
            },
            TextureUpload {
                width: MAX_TEXTURE_DIM + 1,
                height: 1,
                rgba: vec![],
            },
        ] {
            assert!(
                matches!(bad.validate(), Err(PresentError::InvalidTexture(_))),
                "refused: {}x{} len {}",
                bad.width,
                bad.height,
                bad.rgba.len()
            );
        }
    }

    #[test]
    fn surface_events_map_to_recovery_actions() {
        let cases = vec![
            (SurfaceEvent::AcquiredClean, SurfaceAction::Render),
            (
                SurfaceEvent::AcquiredSuboptimal,
                SurfaceAction::RenderThenReconfigure,
            ),
            (SurfaceEvent::Outdated, SurfaceAction::ReconfigureThenRetry),
            (SurfaceEvent::Lost, SurfaceAction::ReconfigureThenRetry),
            (SurfaceEvent::Timeout, SurfaceAction::SkipRetryNext),
            (SurfaceEvent::Occluded, SurfaceAction::SkipKeepLastGood),
            (
                SurfaceEvent::ValidationError,
                SurfaceAction::AbortKeepLastGood,
            ),
        ];
        for (event, action) in cases {
            assert_eq!(surface_action(event), action, "action for {event:?}");
        }
    }

    #[test]
    fn fault_codes_route_to_supervisor_rows() {
        use engine_observe::EventCode;
        // Faults route to their catalog rows.
        assert_eq!(
            surface_fault_code(SurfaceEvent::Lost),
            Some(EventCode::RENDER_DEVICE_LOST)
        );
        assert_eq!(
            surface_fault_code(SurfaceEvent::Outdated),
            Some(EventCode::RENDER_PIPELINE_FAILED)
        );
        assert_eq!(
            surface_fault_code(SurfaceEvent::ValidationError),
            Some(EventCode::RENDER_PIPELINE_FAILED)
        );
        assert_eq!(
            PresentError::DeviceLost("gone".into()).fault_code(),
            Some(EventCode::RENDER_DEVICE_LOST)
        );
        assert_eq!(
            PresentError::SurfaceLost { generation: 2 }.fault_code(),
            Some(EventCode::RENDER_DEVICE_LOST)
        );
        assert_eq!(
            PresentError::SurfaceOutdated { generation: 2 }.fault_code(),
            Some(EventCode::RENDER_PIPELINE_FAILED)
        );
        assert_eq!(
            PresentError::OutOfMemory { generation: 2 }.fault_code(),
            Some(EventCode::RENDER_FRAME_OVER_BUDGET)
        );
        // Non-faults consume no supervisor budget.
        for event in [
            SurfaceEvent::AcquiredClean,
            SurfaceEvent::AcquiredSuboptimal,
            SurfaceEvent::Timeout,
            SurfaceEvent::Occluded,
        ] {
            assert_eq!(surface_fault_code(event), None, "no fault for {event:?}");
        }
        for err in [
            PresentError::NoAdapter,
            PresentError::SurfaceTimeout,
            PresentError::SurfaceOccluded,
            PresentError::Readback("x".into()),
            PresentError::InvalidSize {
                width: 9999,
                height: 1,
            },
            PresentError::NoSurface,
            PresentError::BadDigest("x".into()),
        ] {
            assert_eq!(err.fault_code(), None, "no fault for {err}");
        }
    }

    #[test]
    fn opt19_load_warms_then_walk_hits_under_bar() {
        use crate::cache::{CacheError, PipelineCache};
        // Scene keys: the four shipped forward-PBR variants, stable across
        // runs for one shader identity.
        let keys = scene_variant_keys();
        assert_eq!(keys.len(), 4);
        assert_eq!(
            keys,
            scene_variant_keys_for(OPT19_SHADER_BYTES, OPT19_PIPELINE_LAYOUT)
        );
        // A different shader identity splits the set, so the window build
        // warming the real forward bytes gets its own warm keys.
        let other = scene_variant_keys_for(b"other-shader", OPT19_PIPELINE_LAYOUT);
        assert_eq!(other.len(), 4);
        assert_ne!(keys, other);
        fn compile(key: &crate::cache::PipelineKey) -> Result<Vec<u8>, CacheError> {
            Ok(vec![key.shader_hash.to_le_bytes()[0]; 8])
        }
        // Cold: the walk would create every pipeline and miss the bar.
        let cold = PipelineCache::new();
        assert!(!cold.walk_ready(&keys));
        let cold_walk = walk_pipelines(&cold, &keys);
        assert_eq!(
            (cold_walk.frames, cold_walk.creations),
            (keys.len(), keys.len())
        );
        assert_eq!(cold_walk.max_over_median, 2.0);
        assert!(!cold_walk.passes());
        // Load: warm every scene variant exactly once.
        let mut cache = PipelineCache::new();
        let first = cache.warm_keys(&keys, compile);
        assert_eq!((first.compiled, first.hits, first.failed), (4, 0, 0));
        assert_eq!(first.total(), keys.len());
        assert!(cache.walk_ready(&keys));
        // Walk: every frame hits, creation ratio 0 under the 1.5x bar.
        let walk = walk_pipelines(&cache, &keys);
        assert_eq!((walk.frames, walk.creations), (keys.len(), 0));
        assert_eq!(walk.max_over_median, 1.0);
        assert!(walk.passes());
        // Warm again: the camera walk compiles zero pipelines.
        let second = cache.warm_keys(&keys, compile);
        assert_eq!((second.compiled, second.hits, second.failed), (0, 4, 0));
        assert!(cache.walk_ready(&keys));
        // Steady state allocates nothing on the walk path.
        #[cfg(not(feature = "window"))]
        {
            let (probe, heap) = engine_core::alloc_probe::measure(|| walk_pipelines(&cache, &keys));
            assert_eq!((probe.creations, heap.calls, heap.bytes), (0, 0, 0));
        }
        // Failure case: a refused variant counts loudly and stays unready.
        let mut failing = PipelineCache::new();
        let bad = failing.warm_keys(&keys[..1], |_| {
            Err(CacheError::Io("backend down".to_string()))
        });
        assert_eq!((bad.failed, failing.diagnostic_count()), (1, 1));
        assert!(!failing.walk_ready(&keys[..1]));
    }
}

#[cfg(feature = "v1")]
#[cfg(test)]
mod draw_tests {
    use super::super::*;

    fn test_list() -> DrawList {
        let scene = crate::v1::default_scene(160, 120).expect("default scene is valid");
        DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: MeshKey::new(1, [0x11; 32]),
                material: scene.material,
                base_color: None,
            }],
        }
    }

    #[test]
    fn valid_list_passes_and_bad_material_names_its_index() {
        test_list().validate().expect("default draw is valid");
        let mut list = test_list();
        list.items.push(DrawItem {
            mesh: MeshKey::new(2, [0x22; 32]),
            material: crate::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 2.0,
                roughness: 0.5,
                exposure: 1.0,
            },
            base_color: None,
        });
        match list.validate() {
            Err(PresentError::InvalidDraw { index, reason }) => {
                assert_eq!(index, 1);
                assert!(
                    reason.contains("metallic"),
                    "reason names the factor: {reason}"
                );
            }
            other => panic!("bad material must fail with its index, got {other:?}"),
        }
    }

    #[test]
    fn empty_list_is_a_clear_only_frame() {
        let mut list = test_list();
        list.items.clear();
        list.validate().expect("empty list clears only");
    }

    #[test]
    fn oversized_list_is_refused() {
        let mut list = test_list();
        list.items = vec![list.items[0].clone(); MAX_DRAWS_PER_FRAME + 1];
        assert!(
            matches!(list.validate(), Err(PresentError::InvalidDraw { .. })),
            "draw count over budget is refused"
        );
    }

    #[test]
    fn frame_dim_matches_v1_capture_dim() {
        assert_eq!(MAX_FRAME_DIM, crate::v1::MAX_CAPTURE_DIM);
    }

    #[test]
    fn textured_draw_validates_like_untextured() {
        // The texture key is render-time residency (like the mesh
        // key), so validation only checks the material either way.
        let mut list = test_list();
        list.items[0].base_color = Some(TextureKey::new(7, [0x77; 32]));
        list.validate().expect("textured draw validates");
        list.items[0].material.metallic = 2.0;
        match list.validate() {
            Err(PresentError::InvalidDraw { index, .. }) => assert_eq!(index, 0),
            other => panic!("bad textured material must fail at draw 0, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// RP6 headless tests: selection follows the controller, the TAA-off path
// compiles at the controller scale, every manifest stamps the choice plus
// adapter/driver/scene digest, and the shimmer A/B posts all three
// variances. GPU submit itself runs on the window build (consumer WIN1).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rp6_tests {
    use super::super::*;
    use crate::{aa_scale, manifest, post};

    const DIGEST: &str = "d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614";

    fn full_rate() -> aa_scale::ScaleController {
        aa_scale::ScaleController::new(aa_scale::ScalePolicy::default()).expect("policy builds")
    }

    fn floored() -> aa_scale::ScaleController {
        let mut ctl = full_rate();
        for _ in 0..40 {
            ctl.update(aa_scale::TARGET_60FPS_MS + 9.0)
                .expect("load runs");
        }
        assert_eq!(
            ctl.scale(),
            ctl.policy().min_scale,
            "sustained load floors the scale"
        );
        ctl
    }

    #[test]
    fn selection_follows_controller_state() {
        // Full rate presents run TAA (the default); any stepped-down
        // scale runs the SMAA fallback.
        assert_eq!(select_aa(&full_rate()), AaChoice::Taa);
        assert_eq!(select_aa(&floored()), AaChoice::SmaaFallback);
        assert_eq!(
            (AaChoice::Taa.name(), AaChoice::SmaaFallback.name()),
            ("taa", "smaa")
        );
    }

    #[test]
    fn taa_off_compiles_at_controller_scale() {
        // Full rate compiles at the present size; the floored controller
        // halves 1080p; both compile SMAA-first with the RP1 chain after.
        let chain = post::PostChain::default();
        let full = full_rate();
        let (compiled, rw, rh) = compile_taa_off_at_controller_scale(&full, 1920, 1080, &chain)
            .expect("full rate compiles");
        assert_eq!((rw, rh), (1920, 1080));
        assert!(
            compiled.order_names_match(&aa_scale::taa_off_pass_names()),
            "taa-off stays smaa-first at full rate"
        );
        let low = floored();
        let (compiled, rw, rh) =
            compile_taa_off_at_controller_scale(&low, 1920, 1080, &chain).expect("floor compiles");
        assert_eq!((rw, rh), (960, 540), "floor halves 1080p");
        assert!(
            compiled.order_names_match(&aa_scale::taa_off_pass_names()),
            "taa-off stays smaa-first at the floor"
        );
        // Failure edge: a zero present size fails loudly, never silently.
        assert!(
            compile_taa_off_at_controller_scale(&low, 0, 1080, &chain).is_err(),
            "zero present size is refused"
        );
    }

    #[test]
    fn manifest_stamps_choice_plus_device_plus_digest() {
        // The floored controller stamps smaa with its scale and render
        // size; the summary keeps the RA8 fields first, the RP6 stamp last.
        let ctl = floored();
        let stamped = AaFrameManifest::new(
            "llvmpipe",
            "23.3",
            1920,
            1080,
            manifest::QualityTier::High,
            DIGEST,
            &ctl,
            960,
            540,
        )
        .expect("manifest builds");
        assert_eq!(stamped.aa, AaChoice::SmaaFallback);
        assert_eq!(stamped.scale, ctl.scale());
        let summary = stamped.summary();
        for want in [
            "adapter=llvmpipe",
            "driver=23.3",
            "resolution=1920x1080",
            &format!("scene_digest={DIGEST}"),
            "aa=smaa",
            "render=960x540",
        ] {
            assert!(summary.contains(want), "summary stamps {want}: {summary}");
        }
        let digest_at = summary.find("scene_digest=").expect("digest in summary");
        let aa_at = summary.find("aa=").expect("aa in summary");
        assert!(
            digest_at < aa_at,
            "RA8 fields stay first, the RP6 stamp last: {summary}"
        );
        // Full rate stamps taa on the same shape.
        let full = full_rate();
        let taa = AaFrameManifest::new(
            "gpu",
            "drv",
            64,
            64,
            manifest::QualityTier::Low,
            DIGEST,
            &full,
            64,
            64,
        )
        .expect("taa manifest builds");
        assert_eq!(taa.aa, AaChoice::Taa);
        assert!(taa.summary().contains("aa=taa"));
        // Failure edges: every gap names its field loudly.
        assert!(
            AaFrameManifest::new(
                "",
                "d",
                64,
                64,
                manifest::QualityTier::Low,
                DIGEST,
                &ctl,
                32,
                32
            )
            .is_err(),
            "empty adapter is refused"
        );
        assert!(
            AaFrameManifest::new(
                "a",
                "d",
                64,
                64,
                manifest::QualityTier::Low,
                "not-hex!!",
                &ctl,
                32,
                32
            )
            .is_err(),
            "non-hex digest is refused"
        );
        assert!(
            AaFrameManifest::new(
                "a",
                "d",
                64,
                64,
                manifest::QualityTier::Low,
                DIGEST,
                &ctl,
                0,
                32
            )
            .is_err(),
            "zero render size is refused"
        );
    }

    #[test]
    fn shimmer_ab_posts_all_three_variances() {
        // Q1 evidence: raw flicker ~0.16, TAA settled under the temporal
        // gate, SMAA settled under its posted gate and well under raw.
        let ab = shimmer_ab().expect("a/b runs");
        eprintln!(
            "RP6 shimmer A/B: raw {:.4}, taa {:.6} (gate {}), smaa {:.6} (gate {})",
            ab.raw,
            ab.taa,
            post::SHIMMER_MAX_VAR,
            ab.smaa,
            aa_scale::SMAA_SHIMMER_MAX_VAR
        );
        assert!(
            (ab.raw - 0.16).abs() < 1e-4,
            "raw shimmer posts ~0.16, got {}",
            ab.raw
        );
        assert!(
            ab.taa <= post::SHIMMER_MAX_VAR,
            "taa settled {} exceeds posted gate {}",
            ab.taa,
            post::SHIMMER_MAX_VAR
        );
        assert!(
            ab.smaa <= aa_scale::SMAA_SHIMMER_MAX_VAR,
            "smaa settled {} exceeds posted gate {}",
            ab.smaa,
            aa_scale::SMAA_SHIMMER_MAX_VAR
        );
        assert!(
            ab.taa < ab.smaa,
            "temporal {} beats spatial {}",
            ab.taa,
            ab.smaa
        );
        assert!(ab.smaa < ab.raw, "smaa {} calms raw {}", ab.smaa, ab.raw);
        let again = shimmer_ab().expect("a/b reruns");
        assert_eq!(ab, again, "a/b variances bit-identical x2");
    }
}

// ---------------------------------------------------------------------------
// RL9 headless tests: feed constants mirror their sources, configs validate
// sizes, params ramp from the capture, the depth decode is exact and
// oriented, the graph wires fog plus contact into forward, and every failure
// is loud. The device submit itself runs on the window build (consumer
// WIN1 through `render_offscreen` / `render_surface`).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rl9_tests {
    use super::super::*;

    fn day_capture() -> crate::sky::SkyCapture {
        crate::sky::bake_sky(&crate::sky::SkyParams::midday(), 16, 8)
            .expect("midday bakes")
            .0
    }

    #[test]
    fn feed_constants_mirror_their_sources() {
        // The RL9 feed reuses the RL7 day defaults and the small-detail
        // kernel; any drift in either source fails here, not silently.
        let day = crate::fog::FogParams::day();
        assert_eq!(
            (
                RL9_FOG_DENSITY_PER_M,
                RL9_FOG_FALLOFF_M,
                RL9_FOG_BASE_M,
                RL9_FOG_MAX_DIST_M,
                RL9_FOG_EXPOSURE_EV100
            ),
            (
                day.density_per_m,
                day.falloff_m,
                day.base_height_m,
                day.max_distance_m,
                day.exposure_ev100
            )
        );
        let small = crate::fog::ContactConfig::small();
        assert_eq!(
            (
                RL9_CONTACT_RADIUS_PX,
                RL9_CONTACT_THICKNESS_M,
                RL9_CONTACT_INTENSITY
            ),
            (small.radius_px, small.thickness_m, small.intensity)
        );
        assert_eq!(FOG_BLIT_UNIFORM_BYTES, 32);
    }

    #[test]
    fn contact_for_frame_sizes_and_fits() {
        let contact = contact_for_frame(64, 64).expect("live size builds");
        assert_eq!(
            (
                contact.width_px,
                contact.height_px,
                contact.radius_px,
                contact.thickness_m,
                contact.intensity
            ),
            (
                64,
                64,
                RL9_CONTACT_RADIUS_PX,
                RL9_CONTACT_THICKNESS_M,
                RL9_CONTACT_INTENSITY
            )
        );
        // Suspended and over-bound frames have no feed.
        assert!(matches!(
            contact_for_frame(0, 0),
            Err(PresentError::InvalidSize { .. })
        ));
        assert!(matches!(
            contact_for_frame(4096, 64),
            Err(PresentError::InvalidSize { .. })
        ));
        // Fit: arena 1080p fits the transient budget, 2048x2048 refuses it,
        // zero area never fits.
        assert!(fog_fits_frame(1920, 1080), "arena fits the fog budget");
        assert!(fog_fits_frame(64, 64));
        assert!(
            !fog_fits_frame(2048, 2048),
            "2048x2048 transients refuse the budget"
        );
        assert!(!fog_fits_frame(0, 64));
        assert!(!fog_fits_frame(4096, 64));
    }

    #[test]
    fn fog_params_ramp_from_the_bound_capture() {
        let capture = day_capture();
        let params = fog_params_for_capture(&capture).expect("day ramps");
        let ring = crate::sky::fog_color(&capture).expect("ring reads");
        assert_eq!(params.fog_color, ring, "fog ramps from the day capture");
        // A ragged capture fails loudly instead of tinting the arena.
        let ragged = crate::sky::SkyCapture {
            width_px: 4,
            height_px: 4,
            texels: vec![[0.1; 3]; 5],
        };
        assert!(
            matches!(
                fog_params_for_capture(&ragged),
                Err(PresentError::InvalidDraw { .. })
            ),
            "ragged capture is refused"
        );
    }

    #[test]
    fn depth_decode_identity_is_exact_and_oriented() {
        // Identity inverse: world equals NDC, eye at the origin. Row 0 is
        // NDC y +1 (top), matching the v1 CPU rasterizer row map.
        let ident = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let raw = vec![0.5f32; 4 * 2];
        let feed = decode_depth_feed(&raw, 4, 2, ident, [0.0, 0.0, 0.0]).expect("decodes");
        assert_eq!((feed.width_px, feed.height_px), (4, 2));
        assert_eq!(feed.texels(), 8);
        // Texel (0,0): ndc (-0.75, +0.5, 0.5); distance is exact Euclid.
        let want = (0.75f64 * 0.75 + 0.5 * 0.5 + 0.5 * 0.5).sqrt() as f32;
        assert!(
            (feed.depth_m[0] - want).abs() < 1e-6,
            "exact distance: {} vs {want}",
            feed.depth_m[0]
        );
        assert!(
            (feed.height_m[0] - 0.5).abs() < 1e-6,
            "top row is +y: {}",
            feed.height_m[0]
        );
        // Orientation: every top-row height exceeds every bottom-row height.
        for x in 0..4 {
            assert!(
                feed.height_m[x] > feed.height_m[4 + x],
                "row 0 sits above row 1 at column {x}"
            );
        }
        // Deterministic bytes twice from the same inputs.
        let again = decode_depth_feed(&raw, 4, 2, ident, [0.0, 0.0, 0.0]).expect("redecodes");
        assert_eq!(feed, again, "one seed decodes bit-identical twice");
    }

    #[test]
    fn depth_decode_rejects_bad_inputs_loudly() {
        let ident = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let ok = vec![0.5f32; 8];
        // Ragged length, out-of-range and non-finite depth all refuse with
        // the readback (retry-frame) error, never a half-decoded feed.
        for bad in [
            vec![0.5f32; 7],
            vec![0.5f32; 9],
            {
                let mut v = ok.clone();
                v[3] = f32::NAN;
                v
            },
            {
                let mut v = ok.clone();
                v[3] = 1.5;
                v
            },
            {
                let mut v = ok.clone();
                v[3] = -0.1;
                v
            },
        ] {
            assert!(
                matches!(
                    decode_depth_feed(&bad, 4, 2, ident, [0.0, 0.0, 0.0]),
                    Err(PresentError::Readback(_))
                ),
                "bad depth refused: len {}",
                bad.len()
            );
        }
        let mut bad_mat = ident;
        bad_mat[2][2] = f32::INFINITY;
        assert!(matches!(
            decode_depth_feed(&ok, 4, 2, bad_mat, [0.0, 0.0, 0.0]),
            Err(PresentError::Readback(_))
        ));
        assert!(matches!(
            decode_depth_feed(&ok, 4, 2, ident, [f32::NAN, 0.0, 0.0]),
            Err(PresentError::Readback(_))
        ));
        // Zero area is a size error, not a readback error.
        assert!(matches!(
            decode_depth_feed(&[], 0, 0, ident, [0.0, 0.0, 0.0]),
            Err(PresentError::InvalidSize { .. })
        ));
    }

    #[test]
    fn depth_decode_flat_plane_normals_face_the_eye() {
        // Flat sheet at z = 0.5, eye behind at z = -2: every normal faces
        // the eye (-z), unit length, finite heights.
        let ident = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let feed = decode_depth_feed(&[0.5f32; 16], 4, 4, ident, [0.0, 0.0, -2.0])
            .expect("flat sheet decodes");
        for (i, n) in feed.normals.iter().enumerate() {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(
                n.iter().all(|c| c.is_finite()) && (len - 1.0).abs() < 1e-5,
                "texel {i} normal is unit: {n:?}"
            );
            assert!(n[2] < -0.99, "texel {i} faces the eye: {n:?}");
            assert!(
                feed.height_m[i].is_finite() && feed.depth_m[i] > 0.0,
                "texel {i} stays finite positive"
            );
        }
        // Counter-case: the same sheet with the eye in front flips to +z.
        let flipped = decode_depth_feed(&[0.5f32; 16], 4, 4, ident, [0.0, 0.0, 3.0])
            .expect("flipped sheet decodes");
        assert!(
            flipped.normals.iter().all(|n| n[2] > 0.99),
            "normals track the eye side"
        );
    }

    #[test]
    fn fog_graph_wires_for_presenter_sizes() {
        // The production call site declares fog -> forward and contact ->
        // forward with handover barriers over the live size.
        let params = fog_params_for_capture(&day_capture()).expect("day ramps");
        let contact = contact_for_frame(16, 16).expect("16x16 builds");
        let compiled = compile_fog_for_presenter(&params, &contact).expect("compiles");
        assert!(
            compiled.order_names_match(&[
                crate::fog::FOG_PASS,
                crate::fog::CONTACT_PASS,
                "forward"
            ]),
            "fog plus contact run before forward"
        );
        let pass_at = |name: &str| {
            compiled
                .order()
                .iter()
                .find(|id| compiled.pass(**id).map(|p| p.name.as_str()) == Some(name))
                .copied()
        };
        let fog_id = pass_at(crate::fog::FOG_PASS).expect("fog pass is wired");
        let contact_id = pass_at(crate::fog::CONTACT_PASS).expect("contact pass is wired");
        let forward_id = pass_at("forward").expect("forward pass is wired");
        for (resource, from) in [
            (crate::fog::FOG_RESOURCE, fog_id),
            (crate::fog::CONTACT_RESOURCE, contact_id),
        ] {
            assert!(
                compiled.barriers().iter().any(|b| {
                    b.resource == resource
                        && b.kind == crate::graph::BarrierKind::Transition
                        && b.from == from
                        && b.to == forward_id
                }),
                "{resource} hands over to forward"
            );
        }
        // Over-budget shapes refuse the plan instead of blowing RA2.
        let huge = crate::fog::ContactConfig {
            width_px: 2048,
            height_px: 2048,
            ..contact
        };
        assert!(
            matches!(
                compile_fog_for_presenter(&params, &huge),
                Err(PresentError::InvalidDraw { .. })
            ),
            "2048x2048 transients must refuse the plan"
        );
    }

    #[test]
    fn fog_err_maps_each_variant() {
        use crate::fog::FogError;
        match fog_err("ctx", FogError::Invalid("bad".into())) {
            PresentError::InvalidDraw { index, reason } => {
                assert_eq!(index, 0);
                assert!(reason.contains("ctx") && reason.contains("bad"));
            }
            other => panic!("invalid maps to InvalidDraw, got {other:?}"),
        }
        for (err, want) in [
            (FogError::NoDevice("d".into()), "rl9 ctx: d"),
            (FogError::Device("d".into()), "rl9 ctx: d"),
        ] {
            match fog_err("ctx", err) {
                PresentError::Device(msg) => assert_eq!(msg, want),
                other => panic!("device maps to Device, got {other:?}"),
            }
        }
        match fog_err("ctx", FogError::Validation("v".into())) {
            PresentError::Validation(msg) => assert!(msg.contains("v")),
            other => panic!("validation maps to Validation, got {other:?}"),
        }
    }

    #[test]
    fn fog_blit_uniform_packs_color_and_size() {
        let bytes = fog_blit_uniform_bytes([0.5, 0.25, 1.0], 64, 32);
        assert_eq!(bytes.len(), FOG_BLIT_UNIFORM_BYTES);
        let words: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(&words[..5], &[0.5, 0.25, 1.0, 64.0, 32.0]);
        assert_eq!(&words[5..], &[0.0, 0.0, 0.0]);
        // Counter-case: a second color packs differently, so the pack is not
        // a constant fixture.
        let other = fog_blit_uniform_bytes([0.0, 0.0, 0.0], 64, 32);
        assert_ne!(bytes, other);
    }
}

// ---------------------------------------------------------------------------
// RL8 headless tests: one bake feeds all three production readers, the
// worker-thread bake matches the direct bake on the plan thread, and every
// failure is loud. The device bind itself runs on the window build
// (consumer WIN1 through `render_offscreen` / `render_surface`).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod rl8_tests {
    use super::super::*;

    fn day_capture() -> crate::sky::SkyCapture {
        crate::sky::bake_sky(&crate::sky::SkyParams::midday(), 16, 8)
            .expect("midday bakes")
            .0
    }

    /// Direction hitting the center of texel (`x`, `y`): inverts the bake
    /// mapping (azimuth `2 pi u`, elevation `pi/2 - pi v` at texel centers).
    fn texel_center_dir(x: u32, y: u32, w: u32, h: u32) -> [f32; 3] {
        let u = (f64::from(x) + 0.5) / f64::from(w);
        let v = (f64::from(y) + 0.5) / f64::from(h);
        let elev = std::f64::consts::FRAC_PI_2 - std::f64::consts::PI * v;
        let (sin_e, cos_e) = elev.sin_cos();
        let phi = 2.0 * std::f64::consts::PI * u;
        [
            (cos_e * phi.cos()) as f32,
            sin_e as f32,
            (cos_e * phi.sin()) as f32,
        ]
    }

    #[test]
    fn one_bake_feeds_all_three_readers() {
        // The RA1 skybox, the RL2 ambient and the RL7 fog all read one bake
        // of the bound day capture, so the three can never drift.
        let capture = day_capture();
        let bake = look_bake_for_capture(&capture).expect("look bakes");
        assert_eq!((bake.width_px, bake.height_px), (16, 8));
        assert_eq!(bake.texels_count(), 16 * 8);
        // Reader 2 (RL2 ambient) matches the direct mean over the capture.
        let direct_mean = capture.mean_radiance().expect("mean reads");
        assert_eq!(
            ibl_ambient_for_capture(&capture).expect("ambient reads"),
            direct_mean,
            "ambient reads the same bake"
        );
        assert_eq!(crate::sky::look_ibl_ambient(&bake), direct_mean);
        // Reader 3 (RL7 fog) matches the direct horizon ring on both paths.
        let direct_fog = crate::sky::fog_color(&capture).expect("ring reads");
        assert_eq!(crate::sky::look_fog_color(&bake), direct_fog);
        let via_capture = fog_params_for_capture(&capture).expect("capture ramps");
        let via_bake = fog_params_for_look_bake(&bake).expect("bake ramps");
        assert_eq!(via_capture, via_bake, "one bake feeds both fog paths");
        assert_eq!(via_bake.fog_color, direct_fog);
        // Reader 1 (RA1 skybox) at a texel center matches the bound texel
        // within the posted parity tolerance.
        let dir = texel_center_dir(5, 3, 16, 8);
        let want = capture.texel(5, 3).expect("texel reads");
        let sampled = skybox_sample_for_capture(&capture, dir).expect("center samples");
        for c in 0..3 {
            assert!(
                (sampled[c] - want[c]).abs() <= crate::sky::SKYBOX_SAMPLE_PARITY_TOL,
                "channel {c} samples the bound texel: {sampled:?} vs {want:?}"
            );
        }
        // Counter-case: a second texel center samples differently, so the
        // reader is not a constant.
        let other = skybox_sample_for_capture(&capture, texel_center_dir(11, 6, 16, 8))
            .expect("second center samples");
        assert_ne!(sampled, other, "distinct texels sample distinctly");
        // Same capture bakes bit-identical bytes twice.
        let again = look_bake_for_capture(&capture).expect("second bakes");
        assert_eq!(bake, again);
        assert_eq!(bake.to_bytes(), again.to_bytes());
    }

    #[test]
    fn worker_bake_matches_direct_on_the_plan_thread() {
        // First-frame-hitch guard: the production bake runs on the plan
        // worker thread and yields the same bake the direct call computes.
        let capture = day_capture();
        let (baked, ms) = bake_look_on_worker(capture.clone()).expect("worker bakes");
        assert!(ms >= 0.0, "bake records pass ms");
        assert_eq!(
            baked.baked_on_thread(),
            crate::sky::RL8_LOOK_BAKE_THREAD,
            "bake runs on the plan worker thread"
        );
        let (direct, _) = crate::sky::bake_look_from_day(&capture).expect("direct bakes");
        assert_eq!(
            baked.to_bytes(),
            direct.to_bytes(),
            "worker and direct bakes agree"
        );
        assert_eq!(
            crate::sky::look_ibl_ambient(&baked),
            crate::sky::look_ibl_ambient(&direct)
        );
        assert_eq!(
            crate::sky::look_fog_color(&baked),
            crate::sky::look_fog_color(&direct)
        );
    }

    #[test]
    fn ragged_captures_fail_all_three_readers_loudly() {
        // A ragged backing is refused by the bake and by every reader built
        // on it, never a half-ramped fog or a silent ambient.
        let ragged = crate::sky::SkyCapture {
            width_px: 4,
            height_px: 4,
            texels: vec![[0.1; 3]; 5],
        };
        assert!(matches!(
            look_bake_for_capture(&ragged),
            Err(PresentError::InvalidDraw { .. })
        ));
        assert!(matches!(
            skybox_sample_for_capture(&ragged, [0.0, 1.0, 0.0]),
            Err(PresentError::InvalidDraw { .. })
        ));
        assert!(matches!(
            ibl_ambient_for_capture(&ragged),
            Err(PresentError::InvalidDraw { .. })
        ));
        assert!(matches!(
            fog_params_for_capture(&ragged),
            Err(PresentError::InvalidDraw { .. })
        ));
        // A non-finite texel poisons the bake, never the frame.
        let mut bad = day_capture();
        bad.texels[0] = [f32::NAN, 0.0, 0.0];
        assert!(look_bake_for_capture(&bad).is_err());
        assert!(ibl_ambient_for_capture(&bad).is_err());
        // Degenerate sample directions fail loudly on a good capture.
        let ok = day_capture();
        assert!(matches!(
            skybox_sample_for_capture(&ok, [0.0, 0.0, 0.0]),
            Err(PresentError::InvalidDraw { .. })
        ));
        assert!(matches!(
            skybox_sample_for_capture(&ok, [f32::NAN, 1.0, 0.0]),
            Err(PresentError::InvalidDraw { .. })
        ));
    }

    #[test]
    fn rl8_err_names_each_sky_variant() {
        use crate::sky::SkyError;
        match rl8_err("ctx", SkyError::Invalid("bad".into())) {
            PresentError::InvalidDraw { index, reason } => {
                assert_eq!(index, 0);
                assert!(reason.contains("ctx") && reason.contains("bad"));
            }
            other => panic!("invalid maps to InvalidDraw, got {other:?}"),
        }
        for (err, want) in [
            (SkyError::NoDevice("d".into()), "rl8 ctx: d"),
            (SkyError::Device("d".into()), "rl8 ctx: d"),
        ] {
            match rl8_err("ctx", err) {
                PresentError::Device(msg) => assert_eq!(msg, want),
                other => panic!("device maps to Device, got {other:?}"),
            }
        }
        match rl8_err("ctx", SkyError::Validation("v".into())) {
            PresentError::Validation(msg) => assert!(msg.contains("v")),
            other => panic!("validation maps to Validation, got {other:?}"),
        }
    }
}
