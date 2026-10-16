// REVERTED 2026-10-09 (steal campaign lap 9): full content removed, it did not
// compile (E0614 bool cannot be dereferenced at three filter sites).
// Ready patch: restore the port and change `**culled` / `**flag` to `*culled` /
// `*flag` at the three filter sites, then rerun
// cargo test -p engine-assets --test it meshlet_cull_vectors:: (expect 5 passed).
// Donor: zeux/meshoptimizer MIT https://github.com/zeux/meshoptimizer
// (src/meshletutils.cpp). No donor bytes; fresh write against meshlet.rs bounds.
// (remainder removed with the port; see above. The full port text lives in the
// wave-4 helper output quoted in this lap's steal-campaign report.)
