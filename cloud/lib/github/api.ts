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

// ---------------------------------------------------------------- Reconciliation (review M2)

export interface HookDelivery {
  id: number;
  guid: string;
  delivered_at: string;
  redelivery: boolean;
  status_code: number;
  event: string;
  action?: string | null;
  installation_id?: number | null;
}

/** The URL of rel="next" in a Link header, if any. */
export function nextLink(link: string | null): string | null {
  if (!link) return null;
  for (const part of link.split(",")) {
    const m = /<([^>]+)>\s*;\s*rel="next"/.exec(part);
    if (m) return m[1];
  }
  return null;
}

/**
 * GET /app/hook/deliveries (App JWT), newest first, back to `sinceMs`.
 * GitHub does not redeliver failed webhooks by itself; this is how Quod finds them.
 */
export async function listAppHookDeliveries(jwt: string, sinceMs: number, opts: { failuresOnly?: boolean } = {}, f: Fetch = fetch): Promise<HookDelivery[]> {
  const out: HookDelivery[] = [];
  let url: string | null = `${GITHUB_API}/app/hook/deliveries?per_page=100${opts.failuresOnly ? "&status=failure" : ""}`;
  for (let page = 0; url && page < MAX_PAGES; page++) {
    if (!url.startsWith(`${GITHUB_API}/`)) break; // only follow GitHub's own links
    const res: Response = await f(url, { headers: headers(jwt), cache: "no-store" });
    const batch = await json<HookDelivery[]>(res, "list hook deliveries");
    let older = false;
    for (const d of batch) {
      if (new Date(d.delivered_at).getTime() < sinceMs) older = true;
      else out.push(d);
    }
    if (older || batch.length === 0) break;
    url = nextLink(res.headers.get("link"));
  }
  return out;
}

/** POST /app/hook/deliveries/{id}/attempts: GitHub sends that delivery again (202). */
export async function redeliver(jwt: string, deliveryId: number, f: Fetch = fetch): Promise<boolean> {
  const res = await f(`${GITHUB_API}/app/hook/deliveries/${deliveryId}/attempts`, { method: "POST", headers: headers(jwt), cache: "no-store" });
  return res.status === 202 || res.ok;
}

const repoPath = (fullName: string) => fullName.split("/").map(encodeURIComponent).join("/");

/** GET /repos/{owner}/{repo}/actions/runs/{id}, or null when the run is gone. Same shape as the webhook's workflow_run. */
export async function getRun(token: string, fullName: string, runId: number, f: Fetch = fetch): Promise<Record<string, unknown> | null> {
  const res = await f(`${GITHUB_API}/repos/${repoPath(fullName)}/actions/runs/${runId}`, { headers: headers(token), cache: "no-store" });
  if (res.status === 404) return null;
  return json<Record<string, unknown>>(res, "get run");
}

/** GET /repos/{owner}/{repo}/actions/runs/{id}/jobs (latest attempt). Same shape as the webhook's workflow_job. */
export async function listRunJobs(token: string, fullName: string, runId: number, f: Fetch = fetch): Promise<Record<string, unknown>[]> {
  const url = `${GITHUB_API}/repos/${repoPath(fullName)}/actions/runs/${runId}/jobs?filter=latest`;
  return (await paged<Record<string, unknown>>(url, token, "jobs", "list run jobs", f)).items;
}

/** What GitHub said about the rate limit on a response (null when it did not say). */
export interface RateInfo {
  remaining: number | null;
  /** When the limit resets (ms since epoch). */
  resetAt: number | null;
  /** Retry-After, in ms. */
  retryAfterMs: number | null;
}

export function rateInfo(h: Headers): RateInfo {
  const n = (v: string | null) => (v !== null && /^\d{1,12}$/.test(v.trim()) ? Number(v.trim()) : null);
  const remaining = n(h.get("x-ratelimit-remaining"));
  const reset = n(h.get("x-ratelimit-reset"));
  const retry = n(h.get("retry-after"));
  return { remaining, resetAt: reset === null ? null : reset * 1000, retryAfterMs: retry === null ? null : retry * 1000 };
}

export type JobRead =
  | { kind: "ok"; job: Record<string, unknown>; rate: RateInfo }
  | { kind: "gone"; rate: RateInfo }
  | { kind: "limited"; rate: RateInfo }
  | { kind: "error"; status: number; rate: RateInfo };

/**
 * GET /repos/{owner}/{repo}/actions/jobs/{job_id}: one job with its steps,
 * same shape as the webhook's workflow_job. Never throws on an HTTP status:
 * 404/410 is "gone", 403/429 "limited" (rate or abuse limit), with the
 * rate-limit headers either way.
 */
export async function getJob(token: string, fullName: string, jobId: number, f: Fetch = fetch): Promise<JobRead> {
  const res = await f(`${GITHUB_API}/repos/${repoPath(fullName)}/actions/jobs/${jobId}`, { headers: headers(token), cache: "no-store" });
  const rate = rateInfo(res.headers);
  if (res.ok) return { kind: "ok", job: (await res.json()) as Record<string, unknown>, rate };
  if (res.status === 404 || res.status === 410) return { kind: "gone", rate };
  if (res.status === 403 || res.status === 429) return { kind: "limited", rate };
  return { kind: "error", status: res.status, rate };
}

/** POST /app/installations/{id}/access_tokens, with its expiry (ms since epoch). */
export async function installationTokenWithExpiry(jwt: string, installationId: number, f: Fetch = fetch): Promise<{ token: string; expiresAt: number }> {
  const res = await f(`${GITHUB_API}/app/installations/${installationId}/access_tokens`, { method: "POST", headers: headers(jwt), cache: "no-store" });
  const body = await json<{ token: string; expires_at?: string }>(res, "installation token");
  const exp = body.expires_at ? Date.parse(body.expires_at) : NaN;
  return { token: body.token, expiresAt: Number.isFinite(exp) ? exp : Date.now() + 55 * 60_000 };
}

/** GET /installation/repositories: every repo in the installation. */
export async function listInstallationRepos(instToken: string, f: Fetch = fetch) {
  return paged<GhRepoWithPermissions>(`${GITHUB_API}/installation/repositories`, instToken, "repositories", "list installation repos", f);
}
