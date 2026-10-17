//! assets: manifest entry + 7-gate checklist. Refs store id, never path.

/// Content by logical path through a source: files natively, a packed
/// `.e2bundle` in the browser. Std-only.
pub mod asset_source;
pub use asset_source::{AssetSource, BundleSource, FsSource, SourceError};
pub mod budget;
pub mod datanode;
pub use datanode::{parse, DataNodeError, Node};
/// Blender node extras as load-time data, no name hacks.
pub mod extras;
/// Std-only scene/rules schema gate: compile once, refuse with key paths, never panic.
pub mod validate;
pub use validate::{Refusal, Schema};
/// Zero-copy archived bundles over a shared byte mapping: staged
/// payloads finish to one versioned archive; `open` borrows the mapping with
/// no per-entry copy. Std-only; planned, no caller yet (verified 2026-10-16):
/// game and editor instant loads will consume.
pub mod archive;
/// Validated cook envelope borrowed in place: seal offline, wrap once,
/// then serve roots with no decode. Planned, no caller yet (verified
/// 2026-10-16): the mmap load path will consume it.
pub mod archived_cook;
pub use archived_cook::{seal, wrap_archive_entry, ArchivedCook, COOK_MAGIC, COOK_VERSION};
mod wire;
pub use archive::{
    map_file, ArchiveError, ArchiveKind, ArchiveStats, ArchiveWriter, ArchivedBundle,
    ArchivedEntry, ARCHIVE_MAGIC, ARCHIVE_VERSION,
};
#[cfg(all(feature = "import", feature = "registry"))]
pub mod bundle;
#[cfg(feature = "import")]
pub mod cooked;
pub mod cooker;
#[cfg(all(feature = "import", feature = "registry"))]
pub use bundle::{
    load_bundle, BundleEntry, BundleError, BundleLoader, BundleWriter, EntryKind, LoadedBundle,
    BUNDLE_MAGIC, BUNDLE_VERSION, MAX_BUNDLE_BYTES, MAX_ENTRIES, MAX_NAME_CHARS, MAX_PAYLOAD_BYTES,
};
#[cfg(all(feature = "import", feature = "registry"))]
pub mod bundle_cook;
#[cfg(all(feature = "import", feature = "registry"))]
pub use bundle_cook::{
    cook_and_write, cook_bundle_bytes, demo_staged, header_entry_count, is_stub_bundle,
    load_bundle_file, verify_bundle_bytes, write_bundle_file, BundleCookError, BundleReceipt,
    StagedCooked, STUB_BUNDLE_LEN,
};
/// Native BC1/BC3/BC4/BC5/BC7 block codec in-cook: identical bytes x2, encoder
/// decision UNVERIFIED. Std-only.
pub mod bc;
/// Colliders cooked from the art: convex hulls for props and triangle meshes for static
/// kit pieces, written as deterministic bytes the physics lane reads. Hull and bytes are
/// std-only; the glTF `COL_` node reader rides the `import` feature.
pub mod collider_cook;
/// Skinned body plus clip-source import for render skinning (fighter look).
#[cfg(feature = "import")]
pub mod body_import;
/// Combat batch intake: retarget plus digest dedupe over canonical clips.
#[cfg(feature = "import")]
pub mod clip_intake;
/// 80-clip Tier A receipt: counts plus cook-twice plus provenance gate.
#[cfg(feature = "import")]
pub mod clip_receipt;
#[cfg(feature = "import")]
mod collapse;
/// Generated mesh intake: licence gate, repair, prop cook, bundle, provenance row and cache.
#[cfg(all(feature = "import", feature = "registry"))]
pub mod gen_intake;
#[cfg(feature = "import")]
pub mod import_repair;
#[cfg(feature = "import")]
pub mod lod_swap;
#[cfg(feature = "import")]
pub mod mesh_cook;
/// Native glTF-Transform style document pre-pass (dedup, prune, resize-record).
pub mod optimize;
/// Offline BC cook over `bc` plus KTX2 emit: picked shape, x2 identical.
pub mod texture_cook;
/// Offline uv2 chart pack for arena static meshes (cook-time only): chart
/// plus pack plus hashed provenance row; planned, no caller yet (verified
/// 2026-10-16): the bundle cook will consume `pack_uv2`.
pub mod uv2_chart;
pub use uv2_chart::{
    pack_uv2, provenance_for_chart, provenance_row_for_chart, ChartOptions, PackOptions, Uv2Chart,
    Uv2Error, Uv2Options,
};
#[cfg(feature = "import")]
pub mod mesh_import;
pub mod meshlet;
pub mod recook;
#[cfg(feature = "import")]
pub mod retarget;
#[cfg(feature = "import")]
pub mod skin_import;
/// Asset streaming jobs with priorities and budgets: deterministic
/// completion order over bundles. Std-only; planned, no caller yet (verified
/// 2026-10-16): the asset browser will consume.
pub mod stream;
#[cfg(feature = "import")]
pub mod texture;
/// Traversal and finisher intake: edge markers plus sidecar audit over retarget cooks.
#[cfg(feature = "import")]
pub mod traversal_cook;
pub mod vat;
pub mod watcher;
#[cfg(feature = "import")]
pub use cooked::{
    validate_material, validate_mesh, CookError, CookedMaterial, CookedMesh, CookedTexture, Mip,
    TextureEncoding, TextureRef, TextureSlot,
};
#[cfg(feature = "import")]
pub use import_repair::{
    after_digest_of, asset_id, before_digest_of, diagnose, diagnose_with, repair, BudgetSet,
    ImportDiagnosis, ImportSettings, Issue, RepairRefusal, SlotMap, TangentMode,
    CODE_BUDGET_TEXTURE_PX, CODE_BUDGET_TRIS, CODE_BUDGET_UV, CODE_IMPORT, CODE_MESH_MULTIPLE,
    CODE_TANGENTS_MISSING, CODE_TEXTURE_UNDECODABLE, CODE_TOO_LARGE, CODE_UNITS_MISSING,
    MAX_DETAIL_CHARS, MAX_PROBE_BYTES,
};
#[cfg(feature = "import")]
pub use mesh_cook::{
    asset_digest, build_lod_chain, check_budget, cook_digest, cook_gltf_bytes, cook_mesh,
    import_frac_chunks, is_col_name, is_frac_name, is_lod_name, is_socket_name, lod_name,
    lod_target, simplify, split_frac_chunks, tri_count, validate_frac_name, CollisionHull,
    CookedAsset, FracChunk, MeshClass, MeshCookError, MeshCookOutput, COL_PREFIX, FRAC_PREFIX,
    HERO_MAX_TRIS, LOD_RATIOS, MAX_COOK_DETAIL_CHARS, MAX_FRAC_CHUNKS, MAX_FRAC_NAME_CHARS,
    MAX_FRAC_PIECE_CHARS, MAX_HULL_VERTS, MAX_LOD_LEVELS, OPPONENT_MAX_TRIS, PROP_MAX_TRIS,
    SOCKET_PREFIX,
};
#[cfg(feature = "import")]
pub use mesh_import::{
    import_gltf_bytes, import_gltf_path, texture_asset_id, GltfImport, MeshImportError, MAX_BYTES,
    MAX_DIAGNOSTIC_CHARS, MAX_PATH_CHARS, MAX_PRIMITIVES, MAX_VERTICES,
};
#[cfg(feature = "import")]
pub use retarget::{
    is_traversal_edge, parse_sidecar, parse_sidecar_bytes, retarget_clip, retarget_cook,
    retarget_cook_with_sidecar, retarget_digest, validate_bind, BindPose, BoneMap, ClipTags,
    HitWindow, RetargetError, RetargetPlan, RetargetedClip, TraversalMarker, MAX_HIT_WINDOWS,
    MAX_JSON_DEPTH, MAX_SIDECAR_BYTES, MAX_SIDECAR_DETAIL_CHARS, MAX_TAGS, MAX_TAG_CHARS,
    MAX_TRAVERSAL, TRAVERSAL_EDGES,
};
#[cfg(feature = "import")]
pub use skin_import::{
    cook_clip_tables, cook_contact_table, cook_root_table, import_clip, import_clip_doc,
    import_skeleton, import_skeleton_doc, import_skin_doc, import_skin_primitive,
    import_skinned_bytes, skin_content_id, ClipDoc, CookConfig, ImportedClip, JointDoc,
    RotationKey, SkeletonDoc, Skin, SkinDoc, SkinImportError, SkinnedImport, SkinnedPrimitive,
    TranslationKey, MAX_ANIMATIONS, MAX_CLIP_NAME_CHARS, MAX_COOK_TICKS, MAX_INFLUENCES,
    MAX_TRACK_KEYS, QUAT_NORM_EPS, WEIGHT_SUM_EPS,
};
#[cfg(feature = "import")]
pub use texture::{
    cook_texture, texture_content_id, MAX_MIPS, MAX_TEXTURE_BYTES, MAX_TEXTURE_DIM,
    MAX_TEXTURE_PIXELS,
};
pub mod gltf;
/// Generational handles plus load states over the stream queue.
pub mod handle;
pub use handle::{AssetHandle, AssetKey, AssetStore, HandleBudget, LoadState};
pub mod health;
pub mod import;
pub use import::{
    format_of, gate_results, gates_ok, import, validate_ktx2, Format, ImportError, Imported,
    KtxInfo, Sidecar,
};
/// Format-version registry plus fixture template: a declared version
/// without its fixture fails the proof gate by construction. Std-only.
pub mod museum;
/// Release provenance gate: hand-made assets carry URL+SPDX+author+SHA,
/// generated ones carry the AI generator chain; release refuses gaps.
/// Std-only; the release audit consumes.
pub mod provenance;
#[cfg(feature = "registry")]
pub mod registry;
#[cfg(feature = "registry")]
pub use registry::{
    content_hash, sha_hex, AssetId, AssetIdParseError, AssetRecord, AssetRegistry, BatchConflict,
    BatchOp, BatchOutcome, Guid, Handle, HandleError, ManifestRow, Registry, RegistryError,
    Reimport,
};
#[cfg(not(feature = "registry"))]
pub mod registry;
#[cfg(not(feature = "registry"))]
pub use registry::{content_hash, sha_hex, Guid, Handle, HandleError, ManifestRow, Registry};

