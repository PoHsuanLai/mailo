// Microsoft Graph's change notifications, passed on to mailo over a WebSocket.
//
// Graph posts notifications only to a public HTTPS address, which a desktop app has not got.
// This is that address. mailo registers a key here, subscribes at Graph with
// `<relay>/graph/notify/<key>` as the address, and listens on `<relay>/graph/listen/<key>`. One
// Durable Object per key holds what the key was registered with (two SHA-256 hashes), the
// sockets listening, and whether an alert went unheard.
//
//   POST /graph/register              {key, clientStateHash, listenTokenHash}: 201, or 409 if
//                                     the key is taken
//   POST /graph/notify/<key>          Graph's: the validation handshake (`?validationToken=`),
//                                     answered with the token as text/plain; or notifications,
//                                     whose clientState is checked and whose ids alone go on.
//                                     Always 202 for those.
//   GET  /graph/listen/<key>          WebSocket upgrade, `Authorization: Bearer <listen token>`
//
// What reaches the socket is one JSON frame per notification, ids only, never content:
//
//   {"type":"change","subscriptionId":…,"changeType":"created","resource":…}
//   {"type":"lifecycle","subscriptionId":…,"lifecycleEvent":"subscriptionRemoved"}
//   {"type":"missed"}           on connecting, when an alert came while nobody listened
//
// and "pong" to a "ping", answered without waking the object.

import { DurableObject } from "cloudflare:workers";

export interface GraphEnv {
  GRAPH_RELAY: DurableObjectNamespace<GraphRelay>;
}

/// A key: what mailo makes, 32 random bytes in base64url, with room either side.
const KEY = /^[A-Za-z0-9_-]{16,128}$/;
/// A SHA-256, lowercase hex.
const HASH = /^[0-9a-f]{64}$/;
/// The most a notification post is read to. Graph batches, but ids are small.
const MAX_BODY = 1024 * 1024;
/// A key nobody has listened on or been notified for this long is forgotten. mailo registers
/// again when the relay no longer knows its key.
const FORGET_AFTER_MS = 30 * 24 * 60 * 60 * 1000;

/// What the Worker hands the Durable Object of a notification: the fields it needs, as strings.
export interface Notified {
  subscriptionId: string;
  clientState: string;
  changeType?: string;
  resource?: string;
  lifecycleEvent?: string;
}

/// What the Durable Object keeps for its key.
interface Registration {
  clientStateHash: string;
  listenTokenHash: string;
}

export async function handleGraph(
  request: Request,
  env: GraphEnv,
  _ctx: ExecutionContext,
  url: URL,
): Promise<Response> {
  const parts = url.pathname.split("/").filter((part) => part !== "");
  const [, action, key, ...rest] = parts;
  if (rest.length > 0) {
    return notFound();
  }
  if (action === "register" && key === undefined) {
    return request.method === "POST" ? register(request, env) : notAllowed();
  }
  if (key === undefined || !KEY.test(key)) {
    return notFound();
  }
  if (action === "notify") {
    return request.method === "POST" ? notify(request, env, key, url) : notAllowed();
  }
  if (action === "listen") {
    if (request.method !== "GET") {
      return notAllowed();
    }
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("a WebSocket upgrade is expected\n", { status: 426 });
    }
    return relayFor(env, key).fetch(request);
  }
  return notFound();
}

function relayFor(env: GraphEnv, key: string): DurableObjectStub<GraphRelay> {
  return env.GRAPH_RELAY.get(env.GRAPH_RELAY.idFromName(key));
}

function notFound(): Response {
  return new Response("not found\n", { status: 404 });
}

function notAllowed(): Response {
  return new Response("method not allowed\n", { status: 405 });
}

async function register(request: Request, env: GraphEnv): Promise<Response> {
  let asked: unknown;
  try {
    asked = await request.json();
  } catch {
    return new Response("a JSON body is expected\n", { status: 400 });
  }
  const { key, clientStateHash, listenTokenHash } = (asked ?? {}) as Record<string, unknown>;
  if (
    typeof key !== "string" ||
    !KEY.test(key) ||
    typeof clientStateHash !== "string" ||
    !HASH.test(clientStateHash) ||
    typeof listenTokenHash !== "string" ||
    !HASH.test(listenTokenHash)
  ) {
    return new Response("key, clientStateHash and listenTokenHash are expected\n", {
      status: 400,
    });
  }
  const taken = await relayFor(env, key).register({ clientStateHash, listenTokenHash });
  return taken
    ? new Response("that key is taken\n", { status: 409 })
    : new Response("registered\n", { status: 201 });
}

async function notify(request: Request, env: GraphEnv, key: string, url: URL): Promise<Response> {
  // Graph checks the address when a subscription is made: the token back, as text, at once.
  const token = url.searchParams.get("validationToken");
  if (token !== null) {
    return new Response(token, {
      status: 200,
      headers: { "Content-Type": "text/plain; charset=utf-8" },
    });
  }
  const items = await notifications(request);
  if (items.length > 0) {
    try {
      await relayFor(env, key).notified(items);
    } catch {
      // Graph is answered all the same: a retry would meet the same object.
    }
  }
  return new Response(null, { status: 202 });
}

