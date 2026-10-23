// A5: the page's audio sink, the consumer half of the mixer-to-worklet ring.
//
// The page renders a block of interleaved stereo with engine-audio's own mixer and posts each
// block here as two Float32Arrays (transferred, not copied). This processor is the audio-thread
// consumer: it copies the frames the output quantum needs out of the oldest block and hands the
// empty arrays back to the page, which refills them and posts them again. When a quantum arrives
// and no queued block has frames left, the processor writes silence and counts one underrun.
//
// Why postMessage and not SharedArrayBuffer: a shared ring needs cross-origin isolation (COOP and
// COEP) and an atomics wasm build. The page deliberately runs on a plain host (WEB14's isolation
// worker is best effort and the wasm module has no atomics), so the ring rides the port. Batches
// are whole blocks, far larger than one 128-frame quantum, so the message rate stays near 50 a
// second and the ring holds a few hundred milliseconds of slack. The processor owns the underrun
// counter because it is the side that starves; the page prints it.
//
// Page -> processor: { type: "block", left: Float32Array, right: Float32Array }
// Processor -> page: { type: "spent", left, right, underruns } (the arrays come back, transferred)
//                    { type: "want", free, underruns } (the ring is low; render up to `free` blocks)
//
// `processorOptions` carries `lowWater` and `maxQueued` from the page (crates/web/src/audio_worklet.rs),
// so the two sides cannot drift apart. This file is inlined into the wasm bundle as a blob module
// by crates/web/src/browser.rs; it is never fetched as a separate file.

const DEFAULT_LOW_WATER = 3;
const DEFAULT_MAX_QUEUED = 6;
/// Ask at most once in this many render quanta, so a starving ring does not spam the page.
const ASK_EVERY = 8;

class Engine2040Sink extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const opts = (options && options.processorOptions) || {};
    this.lowWater = Number.isInteger(opts.lowWater) && opts.lowWater > 0 ? opts.lowWater : DEFAULT_LOW_WATER;
    this.maxQueued = Number.isInteger(opts.maxQueued) && opts.maxQueued > 0 ? opts.maxQueued : DEFAULT_MAX_QUEUED;
    /** Blocks posted by the page and not yet spent, oldest first: { left, right }. */
    this.queued = [];
    /** Frames already consumed from `queued[0]`. */
    this.offset = 0;
    /** Render quanta that found no frames after the mixer had fed the ring at least once. */
    this.underruns = 0;
    /** Blocks the page ever posted. */
    this.received = 0;
    /** Blocks fully spent and returned to the page. */
    this.consumed = 0;
    this.sinceAsk = ASK_EVERY;
    this.port.onmessage = (event) => this.onMessage(event.data);
  }

  onMessage(msg) {
    if (!msg || msg.type !== "block" || !msg.left || !msg.right) return;
    this.queued.push({ left: msg.left, right: msg.right });
    this.received += 1;
    this.maybeAsk();
  }

  maybeAsk() {
    this.sinceAsk += 1;
    if (this.queued.length >= this.lowWater || this.sinceAsk < ASK_EVERY) return;
    this.sinceAsk = 0;
    this.port.postMessage({
      type: "want",
      free: Math.max(0, this.maxQueued - this.queued.length),
      underruns: this.underruns,
    });
  }

  process(_inputs, outputs) {
    const out = outputs[0] || [];
    const left = out[0];
    if (!left) return true;
    const right = out[1] && out[1] !== left ? out[1] : null;
    const quantum = left.length;
    let done = 0;
    while (done < quantum && this.queued.length > 0) {
      const head = this.queued[0];
      const take = Math.min(head.left.length - this.offset, quantum - done);
      left.set(head.left.subarray(this.offset, this.offset + take), done);
      if (right) right.set(head.right.subarray(this.offset, this.offset + take), done);
      this.offset += take;
      done += take;
      if (this.offset >= head.left.length) {
        this.queued.shift();
        this.offset = 0;
        this.consumed += 1;
        this.port.postMessage(
          { type: "spent", left: head.left, right: head.right, underruns: this.underruns },
          [head.left.buffer, head.right.buffer],
        );
      }
    }
    if (done < quantum) {
      left.fill(0, done);
      if (right) right.fill(0, done);
      // Before the first block arrives the ring is simply not primed yet; that is start-up, not
      // a starved consumer, so it is not an underrun. After that, every dry quantum counts.
      if (this.received > 0) this.underruns += 1;
    }
    this.maybeAsk();
    return true;
  }
}

registerProcessor("engine2040-sink", Engine2040Sink);
