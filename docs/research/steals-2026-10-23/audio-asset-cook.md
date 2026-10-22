# Audio and asset cook/streaming steals - 2026-10-23

Read at pinned commits. Five steals for `crates/assets` and `crates/audio`.
Every donor claim carries a blob URL at a 40-hex sha.

## Method

| repo | sha | files read |
|---|---|---|
| tesselode/kira | `74b3ef0fd8bd15f90e3b469208ee741f04374a62` | `crates/kira/src/track/sub.rs`, `track/send.rs`, `parameter.rs`, `clock/clock_speed.rs`, `sound/streaming/sound/decode_scheduler.rs` |
| ValveSoftware/steam-audio | `480dd64f513cc8a6437e7d5b9eb0d3f1d30c2fac` | `core/src/core/path_visibility.cpp`, `hrtf_map.cpp`, `hrtf.cpp`, `overlap_save_convolution_effect.cpp`, `reflection_baker.cpp` |
| zeux/meshoptimizer | `f0a684f11ead064e176775c9b796f82a55910e92` | `src/vertexcodec.cpp`, `src/indexcodec.cpp`, `src/quantization.cpp`, `src/vertexfilter.cpp`, `src/meshoptimizer.h` |
| KhronosGroup/KTX-Software | `e131a57723a89dff9d9f731e48a86dbc0dd29e63` | `lib/src/writer2.c`, `lib/src/texture2.c`, `lib/include/ktx.h`, `lib/src/basis_transcode.cpp`, `external/basis_universal/transcoder/basisu_transcoder.h` |

Pinned with `git ls-remote <repo> HEAD`, then a blobless shallow clone and
`git rev-parse HEAD`. The brief's `valveresearch/steamaudio` does not exist
(404); the open-source repo is `ValveSoftware/steam-audio`, pinned above.

Local files read: `crates/assets/src/{bundle,ktx2_cook,stream,archive,cooked,mesh_cook,rig_cook}.rs`,
`crates/audio/src/{lib,dsp,mix,declick,hrtf,occlude,reverb}.rs`,
`content/SIZE-BUDGET.md`, `content/arena/sources/PROVENANCE.md`,
`docs/plan/README.md`, `docs/FOCUS.md`.

Not run: no donor build, no `cargo`, no cook, no re-encode, no timing. All
byte numbers below are the donors' docs or arithmetic over our own encoder,
except where our own test already measured (noted).

## engine2040 today

| capability | file:line | honest state |
|---|---|---|
| cooked mesh type | `crates/assets/src/cooked.rs:77` | `positions`/`normals` as `Vec<[f32;3]>`, `indices` as `Vec<u32>`; no packed vertex form |
| mesh on the wire | `crates/assets/src/bundle.rs:586-622` | every vertex channel written raw `f32` (`3+3+2+4` = 48 B/vertex), then `u32` indices (12 B/triangle); no codec |
| mesh cook | `crates/assets/src/mesh_cook.rs:695,1144` | quadric LOD + vertex-cache and overdraw order exist; no vertex or index codec |
| texture cook | `crates/assets/src/ktx2_cook.rs:486` | BC1/BC5 KTX2 with a full mip chain, no supercompression |
| KTX2 reader | `crates/assets/src/ktx2_cook.rs:569-583` | reads only what it cooked; `Unsupported("supercompression")` |
| streaming queue | `crates/assets/src/stream.rs:1-16,449` | real prioritised queue, but "planned, no caller yet": no bytes are read; `publish` counts simulated chunks |
| archive | `crates/assets/src/archive.rs:1-18` | zero-copy mmap archive, `planned, no caller yet`, no compression |
| audio buses | `crates/audio/src/dsp.rs:48-117` | three fixed buses (SFX, Music, Dialogue) with constant gains, summed in fixed order; no sends, no effect chain |
| audio parameter change | `crates/audio/src/declick.rs:1-6`; `occlude.rs:23-33` | per-tick integer slew and gain ramps only; no general tween, no clock-synced start |
| HRTF | `crates/audio/src/hrtf.rs:16,28` | 8 octants, 16-tap synthetic HRIR; no measured grid |
| occlusion | `crates/audio/src/occlude.rs:216,280` | wall-by-wall `segments_cross` gain + one-pole lowpass; a wall is crossed or not |
| reverb | `crates/audio/src/reverb.rs:97` | one parametric tunnel preset (`ReverbZone::tunnel`); no baked impulse response |
| cooked content | `content/arena/kit.e2bundle` 1,360,818 B; `content/arena/sources/PROVENANCE.md:1-3` | real cooked bundle in the tree; sources are ambientCG CC0. There is no root `content/PROVENANCE`; provenance is one file per directory (`content/arena/sources`, `content/arena/hdri`, `content/maps/*`, `content/rome`) |
| content budget | `content/SIZE-BUDGET.md:10` | total ~110 MB, audio ~3 MB: audio size is not the pressure, mesh/texture bytes are |

