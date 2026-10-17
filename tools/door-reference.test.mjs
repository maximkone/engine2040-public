// Run: node --test tools/door-reference.test.mjs
// DOC3: the door reference page is printed from the endpoint source, so it cannot drift from the verbs.
//  - a fixture crate with one pack, two verbs and one core tool: the verb, its argument range, its default, its scope, its
//    refusal text and its feature gate all reach the page; a changed range changes the page;
//  - the real crates/endpoint/src: every `pack.add(VerbSpec::of::<..>)` is on the page, nothing is left unparsed, and the
//    checked-in docs/book/door-reference.md equals what the tool prints now (regenerate with --write).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { PAGE_PATH, SRC_DIR, buildModel, generate, rangesFor, readCrate, renderPage } from "./door-reference.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const FIXTURE = {
  "packs/mod.rs": `
pub mod demo;
pub const PACK_NAMES: [&str; 1] = [demo::PACK];
`,
  "packs/demo.rs": `
use super::{PackDef, PackError, VerbSpec};
/// Pack name.
pub const PACK: &str = "demo";
/// Verb: stack some crates.
pub const TOOL_CRATE_STACK: &str = "crate.stack";
/// Verb: weigh a crate (feature gated).
pub const TOOL_CRATE_WEIGH: &str = "crate.weigh";
/// Most crates one stack may hold.
pub const MAX_CRATES: u32 = 12;

/// Stack request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", default)]
pub struct StackArgs {
    /// How many crates to stack.
    pub height: u32,
    pub label: String,
}
impl Default for StackArgs {
    fn default() -> Self {
        Self { height: 3, label: String::new() }
    }
}
pub struct StackOutput { pub stacked: u32, pub digest: String }
pub struct WeighArgs { pub grams: Option<u32> }
pub struct WeighOutput { pub heavy: bool }

fn crate_stack(_server: &McpServer, args: &StackArgs) -> Result<StackOutput, McpToolError> {
    if args.height == 0 || args.height > MAX_CRATES {
        return Err(bad(format!("height {} must be 1..={MAX_CRATES} crates; split the stack", args.height)));
    }
    if args.label.len() > 9 {
        return Err(McpToolError::new("label_too_long", "label holds more than 9 bytes; shorten it, then re-run"));
    }
    Ok(StackOutput { stacked: args.height, digest: String::new() })
}

crate::e2_tool!(
    /// \`crate.stack\`: stack crates.
    CrateStackTool,
    name = TOOL_CRATE_STACK,
    description = "Stack crates (height 1..=12, label). Undoable with crate.unstack.",
    readonly = false,
    args = StackArgs,
    output = StackOutput,
    invoke = crate_stack
);

fn bad_weight(message: String) -> McpToolError {
    McpToolError::new("bad_weight", message)
}

fn crate_weigh(_server: &McpServer, args: &WeighArgs) -> Result<WeighOutput, McpToolError> {
    if args.grams.unwrap_or(0) > 9000 {
        return Err(bad_weight(format!("grams {} must be at most 9000; weigh a lighter crate", 1)));
    }
    scale().map_err(|e| bad(e.to_string()))?;
    Ok(WeighOutput { heavy: false })
}

crate::e2_tool!(
    CrateWeighTool,
    name = TOOL_CRATE_WEIGH,
    description = "Weigh a crate. Read-only.",
    readonly = true,
    args = WeighArgs,
    output = WeighOutput,
    invoke = crate_weigh
);

pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(PACK, "crate games: stack and weigh", false);
    pack.add(VerbSpec::of::<CrateStackTool>(TOOL_CRATE_STACK, PACK, CapabilityCard::Commit)?);
    #[cfg(feature = "scales")]
    pack.add(VerbSpec::of::<CrateWeighTool>(TOOL_CRATE_WEIGH, PACK, CapabilityCard::Observe)?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    fn only_in_tests() {
        pack.add(VerbSpec::of::<GhostTool>("ghost.verb", PACK, CapabilityCard::Observe)?);
    }
}
`,
  "mcp.rs": `
pub const TOOL_PING: &str = "ping";
pub const CORE_TOOL_NAMES: [&str; 1] = [TOOL_PING];

/// Ping args.
pub struct PingArgs { pub word: Option<String> }
pub struct PingOutput { pub echoed: String }

pub struct PingTool;
impl ToolBase for PingTool {
    type Parameter = PingArgs;
    type Output = PingOutput;
    type Error = McpToolError;
    fn name() -> Cow<'static, str> { TOOL_PING.into() }
    fn description() -> Option<Cow<'static, str>> { Some("Echo one word. Read-only.".into()) }
    fn annotations() -> Option<ToolAnnotations> { crate::mcp::read_only_annotations() }
}
impl SyncTool<McpServer> for PingTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PING, |state| state.ping(&param))
    }
}
impl ServerState {
    fn ping(&self, args: &PingArgs) -> Result<PingOutput, McpToolError> {
        if args.word.as_deref() == Some("") {
            return Err(McpToolError::new("empty_word", "word is empty; pass 1..=16 letters or omit it"));
        }
        Ok(PingOutput { echoed: String::new() })
    }
}
`,
  "tool_capabilities.rs": `
//! | 6 | commit_insert_mask_none | Commit | op_not_allowed | no |
/// Permission card for one door tool: what it may write.
pub enum CapabilityCard {
    /// Read/compute tools: admitted, no capability spend.
    Observe,
    /// Mask, scope, budget, then EXPLAIN.
    Commit,
    /// Refused, fail closed.
    Deny,
}
pub fn card_for_tool(tool: &str) -> CapabilityCard {
    match tool {
        "tx.apply" => CapabilityCard::Commit,
        _ => CapabilityCard::Observe,
    }
}
`,
};

