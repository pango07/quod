import { createSign } from "node:crypto";

// The few GitHub REST calls Quod makes. `fetch` is injectable so tests never
// reach GitHub. Nothing here logs a token, a code or a response body.

export const GITHUB_API = "https://api.github.com";
export const GITHUB_WEB = "https://github.com";
// A long-supported REST version; the endpoints used here are the same in later ones.
const API_VERSION = "2022-11-28";
const MAX_PAGES = 10; // 1000 items at 100 per page

type Fetch = typeof fetch;

export class GitHubApiError extends Error {
  constructor(
    readonly what: string,
    readonly status: number,
  ) {
    super(`GitHub ${what} failed (${status})`);
    this.name = "GitHubApiError";
  }
}

function headers(token?: string, auth: "Bearer" | "Basic" = "Bearer"): Record<string, string> {
  return {
    accept: "application/vnd.github+json",
    "x-github-api-version": API_VERSION,
    "user-agent": "quod-cloud",
    ...(token ? { authorization: `${auth} ${token}` } : {}),
  };
}

async function json<T>(res: Response, what: string): Promise<T> {
  if (!res.ok) throw new GitHubApiError(what, res.status);
  return (await res.json()) as T;
}

export interface ManifestConversion {
  id: number;
  slug?: string | null;
  name: string;
  html_url: string;
  owner?: { login: string } | null;
  client_id: string;
  client_secret: string;
  webhook_secret: string | null;
  pem: string;
}

/** POST /app-manifests/{code}/conversions. The code is single-use and valid for an hour; no other credential. */
export async function convertManifest(code: string, f: Fetch = fetch): Promise<ManifestConversion> {
  if (!/^[A-Za-z0-9_-]{1,200}$/.test(code)) throw new GitHubApiError("manifest conversion", 400);
  const res = await f(`${GITHUB_API}/app-manifests/${encodeURIComponent(code)}/conversions`, { method: "POST", headers: headers(), cache: "no-store" });
  return json<ManifestConversion>(res, "manifest conversion");
}

// ---------------------------------------------------------------- User tokens (installation OAuth)

/** Exchanges the OAuth code GitHub sends after an install (request_oauth_on_install) for a user access token. */
export async function exchangeUserCode(input: { clientId: string; clientSecret: string; code: string }, f: Fetch = fetch): Promise<string> {
  if (!/^[A-Za-z0-9_-]{1,200}$/.test(input.code)) throw new GitHubApiError("oauth code exchange", 400);
  const res = await f(`${GITHUB_WEB}/login/oauth/access_token`, {
    method: "POST",
    headers: { accept: "application/json", "content-type": "application/json", "user-agent": "quod-cloud" },
    body: JSON.stringify({ client_id: input.clientId, client_secret: input.clientSecret, code: input.code }),
    cache: "no-store",
  });
  // GitHub answers 200 with { error } for a bad or used code.
  const body = await json<{ access_token?: string; error?: string }>(res, "oauth code exchange");
  if (!body.access_token) throw new GitHubApiError("oauth code exchange", 401);
  return body.access_token;
}

/** Best effort: revokes the user token once linking is done (Quod never keeps it). */
export async function revokeUserToken(input: { clientId: string; clientSecret: string; token: string }, f: Fetch = fetch): Promise<void> {
  const basic = Buffer.from(`${input.clientId}:${input.clientSecret}`).toString("base64");
  await f(`${GITHUB_API}/applications/${encodeURIComponent(input.clientId)}/token`, {
    method: "DELETE",
    headers: { ...headers(basic, "Basic"), "content-type": "application/json" },
    body: JSON.stringify({ access_token: input.token }),
    cache: "no-store",
  }).catch(() => undefined);
}

export interface GhUser {
  id: number;
  login: string;
}

export async function getUser(token: string, f: Fetch = fetch): Promise<GhUser> {
  const u = await json<GhUser>(await f(`${GITHUB_API}/user`, { headers: headers(token), cache: "no-store" }), "get user");
  return { id: u.id, login: u.login };
}

export interface GhUserInstallation {
  id: number;
  account: { id: number; login: string; type?: string } | null;
  repository_selection?: string;
  app_slug?: string;
  suspended_at?: string | null;
}

async function paged<T>(url: string, token: string, key: string, what: string, f: Fetch): Promise<{ items: T[]; complete: boolean }> {
  const items: T[] = [];
  for (let page = 1; page <= MAX_PAGES; page++) {
    const sep = url.includes("?") ? "&" : "?";
    const body = await json<Record<string, unknown>>(await f(`${url}${sep}per_page=100&page=${page}`, { headers: headers(token), cache: "no-store" }), what);
    const batch = (body[key] as T[] | undefined) ?? [];
    items.push(...batch);
    const total = typeof body.total_count === "number" ? body.total_count : null;
    if (batch.length < 100 || (total !== null && items.length >= total)) return { items, complete: true };
  }
  return { items, complete: false };
}

/** GET /user/installations: this App's installations the user can access. */
export async function listUserInstallations(token: string, f: Fetch = fetch): Promise<GhUserInstallation[]> {
  return (await paged<GhUserInstallation>(`${GITHUB_API}/user/installations`, token, "installations", "list user installations", f)).items;
}

export interface GhRepoWithPermissions {
  id: number;
  name: string;
  full_name: string;
  private: boolean;
  permissions?: { admin?: boolean; push?: boolean; pull?: boolean };
}

/** GET /user/installations/{id}/repositories: the repos in that installation the user can access, with the user's permissions. */
export async function listUserInstallationRepos(token: string, installationId: number, f: Fetch = fetch) {
  return paged<GhRepoWithPermissions>(`${GITHUB_API}/user/installations/${installationId}/repositories`, token, "repositories", "list user installation repos", f);
}

// ---------------------------------------------------------------- App and installation tokens

const b64url = (v: string | Buffer) => Buffer.from(v).toString("base64url");

/** A 9-minute App JWT (RS256), issued 60 s in the past for clock drift, as GitHub recommends. */
export function appJwt(appId: string, privateKeyPem: string, nowSec = Math.floor(Date.now() / 1000)): string {
  const head = b64url(JSON.stringify({ alg: "RS256", typ: "JWT" }));
  const body = b64url(JSON.stringify({ iat: nowSec - 60, exp: nowSec + 540, iss: appId }));
  const sig = createSign("RSA-SHA256").update(`${head}.${body}`).sign(privateKeyPem);
  return `${head}.${body}.${b64url(sig)}`;
}

/** A short-lived installation token, read-only by the App's permissions. Used once, never stored. */
export async function installationToken(jwt: string, installationId: number, f: Fetch = fetch): Promise<string> {
  const res = await f(`${GITHUB_API}/app/installations/${installationId}/access_tokens`, { method: "POST", headers: headers(jwt), cache: "no-store" });
  return (await json<{ token: string }>(res, "installation token")).token;
}

/** GET /installation/repositories: every repo in the installation. */
export async function listInstallationRepos(instToken: string, f: Fetch = fetch) {
  return paged<GhRepoWithPermissions>(`${GITHUB_API}/installation/repositories`, instToken, "repositories", "list installation repos", f);
}
