// The Graph relay, run inside workerd by the Workers Vitest integration: nothing is deployed and
// no Cloudflare account is asked for. Graph's side is played by posting what Graph posts; mailo's
// by registering and opening the WebSocket as it does.

import { SELF, createExecutionContext } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { sha256 } from "../src/graph";
import worker from "../src/index";

const HOST = "https://relay.example.test";
/// The relay's address as mailo is given it: the host and the path secret (vitest.config.ts).
const BASE = `${HOST}/test-path-secret`;

let counter = 0;
/// A fresh key, secret and clientState for each test, registered.
async function registered(): Promise<{ key: string; secret: string; state: string }> {
  counter += 1;
  const key = `test-key-${counter}-${crypto.randomUUID()}`;
  const secret = `secret-${crypto.randomUUID()}`;
  const state = `state-${crypto.randomUUID()}`;
  const response = await SELF.fetch(`${BASE}/graph/register`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      key,
      clientStateHash: await sha256(state),
      listenTokenHash: await sha256(secret),
    }),
  });
  expect(response.status).toBe(201);
  return { key, secret, state };
}

/// mailo's socket, and the frames it has heard.
async function listen(key: string, secret: string): Promise<{ socket: WebSocket; frames: string[] }> {
  const response = await SELF.fetch(`${BASE}/graph/listen/${key}`, {
    headers: { Upgrade: "websocket", Authorization: `Bearer ${secret}` },
  });
  expect(response.status).toBe(101);
  const socket = response.webSocket;
  if (socket === null) {
    throw new Error("no socket");
  }
  const frames: string[] = [];
  socket.addEventListener("message", (event) => {
    frames.push(String(event.data));
  });
  socket.accept();
  return { socket, frames };
}

async function heard(frames: string[], n: number): Promise<string[]> {
  for (let i = 0; i < 100 && frames.length < n; i++) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return frames;
}

