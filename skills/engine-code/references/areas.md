# Area rules

Full text behind `SKILL.md` Area rules.

**Simulation** (`docs/HASH-LAW.md`): fixed dt, explicit seeded streams (`Rng::fork`),
stable iteration order, NaN guards. No `HashMap` order, wall clock or pointer value in
hashed state. A new fixture gets its own version and a non-aliasing seed expansion.
Rollback needs identical results on every PC: sim code uses `+ - * /`, `sqrt`, `Fixed`
or `det_math`, never `sin`, `exp`, `powf` and friends.

**Scene, editor, commands** (`docs/EDITOR_AI_CONTROL.md`): persistent ids, generational
handles and content digests are three different things. One schema feeds serializer,
inspector and AI tools. Every edit (inspector, gizmo, script, AI) goes through the
command service: validate, preview without side effects, atomic apply, journal, undo.
A stale revision or bad id writes nothing. Play runs in a child process.

**Render** (`docs/VISUAL-TARGET.md`): work stage by stage, V1 to V5. Right-handed, +Y
up, meters. Explicit linear vs sRGB. Handle resize, minimize and device loss. Pixels
never enter the sim hash. See `wgpu-render`, `render-pipeline-math`.

**Audio and real time:** the mix callback never allocates, locks, logs or blocks. Prove
it with `alloc_probe::measure` over the callback body.

**AI** (`docs/EDITOR_AI_CONTROL.md`, `docs/RESEARCH-VERDICT.md`): engine-owned request,
result and capability types. Providers are adapters (mock, local, remote) with
deadlines, cancellation, bounded queues and offline fallback. Model output is a
proposal: validate schema, ranges, references and revision, run `engine-spatial`
checks, then apply through the command service. Models give labels and intent.
Deterministic code owns geometry. Replay uses recorded accepted actions and never calls
a model. See `ai-door`.
