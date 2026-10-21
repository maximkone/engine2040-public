// The concept sheet for all twenty classes, from the roster.
//
// The template is the one that works: front view, arms out from the sides so
// every limb is separate, a plain background, even light, the whole body in
// frame. Attempt 2 asked for a "hulking" orc and got a mass with no head and
// no arms; attempt 4 asked for a lean one with the arms out and got a
// character with a jaw, elbows, knees and a shield. The input is the lever.
//
//   node tools/asset-gen/concepts.mjs            all twenty
//   node tools/asset-gen/concepts.mjs --only ashkin_gunner
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";

const ROOT = "C:\\engine2040";
const OUT = join(ROOT, "content", "generated", "concepts");
const ENDPOINT = "https://stable-diffusion-xl.endpoints.kepler.ai.cloud.ovh.net/api/text2image";
const NEGATIVE = "text, watermark, extra limbs, extra fingers, blurry, lowres, jpeg artifacts, photorealistic, multiple characters, scenery, background objects";

// The two faction palettes, in words the model understands.
const WARDEN_COLOUR = "bone white and silver plate armour with gold trim, steel blue cloth, leather straps";
const ASHKIN_COLOUR = "black iron plate armour with bronze trim, blood red cloth, bone studs";

// subject, gear, and the body the class wants. "lean" and "arms out" are
// what keep the generator from fusing the limbs into one mass. The gear
// follows the class sheet's loadout, so the art and the sheet agree; the
// gate checks that with a hash of the loadout.
const SPECS = {
  warden_fighter: ["a tall human soldier in a tabard over chain", "an arming sword in the right hand and a buckler in the left", "broad shoulders, squared stance"],
  warden_defender: ["a stocky human soldier in heavy silver plate", "an arming sword in the right hand and a heater shield in the left", "wide stance, thick armour"],
  warden_warrior: ["a lean human swordsman in bone white and leather", "an arming sword in each hand, one on each side", "light armour, athletic build"],
  warden_rogue: ["a lean human scout in a hooded leather coat", "a short sword held low in the right hand, the left hand empty", "slim build, travel cloak"],
  warden_assassin: ["a lean human in a dark hooded coat", "a short sword held low in the right hand, the left hand empty", "slim build, deep hood"],
  warden_archer: ["a lean human archer in leather and steel blue cloth", "a short bow held upright in the left hand and a quiver of marked arrows on the back", "slim build, light armour"],
  warden_mage: ["a human caster in a bone white robe with gold trim", "a long staff held upright in the left hand, the right hand open", "slender build, long robe"],
  warden_sorcerer: ["a human elementalist in a steel blue and gold robe", "a long silver focus staff in the left hand and an ember glow in the right", "slender build, flowing robe"],
  warden_enchanter: ["a human enchanter in a bone robe with steel blue trim", "a short silver focus rod in the right hand and a bone white grimoire in the left", "slender build, open hands"],
  warden_acolyte: ["a human acolyte in a bone white robe", "a mace in the right hand and a tome in the left", "slender build, short robe"],
  warden_cleric: ["a human cleric in a heavy bone robe with gold trim", "a hymn mace in the right hand and an oak ward shield in the left", "sturdy build, long robe"],
  warden_priest: ["a human priest in gold trimmed robes", "a censer staff in the right hand and a psalm book in the left", "slender build, long robe"],

  ashkin_combatant: ["a lean armoured orc warrior", "a great axe in the right hand, the left hand empty", "broad shoulders, lean build"],
  ashkin_templar: ["a heavy armoured orc in thick black plate", "an ashkin longsword in the right hand and a tower shield in the left", "thick armour, wide stance"],
  ashkin_attacker: ["a lean armoured orc with red war paint", "a pair of ashkin paired blades, one in each hand", "lean build, red sash"],
  ashkin_gunner: ["a lean armoured orc gunner", "a bronze shoulder cannon mounted on the right shoulder, both arms visible and clear of the body", "broad shoulders, lean build"],
  ashkin_officiator: ["a lean orc caster in a dark robe with ember runes", "a rite rod in the right hand and a charm in the left", "slender build, long robe"],
  ashkin_rune: ["a lean orc in a bone mask and dark robe", "a runewrought stave held upright in the left hand, the right hand open", "slender build, bone mask"],
  ashkin_life: ["a lean orc healer in a blood red robe with bone charms", "a ledger stave in the left hand and an oath tablet in the right", "slender build, long robe"],
  ashkin_shadow: ["a lean orc in a black hooded coat", "twin ash knives, one in each hand", "slim build, deep hood"],
};