const files = (overrides = {}) => new Map(Object.entries({ ...FIXTURE, ...overrides }));
const verbOf = (model, name) => [...model.core, ...model.packs.flatMap((p) => p.verbs)].find((v) => v.name === name);

test("a fixture verb reaches the page with scope, arguments, range, default and refusal text", () => {
  const model = buildModel(files());
  assert.deepEqual(model.unparsed, []);
  assert.deepEqual(model.packs.map((p) => p.name), ["demo"]);
  const stack = verbOf(model, "crate.stack");
  assert.ok(stack, "crate.stack parsed");
  assert.equal(stack.card, "commit");
  assert.equal(stack.readonly, false);
  assert.deepEqual(stack.args.map((a) => a.name), ["height", "label"]);
  assert.equal(stack.args[0].default, "3");
  assert.equal(stack.args[0].optional, true, "struct-level serde(default) makes every field optional");
  const page = renderPage(model);
  assert.match(page, /### `crate\.stack`/);
  assert.match(page, /\| `height` \| `u32` \| no \| `3` \| `1\.\.=12`/, "the range is quoted from the description and the refusal");
  assert.match(page, /\| `bad_args` \| height \{\} must be 1\.\.=12 crates; split the stack \|/, "a const in a refusal is filled in");
  assert.match(page, /\| `label_too_long` \| label holds more than 9 bytes; shorten it, then re-run \|/);
  assert.match(page, /\*\*Undo:\*\* description: "Undoable with crate\.unstack\."/);
  assert.match(page, /Listed in a fresh session: no, call `api\.load` with `demo`/);
  assert.match(page, /`observe`: Read\/compute tools: admitted, no capability spend/, "card meanings are read from the enum docs");
});

test("a core tool is read from its impl and its handler method", () => {
  const model = buildModel(files());
  const ping = verbOf(model, "ping");
  assert.ok(ping);
  assert.equal(ping.readonly, true);
  assert.equal(ping.description, "Echo one word. Read-only.");
  assert.deepEqual(ping.refusals, [{ code: "empty_word", message: "word is empty; pass 1..=16 letters or omit it" }]);
  const page = renderPage(model);
  assert.match(page, /\| `word` \| `Option<String>` \| no \|  \| `1\.\.=16` \|/);
  assert.match(page, /\*\*Undo:\*\* none: read-only/);
});

test("a feature-gated verb is marked and a verb inside cfg(test) is not listed", () => {
  const model = buildModel(files());
  const weigh = verbOf(model, "crate.weigh");
  assert.deepEqual(weigh.gates, ["scales"]);
  assert.match(renderPage(model), /cargo feature `scales`/);
  assert.equal(verbOf(model, "ghost.verb"), undefined);
});

test("a helper that wraps the caller's message is a refusal at every call, and a run-time message says so", () => {
  const weigh = verbOf(buildModel(files()), "crate.weigh");
  assert.deepEqual(weigh.refusals, [
    { code: "bad_weight", message: "grams {} must be at most 9000; weigh a lighter crate" },
    { code: "bad_args", message: "(built at run time: e.to_string())" },
  ]);
  assert.deepEqual(rangesFor({ name: "grams" }, [weigh.refusals[0].message], []), ["at most 9000"]);
});

test("a changed argument range changes the page", () => {
  const before = renderPage(buildModel(files()));
  const demo = FIXTURE["packs/demo.rs"];
  const after = renderPage(buildModel(files({ "packs/demo.rs": demo.replace("MAX_CRATES: u32 = 12", "MAX_CRATES: u32 = 20").replace("height 1..=12", "height 1..=20") })));
  assert.notEqual(before, after);
  assert.match(after, /`1\.\.=20`/);
  assert.doesNotMatch(after, /`1\.\.=12`/);
});

test("a new refusal and a renamed argument change the page", () => {
  const base = renderPage(buildModel(files()));
  const demo = FIXTURE["packs/demo.rs"];
  const refused = renderPage(buildModel(files({ "packs/demo.rs": demo.replace("shorten it, then re-run", "shorten it") })));
  assert.notEqual(base, refused);
  const renamed = renderPage(buildModel(files({ "packs/demo.rs": demo.replaceAll("label", "tag") })));
  assert.match(renamed, /\| `tag` \|/);
  assert.doesNotMatch(renamed, /\| `label` \|/);
});

test("the page text is deterministic and escapes html outside code spans only", () => {
  const demo = FIXTURE["packs/demo.rs"].replace("shorten it, then re-run", "use `<tag>` or <b>bold</b>");
  const page = renderPage(buildModel(files({ "packs/demo.rs": demo })));
  assert.equal(page, renderPage(buildModel(files({ "packs/demo.rs": demo }))));
  assert.ok(page.includes("`<tag>`"), "a code span keeps its angle brackets");
  assert.ok(page.includes("&lt;b&gt;bold&lt;/b&gt;"), "prose is escaped");
});

test("rangesFor reads only the text after the name and stops at the next argument", () => {
  const f = (name) => ({ name });
  const texts = ["bays 1..=16, stories 1..=8, knockouts 0..=8"];
  assert.deepEqual(rangesFor(f("bays"), texts, ["stories", "knockouts"]), ["1..=16"]);
  assert.deepEqual(rangesFor(f("stories"), texts, ["bays", "knockouts"]), ["1..=8"]);
  assert.deepEqual(rangesFor(f("wants"), ["wants holds 3 entries; at most 16 per placement"], []), ["at most 16"]);
  assert.deepEqual(rangesFor(f("seed"), texts, []), []);
});

test("the real source: every registered verb is on the page and nothing is left unparsed", () => {
  const model = buildModel(readCrate(ROOT));
  assert.deepEqual(model.unparsed, [], "the parser reads every declaration; teach it the new style or add a case here");
  let registered = 0;
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".rs") && e.name !== "mod.rs") registered += [...readFileSync(p, "utf8").matchAll(/^\s*pack\.add\(\s*VerbSpec::of::</gm)].length;
    }
  };
  walk(join(ROOT, SRC_DIR, "packs"));
  assert.equal(model.counts.verbs, registered, "verbs on the page equal the pack.add(VerbSpec::of::<..>) lines in the pack files");
  assert.equal(model.counts.core, 13, "CORE_TOOL_NAMES holds thirteen tools");
  assert.equal(model.counts.packs, 14, "PACK_NAMES holds fourteen packs");
  for (const v of [...model.core, ...model.packs.flatMap((p) => p.verbs)]) {
    assert.ok(v.description.length > 10, `${v.name} has a description`);
    assert.ok(v.args !== null, `${v.name}: the argument struct ${v.argsType} was found`);
    assert.ok(["observe", "propose", "commit", "revert"].includes(v.card), `${v.name} has a card`);
  }
  assert.equal(verbOf(model, "tx.undo").card, "revert");
  assert.equal(verbOf(model, "scene.place").card, "commit");
  assert.ok(verbOf(model, "tx.apply").refusals.some((r) => r.code === "stale_revision"), "tx.apply names stale_revision");
});

test("the checked-in docs/book/door-reference.md equals the generated page", () => {
  const have = readFileSync(join(ROOT, PAGE_PATH), "utf8").replace(/\r\n/g, "\n");
  const want = generate(ROOT);
  const a = have.split("\n");
  const b = want.split("\n");
  const at = a.findIndex((line, i) => line !== b[i]);
  assert.ok(have === want, `${PAGE_PATH} is stale (first difference at line ${at + 1}: have ${JSON.stringify(a[at])}, want ${JSON.stringify(b[at])}); run: node tools/door-reference.mjs --write`);
});

test("the command line: --check passes on the real page, an unknown flag exits 2", () => {
  const ok = spawnSync(process.execPath, [join(ROOT, "tools", "door-reference.mjs"), "--check"], { cwd: ROOT, encoding: "utf8" });
  assert.equal(ok.status, 0, ok.stderr);
  const bad = spawnSync(process.execPath, [join(ROOT, "tools", "door-reference.mjs"), "--nope"], { cwd: ROOT, encoding: "utf8" });
  assert.equal(bad.status, 2);
  assert.match(bad.stderr, /unknown argument --nope/);
});
