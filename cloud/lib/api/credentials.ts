// How the dashboard authenticates to quod-server for one team, server-side
// only. Two ways, per team (docs/api.md "Viewer auth"):
//
// 1. QUOD_SERVER_TEAMS (secret): a JSON map from a WorkOS org id to the team's
//    id on quod-server and a static viewer token, for quod-server as built
//    today (static viewer tokens; `quod-server hash-token`). The dashboard
//    has already checked that the viewer belongs to the team (lib/dal.ts).
//      {"org_01H...": {"team": "acme", "token": "<viewer token>"}}
// 2. QUOD_SERVER_VIEWER_AUTH=workos (not secret): the viewer's own WorkOS
//    access token, with the team's WorkOS org id as the path's team id. For
//    quod-server once it verifies WorkOS tokens ("Server changes needed").
//
// Neither for a team: quod-server is not used for it, and Quod's numbers
// are empty ("Appears once Quod runs"). The token never leaves the server
// side, and no cookie is sent with it.

import type { ServerCredentials } from "./http";
import { ApiUnavailableError } from "./types";

export interface StaticTeam {
  team: string;
  token: string;
}

const ORG = /^[A-Za-z0-9_-]{1,100}$/;
const TEAM = /^[A-Za-z0-9._-]{1,100}$/;

/** QUOD_SERVER_TEAMS parsed; a malformed value is an error naming the problem, never the token. */
export function parseServerTeams(raw: string | undefined): { ok: true; teams: Map<string, StaticTeam> } | { ok: false; problem: string } {
  const teams = new Map<string, StaticTeam>();
  if (!raw?.trim()) return { ok: true, teams };
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return { ok: false, problem: "QUOD_SERVER_TEAMS is not JSON." };
  }
  if (!v || typeof v !== "object" || Array.isArray(v)) return { ok: false, problem: 'QUOD_SERVER_TEAMS must be an object: {"<WorkOS org id>": {"team": "<quod-server team>", "token": "<viewer token>"}}.' };
  for (const [org, t] of Object.entries(v as Record<string, unknown>)) {
    const e = t as { team?: unknown; token?: unknown } | null;
    if (!ORG.test(org)) return { ok: false, problem: "QUOD_SERVER_TEAMS has a key that is not a WorkOS org id." };
    if (!e || typeof e.team !== "string" || !TEAM.test(e.team) || e.team === "." || e.team === "..") return { ok: false, problem: `QUOD_SERVER_TEAMS["${org}"].team is not a quod-server team id.` };
    if (typeof e.token !== "string" || e.token.length < 16 || /\s/.test(e.token)) return { ok: false, problem: `QUOD_SERVER_TEAMS["${org}"].token is missing or too short.` };
    teams.set(org, { team: e.team, token: e.token });
  }
  return { ok: true, teams };
}

export function viewerAuthEnabled(env: Partial<Record<string, string | undefined>>): boolean {
  return env.QUOD_SERVER_VIEWER_AUTH?.trim() === "workos";
}

/**
 * Credentials for a team, by its org id (from the membership-checked team
 * context), or null when quod-server is not used for it.
 */
export async function credentialsFor(
  orgId: string | null,
  deps: { teams: Map<string, StaticTeam>; viewerAuth: boolean; accessToken: () => Promise<string | undefined> },
): Promise<ServerCredentials | null> {
  if (!orgId) return null;
  const fixed = deps.teams.get(orgId);
  if (fixed) return { team: fixed.team, token: fixed.token, kind: "static" };
  if (!deps.viewerAuth || !TEAM.test(orgId)) return null;
  // Viewer auth is on for this team: a missing token is the session's problem (an expired or
  // not-yet-refreshed WorkOS session, as on a phone resuming a tab), not "this team has no Quod
  // data". "Unavailable" shows an error with a retry; null would show the empty state.
  const token = await deps.accessToken().catch(() => undefined);
  if (!token) throw new ApiUnavailableError("The viewer's session token is not available; reload to sign in again.");
  return { team: orgId, token, kind: "viewer" };
}
