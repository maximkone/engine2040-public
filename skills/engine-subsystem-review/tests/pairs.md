# Skill-on vs skill-off pairs: engine-subsystem-review

Three review situations. "Off" is what a reviewer says from general habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the evidence it asks for.

## P1: variable dt in the authoritative tick (items 1, 6)

Situation: a diff in `app` replaces the fixed 1/60 s step with `dt = now - last` "so slow machines feel smooth". Physics and AI take dt as before. Commit body says "tests pass".

Off: "dt-based movement is standard and frame-rate independent. LGTM; maybe clamp dt to something sensible."

On: "Reject. Item 1: simulation must advance in fixed steps from an accumulator, with a cap on catch-up steps; one slow frame that causes two steps makes the next slower (spiral). Item 6: a measured dt makes the tick depend on wall-clock timing, so replay and rollback (net, arena) diverge. Evidence needed: seed 2040, 600 ticks gives the unchanged golden hash, plus a test that feeds irregular frame times and still produces the same hash."

## P2: asset registry reuses slots (item 4)

Situation: a diff in `assets` stores loaded resources in a slot table; entities keep `AssetId(u32)` which is the slot index. Unloading a level frees slots and the next load takes the lowest free one. A new `load_blocking()` is called from a system during the tick. Test: load, unload, load once; all green.

Off: "Looks clean, index lookup is O(1) and the test covers load/unload. Approve."

On: "Two findings. Item 4: a plain slot index lets an old handle read the next occupant of its slot; the handle needs index plus generation so a stale id resolves to None. Test to add: load A, unload, load B into the same slot, resolve the old id, expect None. Item 4 again: `load_blocking()` inside the tick is a hitch; stage the load before the level starts or make it async and poll for readiness."

## P3: a "3x faster" parallel rebake (items 5, 6, 8)

Situation: a diff in `nav` rebuilds tiles with a parallel iterator and collects results into a `Mutex<HashMap<TileId, Mesh>>`, then walks the map to build the path graph used by the tick. The PR says "3x faster" from one debug-build run on a small map.

Off: "Nice speedup. Mutex is fine here. Approve."

On: "Not yet. Item 8: a debug run on a small map is not the workload; give before/after from the perf crate in release at real tile counts. Item 5: results must be merged in a fixed order (sort by TileId), not in finish order. Item 6: walking a HashMap feeds the tick an iteration order that can change between runs, so the golden hash may drift; show it unchanged, or switch to a sorted Vec or BTreeMap. Also say what the Mutex guards and whether per-job output buffers remove the need for it."