/// The notifications in a post, each reduced to what is passed on. Anything else in them,
/// resource data included, is dropped here.
async function notifications(request: Request): Promise<Notified[]> {
  const declared = Number(request.headers.get("Content-Length") ?? "0");
  if (declared > MAX_BODY) {
    return [];
  }
  let body: unknown;
  try {
    const text = await request.text();
    if (text.length > MAX_BODY) {
      return [];
    }
    body = JSON.parse(text);
  } catch {
    return [];
  }
  const value = (body as { value?: unknown } | null)?.value;
  if (!Array.isArray(value)) {
    return [];
  }
  const out: Notified[] = [];
  for (const item of value.slice(0, 1000)) {
    if (item === null || typeof item !== "object") {
      continue;
    }
    const { subscriptionId, clientState, changeType, resource, lifecycleEvent } = item as Record<
      string,
      unknown
    >;
    if (typeof subscriptionId !== "string" || typeof clientState !== "string") {
      continue;
    }
    const one: Notified = { subscriptionId, clientState };
    if (typeof lifecycleEvent === "string") {
      one.lifecycleEvent = lifecycleEvent;
    } else {
      if (typeof changeType === "string") one.changeType = changeType;
      if (typeof resource === "string") one.resource = resource;
    }
    out.push(one);
  }
  return out;
}

/// SHA-256 of `text`, lowercase hex.
export async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/// Whether two hex digests are the same, in time that does not depend on where they differ.
function same(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  let differ = 0;
  for (let i = 0; i < a.length; i++) {
    differ |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return differ === 0;
}

/// One key's relay.
export class GraphRelay extends DurableObject<GraphEnv> {
  constructor(ctx: DurableObjectState, env: GraphEnv) {
    super(ctx, env);
    // Answered by Cloudflare without waking the object, so a quiet socket costs nothing.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  /// Register the key with these hashes. `true` when it was registered already: a key is not
  /// taken over.
  async register(registration: Registration): Promise<boolean> {
    if ((await this.ctx.storage.get<Registration>("registration")) !== undefined) {
      return true;
    }
    await this.ctx.storage.put("registration", registration);
    await this.touch();
    return false;
  }

  /// Pass on the notifications whose clientState is the one registered.
  async notified(items: Notified[]): Promise<void> {
    const registration = await this.ctx.storage.get<Registration>("registration");
    if (registration === undefined) {
      return;
    }
    const frames: string[] = [];
    for (const item of items) {
      if (!same(await sha256(item.clientState), registration.clientStateHash)) {
        continue;
      }
      frames.push(
        JSON.stringify(
          item.lifecycleEvent !== undefined
            ? {
                type: "lifecycle",
                subscriptionId: item.subscriptionId,
                lifecycleEvent: item.lifecycleEvent,
              }
            : {
                type: "change",
                subscriptionId: item.subscriptionId,
                changeType: item.changeType,
                resource: item.resource,
              },
        ),
      );
    }
    if (frames.length === 0) {
      return;
    }
    await this.touch();
    let heard = false;
    for (const socket of this.ctx.getWebSockets()) {
      try {
        for (const frame of frames) {
          socket.send(frame);
        }
        heard = true;
      } catch {
        // A socket closing as this is sent: the next connection is told it missed something.
      }
    }
    if (!heard) {
      await this.ctx.storage.put("missed", true);
    }
  }

  /// A WebSocket upgrade from mailo, with the listening token as its bearer.
  async fetch(request: Request): Promise<Response> {
    const registration = await this.ctx.storage.get<Registration>("registration");
    if (registration === undefined) {
      return new Response("unknown key\n", { status: 404 });
    }
    const bearer = request.headers.get("Authorization")?.match(/^Bearer (.+)$/)?.[1];
    if (bearer === undefined || !same(await sha256(bearer), registration.listenTokenHash)) {
      return new Response("not allowed\n", { status: 401 });
    }
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    if ((await this.ctx.storage.get<boolean>("missed")) === true) {
      server.send(JSON.stringify({ type: "missed" }));
      await this.ctx.storage.delete("missed");
    }
    await this.touch();
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(_socket: WebSocket, _message: string | ArrayBuffer): Promise<void> {
    // mailo says nothing but "ping", which the auto-response answers.
  }

  async webSocketClose(socket: WebSocket, code: number, _reason: string): Promise<void> {
    try {
      socket.close(code === 1005 || code === 1006 ? 1000 : code, "closing");
    } catch {
      // Closed already.
    }
  }

  /// Forget the key once it has gone unused for `FORGET_AFTER_MS`.
  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }

  private async touch(): Promise<void> {
    await this.ctx.storage.setAlarm(Date.now() + FORGET_AFTER_MS);
  }
}
