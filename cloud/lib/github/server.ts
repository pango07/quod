import "server-only";
import { sealData, unsealData } from "iron-session";
import { cookies } from "next/headers";
import { authMode } from "@/lib/auth/config";
import { assertCookieFits } from "@/lib/auth/limits";
import type { Viewer } from "@/lib/auth/types";
import type { TeamWithRole } from "@/lib/dal";
import * as gh from "./api";
import type { GitHubAppConfig } from "./env";
import type { GitHubForLinking, LinkClaim } from "./link";

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

/** The viewer's current team: the one the session is scoped to, else their only team. Null when that is ambiguous or empty. */
export function currentTeam(viewer: Viewer, teams: TeamWithRole[]): TeamWithRole | null {
  return teams.find((t) => t.orgId === viewer.currentOrgId) ?? (teams.length === 1 ? teams[0] : null);
}

/** The origin a request came in on, behind Vercel's proxy. */
export function requestOrigin(h: Headers): string {
  const host = h.get("x-forwarded-host") ?? h.get("host") ?? "localhost:3000";
  const proto = h.get("x-forwarded-proto") ?? (host.startsWith("localhost") || host.startsWith("127.0.0.1") ? "http" : "https");
  return `${proto.split(",")[0].trim()}://${host.split(",")[0].trim()}`;
}
