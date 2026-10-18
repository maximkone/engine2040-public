# STEAL CARD: peridot (wave 2026-10-18/2, position 32)

Donor: `Pctg-x8/peridot` @ `dev` (default branch), pushed 2026-10-18T15:56:04Z, created years earlier, 11 stars, 58,577 KB. Docs: `docs.ct2.io/peridot/dev/peridot`. Workspace: `base` (Vulkan through its own `bedrock` bindings), `derive`, `modules/{archive, serialization-utils, vertex-processing-pack, math, image, vg, transition, command-object, memory-manager, semantic-shader, rendering-configuration, asset-processing, native-io, sprite-atlas, mesh}` and `cradle/{android, linux, mac, windows}`. README: "For new-generation game/multimedia development with arbitrary programming languages. Written in Rust." Read live 2026-10-18 with `gh api` and `raw.githubusercontent.com`, read-only: no clone, no build, no bytes vendored.
Licence: **MIT**. Root `LICENSE` read live: MIT text, "Copyright 2018 S.Percentage <Syn.Tri.Naga@gmail.com>", no SPDX tag line in the file; `gh api license.spdx_id = MIT`.

## What it is

A long-lived, cross-platform Rust game engine with a hand-written Vulkan layer, its own async asset pipeline, an archive format, vector graphics with SDF generation, and a "cradle" per platform (Android AAudio, Linux Wayland/XCB + PipeWire/PulseAudio, macOS, Windows) so the portable core links into a thin native shell. Game logic is a separate crate the build links in.

## Steals

**1. A fence reactor thread that wakes parked tasks instead of polling** | `base/src/graphics/async_fence_driver.rs:3-5` (`trait AwaitableFence { fn is_ready(&self) -> Result<bool> }`), `:7-10` (`PendingFenceWakingData { waker: Waker, fence: Weak<dyn AwaitableFence + Send + Sync> }` — a weak reference, so a dropped task does not keep the fence alive), `:12-31` (`FenceReactorThread` = one named thread `"Peridot Fence Reactor"` over `Arc<Mutex<Vec<PendingFenceWakingData>>>` + `Arc<Condvar>`, parked when the list is empty and woken when a fence is registered), `:33-45` (the loop pops signalled indexes and wakes those tasks) | MIT | PRESENTATION (GPU wait) | lands on `crates/render/src/accel.rs:1378-1388` (`map_readback` maps and waits, "one poll covers every pending map") and the GI/accel readback paths
Idea: the wait for a GPU result becomes a future the reactor completes, one waker per pending fence, no spin and no poll in the frame. Because the pending entry holds `Weak`, a cancelled task's fence drops with it.
Moves: a readback or a bind probe result resolves off the frame and lands at a tick boundary (the off-thread rule), instead of `encode + submit + readback` all inside one call (`accel.rs:1040`).
Kill: kill if `map_readback` is already off the frame and inside the posted budget — a reactor adds a thread that must be joined, and a poll whose wall time is under budget is cheaper. Require the reactor to pay for itself in a measured frame time.

**2. Transfer batches that merge copies by resource key** | `base/src/batch.rs:12-14` (`trait TransferrableBufferResource { fn grouping_key(&self) -> u64; fn raw_handle(&self) -> VkBuffer }`), `:141-165` (`Eq`/`Hash`/`Ord` over a single `grouping_key()` and over a *pair* `(grouping_key, grouping_key)`, so two resources in the same allocation compare equal for grouping), `:175-219` (`TransferBatch2::{new, has_ops, copy_buffer, register_before_transition}`) | MIT | PRESENTATION (upload path) | lands on the staging belt / streaming pool in `crates/render` (the VRAM/staging rules)
Idea: a frame's small copies coalesce into one submission when they touch the same allocation, and the before-transition is registered once per batch instead of once per copy. The key is the allocation, not the buffer.
Moves: submissions per frame and transition count per upload both fall; the batch reports `has_ops` so an empty frame submits nothing.
Kill: kill if the belt already merges per allocation and the counter it posts shows one submission per frame; the keyed set pays only when a frame emits many small copies.

**3. An SDF glyph atlas for text at any scale** | `modules/vg/src/lib.rs:1-6` ("Peridot Vector Graphics Dept. powered by Pathfinder 2(lyon)"; `mod font`, `sdf_generator`, `pub use self::sdf_generator::SDFGenerator`) and the vector path/builder bit at `:16-26` | MIT | PRESENTATION (HUD) | lands on `crates/render/src/font_data.rs` and the HUD/text path in `crates/ui`/`crates/render`
Idea: bake glyph outlines to a signed-distance field once, then draw at any pixel size with one tap and a smoothstep — a HUD scale or a bigger window needs no second atlas, and an outline or a drop shadow is another threshold, not another bake.
Moves: per-size bitmap atlases become one atlas per face; a resolution change no longer invalidates text.
Kill: kill if text is only ever drawn at one size in one layout (then the bitmap atlas is smaller and simpler). An SDF tap is one texture read, but the bake stage and the edge quality have to be earned.

**4. A hash-tree asset archive with page-aligned blocks and per-entry compression** | `modules/archive/src/entry_tree.rs:5-31` (`TARGET_PAGE_BLOCK_SIZE = 2048`, `EXACT_TREE_ENTRY_STRIDE = 8*2`, `NORMAL_TREE_ENTRY_STRIDE = 8*3`, `first_hash_tree_block_size`, `MAX_ENTRY_COUNT`), `modules/archive/src/lib.rs:20-34` (`ContentFlags::ROOT_HASH_TREE_EXACT`, `CompressionMethod::{None, Zlib, Lz4, Zstd11}`), `:13-15` (`Archive`, `ArchiveAsync`, `ArchiveBinReader`, `ArchiveBinReaderAsync`) | MIT | TOOLING / content | lands on the content cook/pack (`content/`, `crates/assets`)
Idea: one file the runtime opens, indexed by a name hash-tree laid out in 2 KB page blocks (an exact-name tree plus a normal tree with fixed strides), each entry compressed with the method chosen at cook, and an async reader so a load is a future.
Moves: the runtime stops walking a directory; a load is a page-aligned read plus a decompress, and the index needs no parse of the whole archive.
Kill: kill if the pack is already one file with a sorted name index, or if the page layout fights range requests in the wasm/web build (which is where a browser reads a slice of the file) — then keep only the per-entry compression choice.

## Not worth it (read, rejected)

- `base/src/resource/memory.rs:12-51` (`SharedMemoryBlock` + `SharedMutableRef` + `ExclusiveLockedSharedMemoryBlock`): a hand-rolled shared-mutable reference for GPU memory with the free in `Drop`. Our rules keep locks and refcounts out of the tick and out of the render hot path; adopt the pattern only if a measured contention case appears.
- `modules/asset-processing/src/lib.rs:11-26`: an `AssetProcessor { can_process, process }` trait with a `Result<(), Box<dyn Error>>`. Our cook already owns that seam with typed errors; the boxed error would be a step back.
- `cradle/*` (`Cargo.template.toml` per platform + a thin shell): close to what `crates/web` and the native entry already do, and it would move a lot of build tooling for no gate.

DONE 4 steals, 8 donor files read (async_fence_driver.rs, batch.rs, resource/memory.rs, vg/lib.rs, archive/lib.rs, archive/entry_tree.rs, asset-processing/lib.rs, base/lib.rs, Cargo.toml), plus a tree listing.
