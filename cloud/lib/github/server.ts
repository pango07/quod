import "server-only";
import { randomBytes } from "node:crypto";
import { sealData, unsealData } from "iron-session";
import { cookies } from "next/headers";
import type { NextRequest } from "next/server";
import { authMode } from "@/lib/auth/config";
import { assertCookieFits } from "@/lib/auth/limits";
import type { Viewer } from "@/lib/auth/types";
import type { TeamWithRole } from "@/lib/dal";
import * as gh from "./api";
import { appOrigin, type GitHubAppConfig } from "./env";
import { checkFlowState, FLOW_STATE_TTL, sealFlowState, type GitHubFlowState } from "./flow-state";
import type { GitHubForLinking, LinkClaim } from "./link";
import type { ReconcileGitHub } from "./reconcile";

// Next.js glue for the GitHub App pages and routes: sealed cookies, the
// GitHub client with the App's credentials, and the viewer's current team.

/** The cookie-sealing password: WorkOS's session password, or a fixed one in the local demo session only. */
export function sealPassword(): string {
  const p = process.env.WORKOS_COOKIE_PASSWORD;
  if (p && p.length >= 32) return p;
  if (authMode() === "demo") return "quod-demo-session-only-not-a-secret-0000";
  throw new Error("WORKOS_COOKIE_PASSWORD is not set");
}

export const secureCookies = (origin: string) => origin.startsWith("https://");

// ---------------------------------------------------------------- Link claim cookie

export const CLAIM_COOKIE = "quod-gh-claim";
const CLAIM_TTL = 600;

export async function saveClaim(claim: LinkClaim, origin: string): Promise<void> {
  const sealed = await sealData(claim, { password: sealPassword(), ttl: CLAIM_TTL });
  assertCookieFits(CLAIM_COOKIE, sealed);
  (await cookies()).set(CLAIM_COOKIE, sealed, { httpOnly: true, secure: secureCookies(origin), sameSite: "lax", path: "/", maxAge: CLAIM_TTL });
}

export async function readClaim(): Promise<LinkClaim | null> {
  const raw = (await cookies()).get(CLAIM_COOKIE)?.value;
  if (!raw) return null;
  try {
    const v = await unsealData<LinkClaim>(raw, { password: sealPassword(), ttl: CLAIM_TTL });
    return v?.purpose === "quod-gh-claim" && typeof v.installationId === "number" ? v : null;
  } catch {
    return null;
  }
}

export const clearClaim = async () => (await cookies()).delete(CLAIM_COOKIE);

// ---------------------------------------------------------------- GitHub with the App's credentials

export async function appInstallationRepos(config: GitHubAppConfig, installationId: number) {
  const token = await gh.installationToken(gh.appJwt(config.appId, config.privateKey), installationId);
  return gh.listInstallationRepos(token);
}

export function githubForLinking(config: GitHubAppConfig): GitHubForLinking {
  return {
    exchangeCode: (code) => gh.exchangeUserCode({ clientId: config.clientId, clientSecret: config.clientSecret, code }),
    getUser: (t) => gh.getUser(t),
    listUserInstallations: (t) => gh.listUserInstallations(t),
    listUserInstallationRepos: (t, id) => gh.listUserInstallationRepos(t, id),
    listInstallationRepos: (id) => appInstallationRepos(config, id),
    revoke: (t) => gh.revokeUserToken({ clientId: config.clientId, clientSecret: config.clientSecret, token: t }),
  };
}

/** GitHub for the reconcile cron: the App JWT for deliveries, an installation token per installation (cached for this run only). */
export function githubForReconcile(config: GitHubAppConfig): ReconcileGitHub {
  const jwt = () => gh.appJwt(config.appId, config.privateKey);
  const tokens = new Map<number, Promise<string>>();
  const tokenFor = (id: number) => {
    let t = tokens.get(id);
    if (!t) tokens.set(id, (t = gh.installationToken(jwt(), id)));
    return t;
  };
  return {
    listDeliveries: (since) => gh.listAppHookDeliveries(jwt(), since),
    redeliver: (id) => gh.redeliver(jwt(), id),
    getRun: async (id, fullName, runId) => gh.getRun(await tokenFor(id), fullName, runId),
    listRunJobs: async (id, fullName, runId) => gh.listRunJobs(await tokenFor(id), fullName, runId),
  };
}

/** The viewer's current team: the one the session is scoped to, else their only team. Null when that is ambiguous or empty. */
export function currentTeam(viewer: Viewer, teams: TeamWithRole[]): TeamWithRole | null {
  return teams.find((t) => t.orgId === viewer.currentOrgId) ?? (teams.length === 1 ? teams[0] : null);
}

/** For route handlers: APP_URL, else the request's own origin. Never X-Forwarded-Host (m1). */
export function routeOrigin(request: NextRequest): string {
  return appOrigin(request.nextUrl.origin);
}

/** For server components (no request object): APP_URL, else the Host header. Display only, never a redirect target. */
export function pageOrigin(h: Headers): string {
  const host = h.get("host") ?? "localhost:3000";
  const local = /^(localhost|127\.0\.0\.1)(:\d+)?$/.test(host);
  return appOrigin(`${local ? "http" : "https"}://${host}`);
}

// ---------------------------------------------------------------- Install / OAuth state

/**
 * A code from GitHub is only consumed when it comes back with a state this
 * browser got from Quod for this signed-in user (M1): an install started at
 * /github/install, or the OAuth flow /github/installed starts itself. A code
 * without it is dropped, so an attacker's own install-return URL sent to a
 * team admin cannot link the attacker's installation to the admin's team.
 */
export const STATE_COOKIE = "quod-gh-state";

/** Starts an install or OAuth flow: a random state, sealed with the user (and installation) in an httpOnly cookie. */
export async function startFlowState(userId: string, origin: string, installationId?: number): Promise<string> {
  const state = randomBytes(24).toString("base64url");
  const sealed = await sealFlowState({ state, userId, installationId }, sealPassword());
  (await cookies()).set(STATE_COOKIE, sealed, { httpOnly: true, secure: secureCookies(origin), sameSite: "lax", path: "/github", maxAge: FLOW_STATE_TTL });
  return state;
}

/** The sealed flow state if `returned` matches it and it belongs to this user. Consumed when it matches. */
export async function takeFlowState(returned: string | null, userId: string): Promise<GitHubFlowState | null> {
  const jar = await cookies();
  const v = await checkFlowState(jar.get(STATE_COOKIE)?.value, returned, userId, sealPassword());
  if (v) jar.delete({ name: STATE_COOKIE, path: "/github" });
  return v;
}
