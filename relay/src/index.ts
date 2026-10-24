// mailo's relay. Each route group is a module of its own under `src/`: `/graph/*` is Microsoft
// Graph's push (`graph.ts`). Others (an image relay) join beside it.

import { type GraphEnv, handleGraph } from "./graph";

export { GraphRelay } from "./graph";

export type Env = GraphEnv;

export default {
  async fetch(request, env, ctx): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname.startsWith("/graph/")) {
      return handleGraph(request, env, ctx, url);
    }
    return new Response("not found\n", { status: 404 });
  },
} satisfies ExportedHandler<Env>;
