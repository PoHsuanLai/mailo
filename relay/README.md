# mailo relay

A small Cloudflare Worker that gives mailo the public HTTPS address a desktop app does not
have. Today it does one thing: it receives Microsoft Graph's change notifications for a
Microsoft 365 account and passes them to mailo over a WebSocket, so new mail arrives at once
instead of at the next poll.

The same code runs on anyone's Cloudflare account. Nothing in it names an account, a tenant or
a domain. mailo knows the relay only by the address you paste into its settings. With no relay
set, or with the relay unreachable, mailo polls Graph as it always has.

## Deploy

You need Node.js 22 or later (Miniflare's floor) and a Cloudflare account. The free plan is
enough, since the Durable Object is SQLite-backed.

```sh
cd relay
npm ci                              # exactly what package-lock.json names
npx wrangler login                  # once, in a browser
secret=$(openssl rand -hex 24); echo "$secret"   # the path secret: keep it for mailo
printf %s "$secret" | npx wrangler secret put PATH_SECRET
npx wrangler deploy
```

`PATH_SECRET` is the relay's path secret: every route sits under `/<PATH_SECRET>/`, and any
other path is answered `404`, as is every path while the secret is unset. A new one is put
with the same two lines, and replaces the old. `wrangler deploy` prints the Worker's
address, for example `https://mailo-relay.<your-subdomain>.workers.dev`. In mailo, open
Settings › Accounts, choose the Microsoft 365 account and paste the address with the secret
after it, `https://mailo-relay.<your-subdomain>.workers.dev/<PATH_SECRET>`, into **Push relay**.
Leave the field empty to go back to polling.

The account's organisation has to allow Graph change notifications (subscriptions) for
mailbox messages. Most tenants do. If Graph refuses the subscription, mailo keeps polling and
tries again later.

To run it locally, put `PATH_SECRET=<anything>` in `relay/.dev.vars` and use `npx wrangler dev`.
mailo takes `http://` only on this computer's loopback, for example
`http://127.0.0.1:8787/<anything>`, and Graph cannot reach a local relay, so a local one only
helps with testing the socket.

## Test

```sh
npm ci
npm test          # vitest inside workerd, through Miniflare: nothing is deployed
npm run check     # TypeScript
npm run licences  # every package's licence on the allow-list (below)
```

CI runs the same four in the `relay` job.

## Licences

The relay's npm tree, development tools included, is held to an allow-list the way `deny.toml`
holds the Rust crates': `scripts/licences.mjs` reads `package-lock.json` and fails on any
licence that names a GPL or is not one of MIT, Apache-2.0, ISC, BSD-2-Clause, BSD-3-Clause,
0BSD, CC0-1.0, MPL-2.0, BlueOak-1.0.0 or Python-2.0.

Miniflare, which the tests and `wrangler dev` run on, depends on `sharp` for its Images
binding, and sharp brings libvips, which is LGPL-3.0. The relay binds no Images, so
`package.json` puts a stub in sharp's place (`stubs/sharp`, through `overrides`). Miniflare
loads sharp only when an Images binding is used, inside a `try`, so nothing reaches the stub,
and no `@img/*` package is installed. The dependencies are pinned to exact versions.

## Endpoints

All of them are under `/<PATH_SECRET>/graph/`, written below without the secret. Other relays,
such as an image relay, are meant to sit beside it under paths of their own (`src/index.ts`
routes, `src/graph.ts` is this one).

| Request | Who sends it | What it does |
|---|---|---|
| `POST /graph/register` with `{key, clientStateHash, listenTokenHash}` | mailo | Creates the key's Durable Object holding the two SHA-256 hashes. Answers `201`, or `409` if the key is already taken. A key is never handed over. |
| `POST /graph/notify/<key>?validationToken=…` | Graph | The validation handshake: the token comes back as `text/plain` with `nosniff`, `200`. |
| `POST /graph/notify/<key>` | Graph | For each notification whose `clientState` hashes to the registered hash, only `{subscriptionId, changeType, resource}` (or `lifecycleEvent`) goes to the key's sockets. Always answers `202`. |
| `GET /graph/listen/<key>` with `Authorization: Bearer <listen token>` | mailo | WebSocket upgrade, accepted only for the token whose hash is registered. Held with the hibernation API, so a quiet socket costs nothing. |

Frames on the socket:

```json
{"type":"change","subscriptionId":"…","changeType":"created","resource":"Users('…')/Messages('…')"}
{"type":"lifecycle","subscriptionId":"…","lifecycleEvent":"subscriptionRemoved"}
{"type":"missed"}
```

`missed` is sent on connecting when a notification came in while nobody was listening. mailo
answers every frame with a sync pass, and it sends `ping` every 30 seconds. Cloudflare answers
with `pong` without waking the object.

A key that nobody listens on and nothing is posted to for 30 days is forgotten. When mailo next
connects, the relay no longer knows the key, so mailo registers a new one and subscribes again.

## Threat model

What the relay holds and sees:

- **Kept:** for each key, the SHA-256 of the listen token and the SHA-256 of the subscription's
  `clientState`, plus a flag for a missed notification. Never the token or the `clientState`
  itself.
- **Passing through:** Graph's notifications for the key. They carry the subscription id, the
  change type, the resource path (the user's and message's opaque ids) and the `clientState`.
  The subscriptions mailo makes never ask for resource data (`includeResourceData` is off), so
  no notification carries a subject, a sender or a body. Anything else in a notification is
  dropped before it reaches the socket.
- **Never:** mail content, Graph access or refresh tokens, or the account's password. mailo
  talks to Graph directly to subscribe, renew and read mail. Graph tokens never touch the relay.

What someone can do:

- **Whoever runs the relay** (you, on your Cloudflare account, and Cloudflare) can see when new
  mail arrives and the opaque ids involved. They can withhold notifications or invent them. A
  forged notification only makes mailo sync, and a withheld one only delays mail until the next
  poll. mailo keeps polling at its usual interval with a relay set, so a relay can make mail
  faster but never slower or wrong.
- **Someone who knows a key** (it is in the notification address Graph holds) can post to it.
  Without the `clientState`, which only Graph and mailo have, those posts are dropped. They can
  also send Graph's validation handshake, which only echoes their own token.
- **Someone without the listen token** cannot open the socket. The token is 32 random bytes,
  kept in the system keyring as one of the account's secrets. The relay compares hashes in
  constant time.
- **Someone without the path secret** reaches nothing: every path outside `/<PATH_SECRET>/` is
  a `404`. The secret is in the address Graph posts to and in mailo's settings, so it keeps out
  scanners and strangers, not Microsoft or someone who can read your settings.
- **Anyone who has the relay's whole address** can register keys. Each one is a tiny Durable
  Object that is forgotten after 30 days unused. If that matters on your account, put the Worker
  behind Cloudflare Access or a WAF rule for `/<PATH_SECRET>/graph/register` as well. Graph's
  posts and mailo's socket do not need it.

The `clientState` is derived from the listen token with a different hash
(`SHA-256("mailo graph clientState\n" + token)`). Neither hash the relay keeps reveals the
other secret, so the relay cannot listen with what Graph sends it.
