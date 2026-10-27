// A token relay for mailo's remote images, for Cloudflare Workers.
//
//   GET /i?u=<image url>   with   Authorization: Bearer <token>
//
// The Worker fetches the image and returns it, so the image's host sees Cloudflare's address and
// not the reader's. It sends no cookies, no Referer and nothing that names the reader, follows at
// most three redirects, refuses anything but a small raster image over https, and never fetches
// a private or local address. Set the token with `wrangler secret put TOKEN`.

const MAX_BYTES = 16 * 1024 * 1024;
const KINDS = ["image/png", "image/jpeg", "image/gif", "image/webp"];

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method !== "GET" || url.pathname !== "/i") {
      return new Response("not found", { status: 404 });
    }
    if (!env.TOKEN || !same(request.headers.get("authorization") ?? "", `Bearer ${env.TOKEN}`)) {
      return new Response("unauthorized", { status: 401 });
    }
    let target;
    try {
      target = new URL(url.searchParams.get("u") ?? "");
    } catch {
      return new Response("bad url", { status: 400 });
    }
    let hops = 0;
    for (;;) {
      if (!allowed(target)) return new Response("refused", { status: 403 });
      const response = await fetch(target, {
        redirect: "manual",
        headers: { accept: "image/*", "user-agent": "mailo-image-relay" },
        cf: { cacheTtl: 3600 },
      });
      if (response.status >= 300 && response.status < 400 && response.headers.has("location")) {
        if (++hops > 3) return new Response("too many redirects", { status: 502 });
        try {
          target = new URL(response.headers.get("location"), target);
        } catch {
          return new Response("bad redirect", { status: 502 });
        }
        continue;
      }
      if (!response.ok) return new Response("upstream refused", { status: 502 });
      const kind = (response.headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
      if (!KINDS.includes(kind)) return new Response("not an image", { status: 415 });
      const length = Number(response.headers.get("content-length") ?? 0);
      if (length > MAX_BYTES) return new Response("too large", { status: 413 });
      return new Response(limited(response.body), {
        headers: {
          "content-type": kind,
          "cache-control": "private, max-age=3600",
          "x-content-type-options": "nosniff",
        },
      });
    }
  },
};

// Whether an image may be fetched from `url`: https, on a public name or address.
export function allowed(url) {
  if (url.protocol !== "https:" || url.username || url.password) return false;
  // A name may end in the root's dot ("localhost."), which is the same name.
  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
  if (host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local")) return false;
  if (host.endsWith(".internal") || !host.includes(".") && !host.includes(":")) return false;
  const v4 = host.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])];
    return !(
      a === 0 || a === 10 || a === 127 || a >= 224 ||
      (a === 100 && b >= 64 && b < 128) || (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168)
    );
  }
  if (host.includes(":")) return false;
  return true;
}

// A body that stops after MAX_BYTES, whatever its headers said.
function limited(body) {
  let seen = 0;
  return body.pipeThrough(
    new TransformStream({
      transform(chunk, controller) {
        seen += chunk.byteLength;
        if (seen > MAX_BYTES) controller.error(new Error("too large"));
        else controller.enqueue(chunk);
      },
    }),
  );
}

// A comparison that takes as long whatever the two strings share.
function same(a, b) {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}