const TEMPLATE = (subject, gear, body, colour) =>
  `single character only, one figure, front view full body of ${subject}, ${body}, ${gear}, ${colour}, ` +
  `standing straight facing the viewer, both arms held out from the sides away from the body, ` +
  `plain mid grey background, even flat lighting, whole body from head to boots in frame, ` +
  `stylized video game concept art, no layout, no panels, no studies, no text`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

async function fetchOne(prompt) {
  const res = await fetch(ENDPOINT, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ prompt, negative_prompt: NEGATIVE, size: "1024x1024", n: 1 }),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const type = res.headers.get("content-type") || "";
  const buf = Buffer.from(await res.arrayBuffer());
  if (type.includes("json")) {
    const j = JSON.parse(buf.toString("utf8"));
    const b64 = j.b64_json ?? j?.data?.[0]?.b64_json;
    if (!b64) throw new Error("no image in the response");
    return Buffer.from(b64, "base64");
  }
  return buf;
}

const roster = JSON.parse(readFileSync(join(ROOT, "content", "classes", "roster.json"), "utf8"));
mkdirSync(OUT, { recursive: true });
const only = arg("only");
const list = roster.classes.filter((c) => !only || c.id === only);
const delay = Number(arg("delay", "35")) * 1000;

// The art brief is the class sheet: a hash of the loadout, so the gate can
// tell that the sheet moved and the drawing is stale.
function loadoutHash(id) {
  const sheet = JSON.parse(readFileSync(join(ROOT, "content", "classes", `${id}.json`), "utf8"));
  return createHash("sha256").update(JSON.stringify(sheet.loadout ?? {})).digest("hex").slice(0, 12);
}

console.log(`generating ${list.length} concept(s) at ${delay / 1000}s apart`);
for (let i = 0; i < list.length; i++) {
  const cls = list[i];
  const spec = SPECS[cls.id];
  if (!spec) {
    console.log(`skip ${cls.id}: no spec`);
    continue;
  }
  const colour = cls.faction === "wardens" ? WARDEN_COLOUR : ASHKIN_COLOUR;
  const prompt = TEMPLATE(spec[0], spec[1], spec[2], colour);
  const name = `concept_${cls.id}`;
  try {
    const bytes = await fetchOne(prompt);
    const path = join(OUT, `${name}.jpg`);
    writeFileSync(path, bytes);
    writeFileSync(join(OUT, `${name}.provenance.json`), JSON.stringify({
      provider: "ovh-ai-endpoints",
      model: "stable-diffusion-xl-base-v10",
      endpoint: ENDPOINT,
      cost: "0 per request, anonymous, 2 requests per minute per IP",
      licence: "OpenRAIL++-M (Stability AI Community Licence)",
      commercial_use: true,
      served_from: "FR/EU",
      prompt,
      negative_prompt: NEGATIVE,
      size: "1024x1024",
      output: `content/generated/concepts/${name}.jpg`,
      bytes: bytes.length,
      class_id: cls.id,
      faction: cls.faction,
      role: cls.role,
      loadout_sha: loadoutHash(cls.id),
      purpose: "reference for the base mesh only: never a shipped asset, and the shipped mesh is rebuilt by us",
      template: "front view, arms out, plain background: the input is the lever",
    }, null, 2) + "\n", "utf8");
    console.log(`${i + 1}/${list.length} ${cls.id}: ${bytes.length} bytes`);
  } catch (err) {
    console.log(`${i + 1}/${list.length} ${cls.id}: FAILED ${err.message}`);
  }
  if (i < list.length - 1) await sleep(delay);
}
console.log("done");