function post(key: string, value: unknown[]): Promise<Response> {
  return SELF.fetch(`${BASE}/graph/notify/${key}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ value }),
  });
}

describe("the Graph relay", () => {
  it("answers Graph's validation with the token, as text", async () => {
    const token = "Validation: Token with spaces & symbols";
    const response = await SELF.fetch(
      `${BASE}/graph/notify/some-key-0123456789?validationToken=${encodeURIComponent(token)}`,
      { method: "POST" },
    );
    expect(response.status).toBe(200);
    expect(response.headers.get("Content-Type")).toMatch(/^text\/plain/);
    expect(response.headers.get("X-Content-Type-Options")).toBe("nosniff");
    expect(await response.text()).toBe(token);
  });

  it("registers a key once and refuses to hand it over", async () => {
    const { key } = await registered();
    const again = await SELF.fetch(`${BASE}/graph/register`, {
      method: "POST",
      body: JSON.stringify({
        key,
        clientStateHash: await sha256("other"),
        listenTokenHash: await sha256("other"),
      }),
    });
    expect(again.status).toBe(409);
    const bad = await SELF.fetch(`${BASE}/graph/register`, {
      method: "POST",
      body: JSON.stringify({ key: "short", clientStateHash: "x", listenTokenHash: "y" }),
    });
    expect(bad.status).toBe(400);
  });

  it("lets only the registered secret listen", async () => {
    const { key } = await registered();
    const without = await SELF.fetch(`${BASE}/graph/listen/${key}`, {
      headers: { Upgrade: "websocket" },
    });
    expect(without.status).toBe(401);
    const wrong = await SELF.fetch(`${BASE}/graph/listen/${key}`, {
      headers: { Upgrade: "websocket", Authorization: "Bearer not-it" },
    });
    expect(wrong.status).toBe(401);
    const unknown = await SELF.fetch(`${BASE}/graph/listen/never-registered-key-123`, {
      headers: { Upgrade: "websocket", Authorization: "Bearer anything" },
    });
    expect(unknown.status).toBe(404);
    const plain = await SELF.fetch(`${BASE}/graph/listen/${key}`);
    expect(plain.status).toBe(426);
  });

  it("passes on ids only, and only with the registered clientState", async () => {
    const { key, secret, state } = await registered();
    const { socket, frames } = await listen(key, secret);
    const forged = await post(key, [
      { subscriptionId: "sub-1", clientState: "forged", changeType: "created", resource: "x" },
    ]);
    expect(forged.status).toBe(202);
    const real = await post(key, [
      {
        subscriptionId: "sub-1",
        clientState: state,
        changeType: "created",
        resource: "Users('u')/Messages('m')",
        resourceData: { subject: "never passed on" },
        tenantId: "t",
      },
    ]);
    expect(real.status).toBe(202);
    await heard(frames, 1);
    expect(frames.map((frame) => JSON.parse(frame))).toEqual([
      {
        type: "change",
        subscriptionId: "sub-1",
        changeType: "created",
        resource: "Users('u')/Messages('m')",
      },
    ]);
    socket.close(1000);
  });

  it("passes on lifecycle notifications", async () => {
    const { key, secret, state } = await registered();
    const { socket, frames } = await listen(key, secret);
    for (const lifecycleEvent of ["reauthorizationRequired", "subscriptionRemoved", "missed"]) {
      await post(key, [{ subscriptionId: "sub-2", clientState: state, lifecycleEvent }]);
    }
    await heard(frames, 3);
    expect(frames.map((frame) => JSON.parse(frame).lifecycleEvent)).toEqual([
      "reauthorizationRequired",
      "subscriptionRemoved",
      "missed",
    ]);
    expect(JSON.parse(frames[0])).toEqual({
      type: "lifecycle",
      subscriptionId: "sub-2",
      lifecycleEvent: "reauthorizationRequired",
    });
    socket.close(1000);
  });

  it("tells a listener what it missed while nobody listened", async () => {
    const { key, secret, state } = await registered();
    await post(key, [
      { subscriptionId: "sub-3", clientState: state, changeType: "created", resource: "r" },
    ]);
    const { socket, frames } = await listen(key, secret);
    await heard(frames, 1);
    expect(frames.map((frame) => JSON.parse(frame))).toEqual([{ type: "missed" }]);
    socket.close(1000);
    // Said once.
    const second = await listen(key, secret);
    second.socket.send("ping");
    await heard(second.frames, 1);
    expect(second.frames).toEqual(["pong"]);
    second.socket.close(1000);
  });

  it("answers Graph with 202 whatever it posts", async () => {
    expect((await post("never-registered-key-456", [{ subscriptionId: "s", clientState: "c" }])).status).toBe(202);
    const junk = await SELF.fetch(`${BASE}/graph/notify/never-registered-key-456`, {
      method: "POST",
      body: "not json",
    });
    expect(junk.status).toBe(202);
    expect((await SELF.fetch(`${BASE}/graph/notify/bad`, { method: "POST" })).status).toBe(404);
    expect((await SELF.fetch(`${BASE}/elsewhere`)).status).toBe(404);
  });

  it("is reached only under the path secret", async () => {
    const asked = JSON.stringify({
      key: `outside-key-${crypto.randomUUID()}`,
      clientStateHash: await sha256("s"),
      listenTokenHash: await sha256("t"),
    });
    for (const base of [HOST, `${HOST}/wrong-path-secret`, `${HOST}/test-path-secre`]) {
      const register = await SELF.fetch(`${base}/graph/register`, { method: "POST", body: asked });
      expect(register.status).toBe(404);
      const echo = await SELF.fetch(`${base}/graph/notify/some-key-0123456789?validationToken=t`, {
        method: "POST",
      });
      expect(echo.status).toBe(404);
    }
    // With no secret set, nothing is reached, under any path.
    const unset = await worker.fetch(
      new Request(`${BASE}/graph/register`, { method: "POST", body: asked }) as Parameters<
        typeof worker.fetch
      >[0],
      { GRAPH_RELAY: env.GRAPH_RELAY },
      createExecutionContext(),
    );
    expect(unset.status).toBe(404);
  });
});
