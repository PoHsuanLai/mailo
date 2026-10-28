// The image relay's rules, run under Node: `npm test` in relay/image. Nothing is deployed and
// nothing is fetched; a request the Worker refuses is refused before it would fetch.

import assert from "node:assert/strict";
import { test } from "node:test";
import worker, { allowed } from "./worker.js";

const refused = [
  // Not https, or carrying credentials.
  "http://images.example.com/a.png",
  "ftp://images.example.com/a.png",
  "https://user:secret@images.example.com/a.png",
  "https://user@images.example.com/a.png",
  // Local names, with or without the root's dot.
  "https://localhost/a.png",
  "https://localhost./a.png",
  "https://app.localhost/a.png",
  "https://printer.local/a.png",
  "https://printer.local./a.png",
  "https://db.internal/a.png",
  "https://intranet/a.png",
  // Private, loopback, link-local and reserved IPv4, as the URL parser reads each spelling.
  "https://127.0.0.1/a.png",
  "https://2130706433/a.png",
  "https://0x7f000001/a.png",
  "https://0x7f.1/a.png",
  "https://017700000001/a.png",
  "https://127.1/a.png",
  "https://0.0.0.0/a.png",
  "https://10.1.2.3/a.png",
  "https://172.16.0.1/a.png",
  "https://172.31.255.255/a.png",
  "https://192.168.1.1/a.png",
  "https://169.254.169.254/a.png",
  "https://100.64.0.1/a.png",
  "https://224.0.0.1/a.png",
  "https://255.255.255.255/a.png",
  // IPv6 in any spelling.
  "https://[::1]/a.png",
  "https://[::ffff:127.0.0.1]/a.png",
  "https://[fe80::1]/a.png",
  "https://[fd00::1]/a.png",
  "https://[2001:db8::1]/a.png",
];

const allowedUrls = [
  "https://images.example.com/a.png",
  "https://images.example.com./a.png",
  "https://8.8.8.8/a.png",
  "https://172.32.0.1/a.png",
  "https://172.15.0.1/a.png",
  "https://100.63.0.1/a.png",
  "https://192.169.0.1/a.png",
];

test("allowed() refuses what is not a public https address", () => {
  for (const text of refused) {
    assert.equal(allowed(new URL(text)), false, text);
  }
});

test("allowed() admits a public https name or address", () => {
  for (const text of allowedUrls) {
    assert.equal(allowed(new URL(text)), true, text);
  }
});

const env = { TOKEN: "a-long-random-token" };
const ask = (path, headers = {}) =>
  worker.fetch(new Request(`https://relay.example.test${path}`, { headers }), env);
const target = encodeURIComponent("https://127.0.0.1/a.png");

test("a request without the token is refused, and so is a wrong one", async () => {
  assert.equal((await ask(`/i?u=${target}`)).status, 401);
  assert.equal((await ask(`/i?u=${target}`, { authorization: "Bearer wrong" })).status, 401);
  assert.equal(
    (await worker.fetch(new Request(`https://relay.example.test/i?u=${target}`), {})).status,
    401,
    "a Worker with no TOKEN set refuses everyone",
  );
});

test("only GET /i is answered", async () => {
  assert.equal((await ask("/other")).status, 404);
  const post = new Request("https://relay.example.test/i", { method: "POST" });
  assert.equal((await worker.fetch(post, env)).status, 404);
});

test("a private address with the right token is refused before anything is fetched", async () => {
  const authorization = `Bearer ${env.TOKEN}`;
  assert.equal((await ask(`/i?u=${target}`, { authorization })).status, 403);
  assert.equal((await ask("/i?u=not-a-url", { authorization })).status, 400);
});
