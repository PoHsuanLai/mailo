import type { Env } from "../src/index";

declare global {
  namespace Cloudflare {
    // Spelled out: an `extends` of src/index's Env left this empty to the type checker.
    interface Env {
      GRAPH_RELAY: DurableObjectNamespace<import("../src/graph").GraphRelay>;
      PATH_SECRET?: string;
    }
  }
}

export type { Env };
