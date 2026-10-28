import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { configDefaults, defineConfig } from "vitest/config";

// The tests run inside workerd, through Miniflare: nothing is deployed and no Cloudflare account
// is asked for. PATH_SECRET is the test's own; a deployed relay has it from `wrangler secret put`.
export default defineConfig({
  // image/ is its own package, tested under Node by `npm test` there.
  test: { exclude: [...configDefaults.exclude, "image/**"] },
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: { bindings: { PATH_SECRET: "test-path-secret" } },
    }),
  ],
});
