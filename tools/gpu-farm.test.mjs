// tools/gpu-farm.test.mjs: the pure parts of the GPU farm driver.
import test from "node:test";
import assert from "node:assert/strict";
import { cargoLine, kernelScript, kernelMetadata, receiptVerdict } from "./gpu-farm.mjs";

test("the cargo line comes from the crate and the filter", () => {
  assert.equal(
    cargoLine({ crate: "engine-render", filter: "auto_exposure" }),
    "cargo test -p engine-render --lib auto_exposure",
  );
  assert.equal(cargoLine({ crate: "engine-render", filter: "" }), "cargo test -p engine-render --lib");
  assert.equal(cargoLine({ crate: "", filter: "" }), "", "no crate, no line");
});

test("an explicit line wins over the crate and the filter", () => {
  const line = "cargo test -p engine-render --features window --lib contact_shadow";
  assert.equal(cargoLine({ crate: "engine-render", filter: "auto_exposure", line }), line);
});

test("the kernel script runs one plain line and reports the exit code", () => {
  const script = kernelScript({
    repo: "https://github.com/maximkone/engine2040-public",
    ref: "master",
    line: "cargo test -p engine-render --lib auto_exposure",
  });
  assert.ok(script.includes("cargo test -p engine-render --lib auto_exposure"), "the line is in the kernel");
  assert.ok(script.includes("set -uo pipefail"), "no errexit, so the receipt still lands");
  assert.ok(script.includes("echo \"--- exit: $CODE ---\""), "the exit code is in the receipt");
  assert.ok(script.includes("CARGO_INCREMENTAL=0"), "no stale incremental link");
  assert.ok(script.includes("libvulkan1"), "the graphics libraries are installed");
  assert.ok(script.includes("nvidia-smi"), "the GPU is named in the receipt");
});

test("the kernel metadata asks for a GPU and stays private", () => {
  const meta = kernelMetadata({ name: "gs44-meter", line: "cargo test -p engine-render --lib auto_exposure" });
  assert.equal(meta.enable_gpu, true, "the kernel runs on a T4");
  assert.equal(meta.is_private, true, "the engine tree is not public on Kaggle");
  assert.equal(meta.id, "maximkone/engine2040-gpu-farm-gs44-meter");
  assert.equal(meta.code_file, "run.sh");
});

test("the receipt verdict reads the exit line of the kernel", () => {
  assert.equal(receiptVerdict(""), "no-receipt");
  assert.equal(receiptVerdict("cargo test ... ok\n--- exit: 0 ---"), "pass");
  assert.equal(receiptVerdict("error: could not compile\n--- exit: 101 ---"), "fail");
  assert.equal(receiptVerdict("still running"), "incomplete");
});
