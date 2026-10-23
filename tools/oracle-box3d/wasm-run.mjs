// Run the wasm32-wasip1 oracle under Node's WASI host (Node 24, no install).
//
// Usage: node wasm-run.mjs <oracle.wasm> [oracle args...]
//
// The point of this leg: the same scene, the same digest code, compiled for
// wasm32. Any floating point difference between the hosts shows up as a
// different digest, which is exactly what the spike measures.
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";

const [wasmPath, ...oracleArgs] = process.argv.slice(2);
if (!wasmPath) {
  console.error("usage: node wasm-run.mjs <oracle.wasm> [oracle args...]");
  process.exit(2);
}

const wasi = new WASI({
  version: "preview1",
  args: ["box3d-oracle", ...oracleArgs],
});

const bytes = await readFile(wasmPath);
const { instance } = await WebAssembly.instantiate(bytes, wasi.getImportObject());
wasi.start(instance);
