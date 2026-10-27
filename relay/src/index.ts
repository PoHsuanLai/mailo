// mailo's relay. Each route group is a module of its own under `src/`: `/graph/*` is Microsoft
// Graph's push (`graph.ts`). Others (an image relay) join beside it.
//
// Every route sits under `/<PATH_SECRET>/`, a Worker secret (`wrangler secret put PATH_SECRET`),
// so only someone given the relay's whole address can register a key or reach anything else.
// Any other path, and every path while the secret is unset, is a 404.

import { type GraphEnv, handleGraph, same } from "./graph";

export { GraphRelay } from "./graph";

export interface Env extends GraphEnv {
  PATH_SECRET?: string;
}

export default {
  async fetch(request, env, ctx): Promise<Response> {
    const url = new URL(request.url);
    const inner = within(url, env.PATH_SECRET);
    if (inner?.pathname.startsWith("/graph/")) {
      return handleGraph(request, env, ctx, inner);
    }
    return new Response("not found\n", { status: 404 });
  },
} satisfies ExportedHandler<Env>;

/// `url` with its `/<secret>` prefix taken off, or `undefined` when it has not got that prefix
/// or no secret is set.
function within(url: URL, secret: string | undefined): URL | undefined {
  if (secret === undefined || secret === "") {
    return undefined;
  }
  const [, first, ...rest] = url.pathname.split("/");
  if (first === undefined || !same(first, secret)) {
    return undefined;
  }
  const inner = new URL(url);
  inner.pathname = `/${rest.join("/")}`;
  return inner;
}
