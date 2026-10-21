# Bevy port map

Historical, the viewer was dropped 2026-10-07.

The picture is our own renderer in `crates/render`. Never build or extend a Bevy viewer. Reading Bevy source and stealing its methods into our code is fine.

Reference version: Bevy 0.19.1. Bevy 0.20 is a release candidate: do not use it. Old tutorials (0.12 to 0.18) use names that no longer exist. The WGSL ports in `crates/render/src/v1_gpu.rs` came from Bevy `v0.15.0`, so the 0.19 names apply to new ports only. Write the donor version in the header of each port.

How to port a Bevy method:

- Write plain Rust over our own types. A Bevy system becomes a function over our state. Sim crates never depend on Bevy.
- Keep the fixed tick free of Bevy scheduling. Time and order come from our tick (`docs/HASH-LAW.md`).
- Check any Bevy name you are unsure of in the 0.19.1 source, not a tutorial.
- Steal note: `bevy@<tag>`, the donor path and symbol, then the line you wrote in our code.

Renamed names are in `bevy-0.19-names.md` in this folder.
