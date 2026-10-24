import type { Env } from "../src/index";

declare global {
  namespace Cloudflare {
    interface Env extends import("../src/index").Env {}
  }
}

export type { Env };
