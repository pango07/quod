// Which adapter serves Quod's data in this process. Pure, so it can be tested.
//
// QUOD_API is the switch (GitHub's data never depends on it):
// - unset: no Quod server. Teams see their GitHub CI, and Quod's numbers say
//   "Appears once Quod runs". Safe on any deploy.
// - "http": quod-server at QUOD_SERVER_URL (quod.v1). https only, except
//   localhost and private `.internal` hosts (Railway's private network).
// - "fixture": the built-in sample data, only ever by that explicit value
//   (cloud/.env.development sets it for `next dev`); real teams stay empty.
// Anything else is misconfigured: `next start` refuses to boot (next.config.ts).

import { parseServerTeams } from "./credentials";

export type ApiMode = { kind: "http"; url: string } | { kind: "fixture" } | { kind: "none" } | { kind: "misconfigured"; problem: string };

const PLAIN_HTTP_OK = /^(localhost|127\.0\.0\.1|\[::1\]|[a-z0-9-]+(\.[a-z0-9-]+)*\.internal)$/i;

export function resolveApiMode(env: Partial<Record<string, string | undefined>>): ApiMode {
  const mode = env.QUOD_API?.trim() ?? "";
  if (mode === "fixture") return { kind: "fixture" };
  if (mode === "http") {
    const raw = env.QUOD_SERVER_URL?.trim();
    if (!raw) return { kind: "misconfigured", problem: "QUOD_API=http needs QUOD_SERVER_URL (quod-server's base URL)." };
    let url: URL;
    try {
      url = new URL(raw);
    } catch {
      return { kind: "misconfigured", problem: "QUOD_SERVER_URL is not a URL." };
    }
    if (url.username || url.password || url.search || url.hash) return { kind: "misconfigured", problem: "QUOD_SERVER_URL must be a plain base URL (no credentials, query or fragment)." };
    if (url.protocol !== "https:" && !(url.protocol === "http:" && PLAIN_HTTP_OK.test(url.hostname)))
      return { kind: "misconfigured", problem: "QUOD_SERVER_URL must be https (plain http only for localhost or a private .internal host)." };
    const teams = parseServerTeams(env.QUOD_SERVER_TEAMS);
    if (!teams.ok) return { kind: "misconfigured", problem: teams.problem };
    return { kind: "http", url: url.origin + url.pathname.replace(/\/+$/, "") };
  }
  if (mode === "") {
    // The stub's old variable: refuse rather than silently run without Quod.
    if (env.QUOD_API_URL?.trim()) return { kind: "misconfigured", problem: "QUOD_API_URL was renamed: set QUOD_API=http and QUOD_SERVER_URL." };
    return { kind: "none" };
  }
  return { kind: "misconfigured", problem: `QUOD_API is "${mode.slice(0, 20)}"; use "http", "fixture", or leave it unset.` };
}