/// One manifest row: where the asset lives, its digest and its size. It
/// carries no asset bytes, so no content gate can pass from it alone.
pub struct Entry {
    /// Pack-relative asset path.
    pub path: String,
    /// Hex digest recorded for the asset bytes.
    pub sha: String,
    /// Recorded size in bytes.
    pub bytes: u64,
}

/// Content gate names in report order; [`check`] answers one row per name.
pub const GATES: [&str; 7] = [
    "watertight",
    "polyBudget",
    "uvOverlap",
    "pbrSplit",
    "albedoClean",
    "rig",
    "import",
];

/// A manifest entry carries no asset bytes, so no gate can pass here and every
/// gate reports false. Use `import::gate_results(path, bytes)` on real files.
pub fn check(_entry: &Entry) -> Vec<(&'static str, bool)> {
    GATES.iter().map(|g| (*g, false)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_entry() -> Entry {
        Entry {
            path: "pack/a.glb".into(),
            sha: "deadbeef".into(),
            bytes: 128,
        }
    }

    #[test]
    fn manifest_without_bytes_passes_no_content_gate() {
        let results = check(&valid_entry());
        assert_eq!(results.len(), GATES.len());
        for (i, (name, ok)) in results.iter().enumerate() {
            assert_eq!(*name, GATES[i]);
            assert!(!*ok, "gate {name} was never evaluated");
        }
    }

    #[test]
    fn bad_entry_fails_every_gate() {
        for entry in [
            Entry {
                path: String::new(),
                sha: "deadbeef".into(),
                bytes: 128,
            },
            Entry {
                path: "pack/a.glb".into(),
                sha: "deadbeef".into(),
                bytes: 0,
            },
        ] {
            let results = check(&entry);
            assert_eq!(results.len(), 7);
            assert!(results.iter().all(|(_, ok)| !ok));
        }
    }

    #[test]
    fn check_is_deterministic() {
        let e = valid_entry();
        assert_eq!(check(&e), check(&e));
        let bad = Entry {
            path: String::new(),
            sha: String::new(),
            bytes: 0,
        };
        assert_eq!(check(&bad), check(&bad));
    }
}
