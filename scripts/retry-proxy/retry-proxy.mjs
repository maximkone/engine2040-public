// Singleton guard: engine2040 NEVER runs its own proxy. It reuses the forge
// proxy on 127.0.0.1:13579. Full implementation: C:/9router-free/retry-proxy/retry-proxy.mjs
import net from "node:net";
const raw = Number(process.env.RETRY_PROXY_PORT);
const PORT = Number.isInteger(raw) && raw > 0 && raw < 65536 ? raw : 13579;
const fail = () => { console.error(`[retry-proxy] no proxy on :${PORT} - start it first: node C:/9router-free/retry-proxy/retry-proxy.mjs`); process.exit(1); };
const sock = net.connect(PORT, "127.0.0.1");
sock.setTimeout(3000);
sock.on("timeout", fail);
sock.on("connect", () => { console.log(`[retry-proxy] reusing live proxy on :${PORT}, nothing to start`); sock.end(); process.exit(0); });
sock.on("error", fail);
