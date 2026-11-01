import type { NextConfig } from "next";
import { PHASE_PRODUCTION_SERVER } from "next/constants";
import { resolveApiMode } from "./lib/api/mode";
import { authMode } from "./lib/auth/config";

// Mirrors web/next.config.ts.
const nextConfig: NextConfig = {
  cacheComponents: true,
  partialPrefetching: true,
  poweredByHeader: false,
  turbopack: {
    rules: {
      "*.css": {
        loaders: ["@tailwindcss/turbopack"],
        as: "*.css",
      },
    },
  },
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "Content-Security-Policy", value: "frame-ancestors 'none'" },
          { key: "X-Frame-Options", value: "DENY" },
          { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
          { key: "X-Content-Type-Options", value: "nosniff" },
        ],
      },
    ];
  },
};

// `next start` evaluates this file at boot with PHASE_PRODUCTION_SERVER: a
// missing or partial WorkOS config, or no data source, stops the server
// before it listens (fail closed). `next build` and `next dev` are not
// checked here; the proxy still refuses every request when misconfigured.
export default function config(phase: string): NextConfig {
  if (phase === PHASE_PRODUCTION_SERVER) {
    authMode(); // throws AuthConfigError
    const data = resolveApiMode(process.env);
    if (data.kind === "misconfigured") throw new Error(`Quod Cloud data source is not configured. ${data.problem}`);
  }
  return nextConfig;
}
