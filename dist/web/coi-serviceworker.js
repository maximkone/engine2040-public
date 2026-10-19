// WEB14: cross-origin isolation without server headers. GitHub Pages cannot send COOP/COEP, so the
// page registers this worker, waits until it controls the page, then reloads once; this worker puts
// the two headers on every response it hands back, which is what makes `crossOriginIsolated` true
// and SharedArrayBuffer legal. Idea from Godot's PWA export (MIT) and gzuidhof/coi-serviceworker
// (MIT); this is the engine's own small version, no dependency and no build step.

// Take over straight away: the page's next reload must already be controlled.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

const COOP = "same-origin";
const COEP = "require-corp";

self.addEventListener("fetch", (event) => {
  const request = event.request;
  // A cached-only request the page did not make itself has no response to restamp.
  if (request.cache === "only-if-cached" && request.mode !== "same-origin") return;
  event.respondWith(
    fetch(request)
      .then((response) => {
        // An opaque answer (a cross-origin `no-cors` fetch) cannot be restamped: hand it back as is.
        if (response.status === 0) return response;
        const headers = new Headers(response.headers);
        headers.set("Cross-Origin-Opener-Policy", COOP);
        headers.set("Cross-Origin-Embedder-Policy", COEP);
        return new Response(response.body, {
          status: response.status,
          statusText: response.statusText,
          headers,
        });
      })
      .catch((error) => console.error("coi-serviceworker: fetch failed", error))
  );
});
