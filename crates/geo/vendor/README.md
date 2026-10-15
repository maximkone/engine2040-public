# vendor

`xatlas/xatlas.cpp` and `xatlas/xatlas.h`: xatlas by Jonathan Young (MIT), copied unmodified from the
`xatlas-rs-v2` 0.1.4 crate (MIT), which carries the same files from github.com/jpcy/xatlas. The files
hold their own license text: xatlas (MIT), thekla_atlas (MIT, Thekla and NVIDIA), Brandon Pelfrey (MIT),
Bruno Levy (BSD-3-Clause). All four are on the `deny.toml` allow list.

Why a copy: `xatlas-rs-v2` generates its bindings with bindgen at build time, which needs a libclang
install; this machine has none and the engine builds with cargo alone. `xatlas_shim.cpp` is a small C ABI
of our own over the same source (one call in, a handle out), and `build.rs` compiles both with the `cc`
crate when the `uv` feature is on. `XA_MULTITHREADED=0` is defined so the result never depends on the
thread count.

Update: copy the two files again from a newer crate or from upstream, rerun `cargo test -p engine-geo
--features uv`.
