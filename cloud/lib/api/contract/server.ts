// quod-server's recorded answers (contract/quod-server/*), served as a fetch
// function (tests) or by a local HTTP stub (scripts/quod-server-stub.mjs,
// dev only). Recorded from crates/quod-server on exp/cloud-ledger by
// contract/quod-server/dump.rs: a team "acme", repo acme/app, runs 1-3 of
// two steps ("go test", "go vet"), proven on run 2 and replayed on run 3.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const DIR = fileURLToPath(new URL("./quod-server/", import.meta.url));

export const fixture = (name: string): string => readFileSync(`${DIR}${name}`, "utf8");
export const fixtureJson = <T = unknown>(name: string): T => JSON.parse(fixture(name)) as T;

/** The recorded step keys (64 hex digits), as quod-server shows them. */
export const KEY_TEST = "9c9dd597eb9d692589f1641117d2da28e00a745e74554ccb181ff0fb8fd8e686";
export const KEY_VET = "9a097a0a9b9dc5cfc7738a86a9cba3fb8383b4470ee5fd08ebc34d4075179938";

/** "METHOD /path?query" -> [status, fixture file]. Paths are quod-server's, team "acme". */
export const ROUTES: Record<string, [number, string]> = {
  "GET /v1/teams/acme/health": [200, "health.json"],
  "GET /v1/teams/acme/overview?range=7d": [200, "overview-7d.json"],
  "GET /v1/teams/acme/overview?range=30d": [200, "overview-7d.json"],
  "GET /v1/teams/acme/overview?range=90d": [200, "overview-7d.json"],
  "GET /v1/teams/acme/overview?range=1y": [400, "range-400.json"],
  "GET /v1/teams/acme/repos": [200, "repos.json"],
  "GET /v1/teams/acme/repos/app": [200, "repo.json"],
  "GET /v1/teams/acme/repos/nope": [404, "repo-404.json"],
  "GET /v1/teams/acme/runs": [200, "runs.json"],
  "GET /v1/teams/acme/runs?limit=50": [200, "runs.json"],
  "GET /v1/teams/acme/runs?limit=10": [200, "runs.json"],
  "GET /v1/teams/acme/runs?repo=app": [200, "runs.json"],
  "GET /v1/teams/acme/runs?repo=nope": [404, "repo-404.json"],
  "GET /v1/teams/acme/repos/app/runs/1": [200, "run-1.json"],
  "GET /v1/teams/acme/repos/app/runs/3": [200, "run-3.json"],
  "GET /v1/teams/acme/steps": [200, "steps.json"],
  "GET /v1/teams/acme/steps?repo=app": [200, "steps.json"],
  [`GET /v1/teams/acme/repos/app/steps/${KEY_VET}`]: [200, "step.json"],
  "GET /v1/teams/acme/search-index": [200, "search-index.json"],
  "GET /v1/teams/acme/activity": [200, "activity.json"],
  "GET /v1/teams/acme/runners": [501, "runners-501.json"],
  "GET /v1/teams/acme/connect": [501, "connect-501.json"],
  "PUT /v1/teams/acme/connect": [501, "save-connect-501.json"],
  "POST /v1/teams/acme/repos/app/runs/3/rerun": [501, "rerun-501.json"],
  [`PUT /v1/teams/acme/repos/app/steps/${KEY_VET}/proofs`]: [501, "proofs-501.json"],
  "POST /v1/teams/resolve": [501, "resolve-501.json"],
};

/** quod-server's answer to a request, as it recorded it; another team is its 404. */
export function answer(method: string, pathAndQuery: string): { status: number; body: string; type: string } {
  const r = ROUTES[`${method} ${pathAndQuery}`];
  if (r) return { status: r[0], body: fixture(r[1]), type: "application/json" };
  if (method === "GET" && /^\/v1\/teams\/acme\/repos\/app\/runs\/\d+$/.test(pathAndQuery)) return { status: 404, body: '{"error":"not_found","message":"no such run"}', type: "application/json" };
  if (/^\/v1\/teams\/acme\//.test(pathAndQuery)) return { status: 404, body: '{"error":"not_found","message":"no such repo"}', type: "application/json" };
  return { status: 404, body: fixture("team-404.json"), type: "application/json" };
}
