// node --test tools/asset-gen/fighter.test.mjs (no network, no Blender, no window)
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { checkName, clipMap, checkFamily, proofVerdict, plan, main, ROLES } from "./fighter.mjs";

test("the variant name is the binder's charset", () => {
  assert.equal(checkName("rb_legionary"), "rb_legionary");
  for (const bad of ["RB_Legionary", "rb legionary", "9rb", "", undefined]) {
    assert.throws(() => checkName(bad), /lowercase/);
  }
});

test("clips default to the proven pack files and --clip overrides one role", () => {
  const map = clipMap("P", ["attack3=sword and shield kick.fbx", "death=C:\\x\\die.fbx"]);
  assert.equal(map.idle, join("P", "sword and shield idle.fbx"));
  assert.equal(map.attack3, join("P", "sword and shield kick.fbx"));
  assert.equal(map.death, "C:\\x\\die.fbx");
  assert.throws(() => clipMap("P", ["block=x.fbx"]), /role one of/);
});

test("the roman kit grafts the gladius and the scutum; none grafts nothing", () => {
  const avatar = { group: "Professions", name: "Military_Male_01" };
  const clips = clipMap("P");
  const roman = plan({ root: "R", blender: "b", node: "n", avatar, name: "rb_x", home: "H", pack: "P", clips, kit: "roman", out: "O" });
  assert.deepEqual(roman.map((s) => s.label), ["fetch (rocketbox.mjs)", "kit (roman_kit.py)", "cook (rocketbox_fighter.py)"]);
  const cook = roman.at(-1).args;
  assert.ok(cook.includes("--weapon") && cook.includes("--shield"));
  assert.equal(cook.filter((a) => a === "--clip").length, ROLES.length);
  const bare = plan({ root: "R", blender: "b", node: "n", avatar, name: "rb_x", home: "H", pack: "P", clips, kit: "none", out: "O" });
  assert.equal(bare.length, 2);
  assert.ok(!bare.at(-1).args.includes("--weapon"));
});

function family(body) {
  const sha = createHash("sha256").update(body).digest("hex");
  return { variants: [{ name: "rb_a", body_glb_sha256: sha, joints: 57, clips: 6, tris_lod0: 100 }] };
}

test("the family check ties the manifest row to the files", () => {
  const body = Buffer.from("glb");
  const files = (rel) => (rel.includes("rb_a") ? body : null);
  assert.deepEqual(checkFamily(family(body), "rb_a", files), []);
  assert.match(checkFamily(family(body), "rb_b", files).join(), /no manifest row/);
  assert.match(checkFamily(family(Buffer.from("old")), "rb_a", files).join(), /does not match/);
  const noClip = (rel) => (rel.includes("death") ? null : files(rel));
  assert.match(checkFamily(family(body), "rb_a", noClip).join(), /rb_a_death\.glb is missing/);
  const short = family(body);
  short.variants[0].joints = 22;
  assert.match(checkFamily(short, "rb_a", files).join(), /joints 22/);
});

test("the window proof needs exit 0 and view=ok on every capture line", () => {
  const good = "arena capture: frames=4 dir=x view=ok\n";
  assert.ok(proofVerdict(0, good).ok);
  const sunk = proofVerdict(0, "arena capture: frames=4 view=FAIL(view_eye_in_body)\n");
  assert.match(sunk.wrong.join(), /view FAIL\(view_eye_in_body\)/);
  const boxes = proofVerdict(1, "arena notice: x; drawing staged boxes\narena capture: frames=4 view=ok\n");
  assert.match(boxes.wrong.join(), /exit 1/);
  assert.match(boxes.wrong.join(), /staged boxes/);
  assert.match(proofVerdict(0, "").wrong.join(), /no 'arena capture:' line/);
  assert.match(proofVerdict(0, "arena capture: frames=4\n").wrong.join(), /no view verdict/);
});

test("usage errors exit 2 before any work", () => {
  const log = console.error;
  console.error = () => {};
  try {
    assert.equal(main(["node", "fighter.mjs", "--avatar", "Adults/X", "--name", "Bad Name"]), 2);
    assert.equal(main(["node", "fighter.mjs", "--name", "rb_a"]), 2);
  } finally {
    console.error = log;
  }
});
