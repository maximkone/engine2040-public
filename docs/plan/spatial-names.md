# Spatial names, tags and sizes at the door (2026-10-18)

Question from the audit (`docs/research/spatial-2026-10-18/01-ours.md` section 4): every door actor is a 1 m cube
named `prop_NNNN`; nodes have no name or tag. This note says what the data really holds and the smallest change
that fixes it without moving the golden. It feeds SPR1 (lane A); nothing here is code yet.

## What exists today (read in source)

- `engine_scene::SceneNode` is `{id, parent, transform}` only (`crates/scene/src/codec.rs:36`). It never carried a
  name, and it should not: the scene save hash and every id-sorted order depend on it.
- Components live beside the hierarchy, not on the node: `Document.components: BTreeMap<String, BTreeMap<ComponentKind,
  Component>>` keyed by node id text (`crates/editor/src/commands.rs:58-68`). Eight kinds
  (`crates/scene/src/component.rs:110`).
- A node that has a **Mesh** component already carries a name: `Mesh.asset`, an `AssetName` (ASCII, 1..64 bytes,
  `0-9 A-Z a-z - _ . /`). A **Collider** carries real half extents in meters (`Collider.half`, box/sphere/capsule), and
  the node transform carries scale. A **Material** carries a material asset name.
- The door throws all of that away: `scene_tools::snapshot` builds `Actor { name: String::new(), sx: 1.0, sy: 1.0,
  sz: 1.0, .. }` and the label `prop_NNNN` is invented from the position in the UUID-text-sorted order
  (`crates/endpoint/src/scene_tools.rs` near `snapshot`, and `intents.rs:433`).

So the audit's "no name field" is true for the node, but the information exists one table over. A name does not need
a new field to be useful.

## Smallest change, in order (each step lands alone)

1. **Label from the Mesh asset, no new data (SPR1).** In `snapshot`, read `doc.components[id][Mesh]`. Label =
   `<asset basename>_<n>` where `<n>` counts same-asset nodes in UUID-text order (so inserting an unrelated node never
   moves another label; inserting a node of the same asset can only append if the sort keeps new ids last, which UUID
   v7 time order gives; say so in the tool text). Nodes without a Mesh keep `node_<n>`. `Actor.name` is then a real
   token, `dupeSpam` and `grammar` start to mean something, and the brief reads `lamp_2 [uuid]`.
2. **Size from Collider and scale (AD7, already a board row).** `sx,sy,sz = 2 * half * world scale` for a box
   collider (sphere: diameter; capsule: radius and height). No collider: fall back to 1 m and mark the row
   `size=guess` in the brief so the model is told the number is a default, not a measurement.
3. **Brief row per object (SPR1).** `lamp_2 x y z sx sy sz yaw_octant` in cm: pure formatting over the actors the
   door already builds. Budgeted with a cursor.
4. **Optional free-form name and tags, only if step 1 proves too coarse.** Add `name: Option<AssetName>` and
   `tags: [AssetName; 0..4]` to `Mesh`'s JSON form with `#[serde(default, skip_serializing_if = "Option::is_none")]`,
   and append them to `Component::to_bytes` **only when present** (a trailing length byte written only when non-empty
   changes nothing for existing saves; a decode that finds no trailing bytes reads `None`). A separate `Tag`
   component kind would change `ALL_KINDS` and the tag table, which every codec reads: avoid it.

## Why the golden does not move

The golden `d57b5c8ae3096614` is the simulation hash. Labels, sizes and brief text are produced on the read side
(`scene.brief`, `scene.query`, `scene.spatial`) from the document; none of them feeds the tick. Steps 1-3 change no
saved byte. Step 4 changes bytes only for nodes that set a name or tag, so existing scene saves and their digests
stay equal. Prove it with a save round trip over the shipped scenes before and after (digest equal) and
`tools`/HASH-LAW unchanged.

## Risks to check when SPR1 starts

- Same-asset ordinals are stable only if "UUID text order" is stable under insert; a delete shifts later ordinals.
  If the agent holds `lamp_2` across a delete, the door must keep answering by the uuid in brackets and say so.
- `AssetName` allows `/` and `.`; labels must be sanitized to `[a-z0-9_]` because the `grammar` check and
  `brief.rs` `sanitize_name` expect it.
- The eval stand-in already uses this convention (`crates/playtest/src/spatial_eval/llm.rs` labels `prop_NNNN`), so the
  SPR4 questions must be regenerated and the cassettes re-recorded after SPR1 lands (cassette keys include the prompt).
