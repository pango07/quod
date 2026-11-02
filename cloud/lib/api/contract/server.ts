// quod-server's recorded answers (contract/quod-server/*), served as a fetch
// answer (tests) or by a local HTTP stub (scripts/quod-server-stub.mjs, dev
// only; it reads the same routes.json). Recorded from crates/quod-server on
// exp/cloud-ledger by contract/quod-server/dump.rs: a team "acme", repo
// acme/app, runs 1-3 of two steps ("go test", "go vet"), proven on run 2 and
// replayed on run 3.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const DIR = fileURLToPath(new URL("./quod-server/", import.meta.url));

export const fixture = (name: string): string => readFileSync(`${DIR}${name}`, "utf8");
export const fixtureJson = <T = unknown>(name: string): T => JSON.parse(fixture(name)) as T;

/** The recorded step keys (64 hex digits), as quod-server shows them. */
export const KEY_TEST = "9c9dd597eb9d692589f1641117d2da28e00a745e74554ccb181ff0fb8fd8e686";
export const KEY_VET = "9a097a0a9b9dc5cfc7738a86a9cba3fb8383b4470ee5fd08ebc34d4075179938";

/** "METHOD /path?query" -> [status, fixture file]. Paths are quod-server's, team "acme". */
export const ROUTES = fixtureJson<Record<string, [number, string]>>("routes.json");

/** quod-server's answer to a request, as it recorded it; another team is its 404. */
export function answer(method: string, pathAndQuery: string): { status: number; body: string; type: string } {
  const r = ROUTES[`${method} ${pathAndQuery}`];
  if (r) return { status: r[0], body: fixture(r[1]), type: "application/json" };
  if (method === "GET" && /^\/v1\/teams\/acme\/repos\/app\/runs\/\d+$/.test(pathAndQuery)) return { status: 404, body: '{"error":"not_found","message":"no such run"}', type: "application/json" };
  if (/^\/v1\/teams\/acme\//.test(pathAndQuery)) return { status: 404, body: '{"error":"not_found","message":"no such repo"}', type: "application/json" };
  return { status: 404, body: fixture("team-404.json"), type: "application/json" };
}