Audio is a parked area (`docs/FOCUS.md:47`: "audio, UI, modding ... no new
rows ... unless a bar needs them"). The audio cards below are ideas that wait
for the impact-audio bar (`docs/plan/melee-feel.md:62`, FEEL8), not board rows.
Assets are not parked; `docs/plan/README.md:39` (`content.md`) is NEXT.

## The steals, ranked

### 1. Quantized vertex stream + meshopt block codec

Donor:
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/meshoptimizer.h#L352-L361

> "Encodes vertex data into an array of bytes that is generally smaller and
> compresses better compared to original." ... "For maximum efficiency the
> vertex buffer being encoded has to be quantized and optimized for locality
> of reference (cache/fetch) first."

Code: `meshopt_encodeVertexBufferLevel` walks vertex blocks whose size comes
from the vertex stride and deltas every 4-byte channel:
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/vertexcodec.cpp#L1632-L1688

> `data = encodeVertexBlock(data, data_end, vertex_data + vertex_offset * vertex_size, block_size, vertex_size, last_vertex, channels, version, level);`

Companions read:
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/quantization.cpp#L14
(`meshopt_quantizeHalf`, float bits rounded to half) and
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/vertexfilter.cpp#L1276
(`meshopt_encodeFilterOct`, a unit vector to K-bit X/Y).

What it buys: the cooked bundle stops carrying 48 raw bytes per vertex.
Half positions (6 B), half UVs (4 B), 8-bit octahedral normals (4 B) and an
8-bit octahedral tangent frame take a vertex to about 18 B before the block
codec runs, so a 100k-vertex mesh drops from ~4.8 MB to ~1.8 MB, and the
codec takes another bite. Our own rig cook already records the pressure:
`crates/assets/src/rig_cook.rs:7` - "6,239,621 B brotli against a 2,040,000 B"
budget.

Local gap: `crates/assets/src/cooked.rs:77-87` and
`crates/assets/src/bundle.rs:586-622` (raw `f32` loop, raw `u32` loop).

Size: scalar port of `vertexcodec.cpp` encode/decode plus the two filters,
roughly 1,200 lines with tests; the codec's SIMD paths are not needed on the
cook side and the scalar decoder is the reference.

Risk: medium. The decoder must be safe on untrusted bundle bytes (bounds
checks, typed errors). Oct normals and half positions are lossy, so the cook
needs an error-bound test (the shape at `crates/assets/src/ktx2_cook.rs:678`).
`meshopt` requires quantized input, so the schema and the digest move together.

Reproducibility: pure integer/bit operations, no float FMA in the codec; same
input -> same bytes twice. Encoding an already-quantized stream is lossless, so
`decode(encode(q)) == q` plus the quantization error bound is the check.
Cooked mesh digests (`mesh_cook.rs:1246`) change; the sim golden
`d57b5c8ae3096614` is a fixture that loads no content bundle, but run the
golden gate anyway - if it moves, the codec leaked into the sim and the change
is refused.

### 2. Index codec: edge FIFO + vertex FIFO

Donor:
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/meshoptimizer.h#L263-L264

> "Encodes index data into an array of bytes that is generally much smaller
> (<1.5 bytes/triangle) and compresses better (<1 bytes/triangle) compared to
> original. Input index buffer must represent a triangle list."

Implementation (`getEdgeFifo`, per-triangle code byte, aux table):
https://github.com/zeux/meshoptimizer/blob/f0a684f11ead064e176775c9b796f82a55910e92/src/indexcodec.cpp#L170-L225

> `int fer = getEdgeFifo(edgefifo, indices[i + 0], indices[i + 1], indices[i + 2], edgefifooffset);`

What it buys: our indices are 12 B/triangle; the donor's own bound is under
1.5 B/triangle, so a 200k-triangle kit drops ~2.4 MB to ~0.3 MB. It also
compresses better under any later container compression.

Local gap: `crates/assets/src/bundle.rs:620-622`. The precondition mostly
exists: `cook_mesh` applies `optimize_index_order` (`mesh_cook.rs:1162`),
which runs `optimize_vertex_cache` first (`:794`) and keeps it unless overdraw
scores better (`:799-805`). So every cooked LOD is already cache-ordered, which
is what the donor asks for (`meshoptimizer.h:267`). The FIFO codec would then
sit on top of an order we already produce.

Size: ~300 lines plus the codec table and a fuzz-shaped decode test.

Risk: low-medium. No fidelity loss (lossless index remap), but the encoder
must never emit a code the decoder misreads; round-trip over generated meshes
and a corrupt-input refusal test are required.

Reproducibility: deterministic integer state machines; same mesh -> same bytes
twice. Bundle digest changes; re-cook `content/arena/kit.e2bundle` in the same
commit. No golden effect for the same reason as steal 1.

### 3. KTX2 per-mip Zstd supercompression

Donor:
https://github.com/KhronosGroup/KTX-Software/blob/e131a57723a89dff9d9f731e48a86dbc0dd29e63/lib/src/writer2.c#L797-L885

> `ktxTexture2_DeflateZstd(ktxTexture2* This, ktx_uint32_t compressionLevel)`
> ... `for (int32_t level = This->numLevels - 1; level >= 0; level--) {`
> `nindex[level].byteOffset = levelOffset;`
> `nindex[level].uncompressedByteLength = cindex[level].byteLength;`

Scheme id:
https://github.com/KhronosGroup/KTX-Software/blob/e131a57723a89dff9d9f731e48a86dbc0dd29e63/lib/include/ktx.h#L716
- `KTX_SS_ZSTD = 2, /*!< ZStd supercompression. */`.

What it buys: each mip level is compressed on its own and keeps its
`uncompressedByteLength`, so a streamer can fetch and inflate one level
instead of the whole texture. That is exactly the shape our
`stream.rs` queue needs for mip streaming, and it shrinks the cooked texture
bytes our bundle and page fetch carry.

Local gap: `crates/assets/src/ktx2_cook.rs:486` writes full mips with no
supercompression, and `crates/assets/src/ktx2_cook.rs:569-583` refuses any
KTX2 that has it. Our own test
measures the mip total we are compressing: `ktx2_cook.rs:852-853`
"`mips == 349_552`" for a 512x512 BC5 normal.

Size: parser/writer change in one module plus a zstd call per level; ~200
lines of Rust glue, most of the work is the dependency and the wasm budget.

Risk: medium. Adds a zstd dependency to the `import` feature (C via
`zstd-sys`, or a pure-Rust decoder plus a cook-only encoder); the page bundle
gains decoder bytes. The KTX2 reader must refuse a level whose inflated length
disagrees with `uncompressedByteLength`.

Reproducibility: pin the zstd version in `Cargo.lock` and the compression level
in a const; same binary then produces the same bytes twice. Assert both
`compressed x2 equal` and `inflate(compressed) == level bytes`; zstd output is
not guaranteed across versions, so the cook-twice check is the authority, not a
golden hash. Texture digests change; golden unaffected.

### 4. Send/return tracks and clock-synced parameter tweens (kira)

Donor:
https://github.com/tesselode/kira/blob/74b3ef0fd8bd15f90e3b469208ee741f04374a62/crates/kira/src/track/sub.rs#L49-L52

> `effects: Vec<Box<dyn Effect>>,`
> `sends: Vec<(SendTrackId, SendTrackRoute)>,`

Routing after the track renders:
https://github.com/tesselode/kira/blob/74b3ef0fd8bd15f90e3b469208ee741f04374a62/crates/kira/src/track/sub.rs#L244-L252

> `send_track.add_input(out, volume.value());`

A send track is itself a volume plus an effect chain:
https://github.com/tesselode/kira/blob/74b3ef0fd8bd15f90e3b469208ee741f04374a62/crates/kira/src/track/send.rs#L29-L36

> `volume: Parameter<Decibels>,`
> `effects: Vec<Box<dyn Effect>>,`

Parameter tween: a target, a duration and a start time that can be a clock
time, updated by audio time rather than by a tick:
https://github.com/tesselode/kira/blob/74b3ef0fd8bd15f90e3b469208ee741f04374a62/crates/kira/src/parameter.rs#L96-L160

> `pub fn update(&mut self, dt: f64, info: &Info) -> JustFinishedTween`
> `StartTime::ClockTime(clock_time) => {`
> `info.when_to_start(*clock_time) == WhenToStart::Now`
> `.map(|target| T::interpolate(*start, target, tween.value(*time)))`

What it buys: one voice can feed the dry bus and a reverb send with its own
tweened volume, so the tunnel tail is a send, not a per-voice special case;
music ducking becomes a route, not a hardcoded crossfade. The clock start lets
a cue fade begin on a named tick instead of at the next callback.

Local gap: `crates/audio/src/dsp.rs:48-117` is three fixed buses with gains
from constants; `crates/audio/src/declick.rs:1-6` only ramps a gain between 0
and 1; `occlude.rs:23-33` is a per-tick slew. No `Send`, no effect chain, no
general `Parameter`.

Size: ~400-600 lines: send track, route volume, a small `Tween`/`Parameter`
over the fixed tick, plus an effect chain trait. Port the shape only; kira's
`f64` and `Duration` do not belong on our audio path.

Risk: low-medium. This is presentation only and must stay 0-allocation on the
callback (`engine.rs` drain path already proves that). Summation order must be
fixed for the mix determinism test.

Reproducibility: audio never feeds the golden (`reverb.rs:6`, `hrtf.rs:8`,
`occlude.rs:10`: "presentation only, never hashed"). Same tick inputs -> same
mix twice remains the test; the golden must not move.

### 5. Soft occlusion from the unoccluded ray fraction (Steam Audio)

Donor:
https://github.com/ValveSoftware/steam-audio/blob/480dd64f513cc8a6437e7d5b9eb0d3f1d30c2fac/core/src/core/path_visibility.cpp#L46-L79

> "To determine mutual visibility between probes A and B, they are considered
> as spheres of a given radius. A set of numSamples points are generated in A
> and B, and rays are traced between each pair ..."
> `if ((static_cast<float>(numVisibleSamples) / static_cast<float>(mSamples.size(0))) >= threshold)`

What it buys: a voice behind a pillar edge fades by the visible fraction
instead of switching on one center segment. The same idea works with our
integer wall list: sample a small sphere around the source, count how many
source-to-listener segments are clear, and use the fraction as the target gain
the existing slew already smooths.

Local gap: `crates/audio/src/occlude.rs:216` `segments_cross` and `:280`
`target_muffle` cross one segment per wall set; `gunfire.rs:473-489` adds doors
on the same binary path. There is no partial visibility.

Size: ~150 lines plus tests. Sample count must be capped (for example 8
points, then the existing `ATTACK_MS`/`RELEASE_MS` slew), and the cost per
voice per tick stays under the module's stated 0.1 ms budget
(`occlude.rs:13`).

Risk: medium, because cost scales with voices x samples. Keep it integer Q16,
reuse `segments_cross`, and only run for the nearest N audible voices. No new
data, no scene dependency.

Reproducibility: integer-only, same inputs -> same gain curve twice; audio is
not hashed. Golden untouched. This is the cheapest audio-quality win of the
five and needs no measured HRIR data.

## What not to steal

- **Steam Audio baked reflections / convolution reverb.** Read
  `reflection_baker.cpp:33-70` and `overlap_save_convolution_effect.cpp:26-60`.
  It bakes probe IRs and runs partitioned FFT convolution. That needs an FFT,
  a probe bake pipeline and baked data in the bundle, for presentation-only
  audio whose area is parked. Our parametric tunnel (`reverb.rs:97`) is enough
  until a look/feel bar asks for real room tails.
- **Basis Universal runtime transcode.** `lib/src/basis_transcode.cpp:45-70`
  and `basisu_transcoder.h:54-59` transcode UASTC/ETC1S to BC/ASTC on load.
  It is the right answer when one container must serve several GPU families.
  Today our cook targets BC1/BC5 for one class of device and the web bundle is
  already tight; the C encoder plus runtime transcoder is too much for no
  second target.
- **Kira's per-sound decode thread.** `decode_scheduler.rs:95-110` spawns a
  thread per streaming sound with a ring buffer. Our audio content is ~3 MB
  (`SIZE-BUDGET.md:10`) and the feed is deliberately lock-free with no thread
  join on the callback. Revisit only when a long music bed must stream.
- **Kira's `f64`/`Duration` parameter types.** Take the tween shape, keep our
  Q16/Q24 integer math (`declick.rs:1-6`).

## Could not verify

- Steam Audio's default HRIR is CIPIC-derived (`hrtf.cpp:23-27` includes
  `cipic_124.inl`; `hrtf_map.cpp:56-62` names the CIPIC database:
  https://github.com/ValveSoftware/steam-audio/blob/480dd64f513cc8a6437e7d5b9eb0d3f1d30c2fac/core/src/core/hrtf_map.cpp#L56-L62).
  I did not confirm the data's redistribution terms, so the HRTF-grid steal
  (`hrtf_map.cpp:156-170` bilinear weights) is dropped, not recommended.
- No donor code was built, profiled or fuzzed. Byte wins are donor doc claims
  plus arithmetic over `bundle.rs`; I did not re-cook the 1,360,818 B
  `content/arena/kit.e2bundle` or measure any saving.
- KTX2 zstd decode cost, wasm size and cross-version byte stability were not
  measured. The cook-twice test, not a hash, is the proposed guard.
- I read google/draco's pinned sha only (`d429de84b52a860705507da0884764c5d8321423`)
  and no file, so no Draco claim is made; per the rules it is dropped.
- I did not verify that `dist/web` and the native path both accept a KTX2 with
  supercompression; that is the first spike step for steal 3.

## Cards

- CARD | P3 | quantized vertex stream with the meshopt block codec | zeux/meshoptimizer@f0a684f `src/vertexcodec.cpp` + `src/quantization.cpp` + `src/vertexfilter.cpp` (MIT) | not run: one arena kit mesh and one fighter LOD0, raw `3+3+2+4` f32 + u32 -> half/oct quantized + codec; metrics: encoded bytes, decode equals quantized input bytes, decode ms, cook twice equal; kill: decode differs, bundle grows, or the page first-fetch budget cannot be shown to shrink | spike first (AAC1)
- CARD | P3 | edge-FIFO index codec over the cache-optimized order we already cook | zeux/meshoptimizer@f0a684f `src/indexcodec.cpp` (MIT) | not run: a 200k-triangle kit mesh, `u32` indices -> encoded; metrics: bytes, bytes/triangle against the donor's <1.5, round-trip exact, corrupt input refused; kill: round-trip differs or saving is under 2x | spike first (AAC2)
- CARD | P3 | per-mip Zstd supercompression in the KTX2 cook, one inflatable level per mip | KhronosGroup/KTX-Software@e131a577 `lib/src/writer2.c` + `lib/include/ktx.h` (Apache-2.0) | not run: the five CC0 1K arena maps, BC1/BC5 full mips -> level 15 zstd per level; metrics: KTX bytes, inflate ms per level, wasm decoder bytes, cook twice equal, parse round-trip; kill: inflated level differs, bytes not smaller, or the page budget cannot carry the decoder | spike first (AAC3)
- CARD | P4 | send/return tracks plus clock-synced parameter tweens on the mix | tesselode/kira@74b3ef0 `crates/kira/src/track/sub.rs` + `parameter.rs` (MIT/Apache-2.0) | not run: one arena mix with a reverb send and a music duck, 60/120/144 Hz ticks, fixed bus -> send graph; metrics: same voice count, 0 callback allocations, mix bytes identical x2, tween lands on the named tick; kill: callback allocates, a mix differs twice, or the golden moves | spike first (AAC4)
- CARD | P4 | soft occlusion: target gain from the fraction of clear source-listener rays | ValveSoftware/steam-audio@480dd64 `core/src/core/path_visibility.cpp` (Apache-2.0) | not run: one voice behind a pillar edge, binary wall crossing -> 8-sample fraction with the existing slew; metrics: gain curve continuous (no step over 1/8), segment tests per voice per tick, cost under 0.1 ms, x2 identical; kill: cost over budget or the curve still pops | spike first (AAC5)
