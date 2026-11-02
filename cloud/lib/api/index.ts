import "server-only";
import { notFound, redirect } from "next/navigation";
import { connection } from "next/server";
import { cache } from "react";
import { accessToken } from "@/lib/auth/session";
import { dbConfigured, getDb } from "@/lib/db";
import { githubAppConfig } from "@/lib/github/env";
import { CompositeCloudApi } from "./composite";
import { credentialsFor, parseServerTeams, viewerAuthEnabled } from "./credentials";
import { FixtureCloudApi, newFixtureState, type FixtureState } from "./fixture";
import { isSampleOrg } from "./fixture-data";
import { HttpCloudApi } from "./http";
import { resolveApiMode, type ApiMode } from "./mode";
import { NoQuodCloudApi } from "./none";
import { ApiForbiddenError, ApiInvalidRequestError, type CloudApi } from "./types";

export type * from "./types";

// The fixture's changes (proofs needed, onboarding choice) live in one object
// per process, kept across dev reloads, so they stick until restart.
const g = globalThis as unknown as { __quodFixtureState?: FixtureState };

/** The team's org id, from the viewer's membership-checked teams. Which data a team sees (sample or real) is decided by this id, never by slug or name. */
const orgOf = async (team: string) => (await (await import("@/lib/dal")).findTeam(team))?.team.orgId ?? null;

function quodAdapter(mode: Exclude<ApiMode, { kind: "misconfigured" }>): CloudApi {
  if (mode.kind === "fixture") return new FixtureCloudApi((g.__quodFixtureState ??= newFixtureState()), undefined, orgOf);
  if (mode.kind === "none") return new NoQuodCloudApi();
  const teams = parseServerTeams(process.env.QUOD_SERVER_TEAMS);
  const viewerAuth = viewerAuthEnabled(process.env);
  return new HttpCloudApi({
    baseUrl: mode.url,
    // Server-side only: the static token or the viewer's WorkOS access token, never a cookie.
    credentials: async (team) => credentialsFor(await orgOf(team), { teams: teams.ok ? teams.teams : new Map(), viewerAuth, accessToken }),
  });
}

/**
 * The CloudApi for this request. Pages and actions read data only through
 * this. One instance per request (React's cache), so a page's sections share
 * its lookups (the team's GitHub link, quod-server's repo list).
 */
export const api = cache((): CloudApi => {
  const mode = resolveApiMode(process.env);
  if (mode.kind === "misconfigured") throw new Error(`Quod Cloud data source is not configured. ${mode.problem}`);
  // CI job activity from the GitHub App for linked teams, whatever the inner adapter is.
  const composite = new CompositeCloudApi(quodAdapter(mode), {
    github: {
      configured: () => dbConfigured(),
      appReady: () => githubAppConfig().ok,
      db: () => getDb(),
      protection: async (teamId, db, repos) => (await import("@/lib/github/protection-server")).protectionFor(teamId, db, repos),
    },
    // The TeamId only ever comes from the membership-checked team context (lib/github/team.ts).
    orgOf,
    // Only the demo session's sample org has sample data (and only with the fixture adapter).
    isSample: (orgId) => mode.kind === "fixture" && isSampleOrg(orgId),
  });
  return withRefusals(composite);
});

/**
 * Turns the backend's refusals into navigation: 401 signs in again, 403 and
 * invalid ids are a 404 (so a team's existence does not leak). "Not
 * available yet" (501) and "unavailable" pass through to the page's section.
 */
function withRefusals(inner: CloudApi): CloudApi {
  return new Proxy(inner, {
    get(target, prop, receiver) {
      const v = Reflect.get(target, prop, receiver);
      if (typeof v !== "function") return v;
      // The stream is an async iterable, not a promise; the SSE route handles its errors.
      if (prop === "subscribe") return v.bind(target);
      return async (...args: unknown[]) => {
        // Live data is per request, never prerendered (the simulator reads the clock).
        await connection();
        try {
          return await v.apply(target, args);
        } catch (e) {
          if (e instanceof ApiForbiddenError && e.status === 401) redirect("/sign-in");
          if (e instanceof ApiForbiddenError) notFound();
          if (e instanceof ApiInvalidRequestError) notFound();
          throw e;
        }
      };
    },
  });
}

export function usingSampleData(): boolean {
  return resolveApiMode(process.env).kind === "fixture";
}
