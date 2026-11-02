import { randomBytes } from "node:crypto";
import { sealData, unsealData } from "iron-session";

// GitHub App Manifest flow: the operator's browser POSTs a manifest to
// github.com/settings/apps/new?state=..., GitHub creates the App and sends the
// browser to redirect_url with ?code=&state=. The state is random, kept in a
// sealed httpOnly cookie, and must match on return.

export const MANIFEST_COOKIE = "quod-gh-manifest";
export const MANIFEST_TTL = 600; // seconds; GitHub's code is valid for an hour
export const GITHUB_NEW_APP_URL = "https://github.com/settings/apps/new";
export const DEFAULT_APP_NAME = "Quod CI";

export interface AppManifest {
  name: string;
  url: string;
  hook_attributes: { url: string; active: boolean };
  redirect_url: string;
  callback_urls: string[];
  setup_url: string;
  setup_on_update: boolean;
  request_oauth_on_install: boolean;
  public: boolean;
  description: string;
  default_permissions: Record<string, "read" | "write">;
  default_events: string[];
}

/**
 * The manifest for `origin` (https://app.quodci.com in production). The
 * permissions are the minimum for live CI status: actions:read (workflow
 * runs and jobs) and metadata:read (required by GitHub for every App), plus
 * administration:read for default-branch protection (the writer-trust
 * warning, DECISIONS.md 2026-11-02), with the webhooks that say it changed.
 * The App that exists today was created before these; its settings are
 * changed by hand (docs/api.md "Go-live").
 */
export function buildManifest(origin: string, name: string = DEFAULT_APP_NAME): AppManifest {
  const o = new URL(origin).origin;
  return {
    name,
    url: "https://quodci.com",
    hook_attributes: { url: `${o}/api/github/webhook`, active: true },
    redirect_url: `${o}/setup/github-app/callback`,
    callback_urls: [`${o}/github/installed`],
    setup_url: `${o}/github/installed`,
    setup_on_update: true,
    request_oauth_on_install: true,
    public: true,
    description: "Live CI job status for Quod Cloud. Reads workflow runs and jobs; never writes to your repositories.",
    default_permissions: { actions: "read", administration: "read", metadata: "read" },
    default_events: ["workflow_job", "workflow_run", "branch_protection_rule", "repository_ruleset", "repository"],
  };
}

/** GitHub App names: up to 34 characters; letters, digits, spaces, dashes, dots, underscores. */
export function checkAppName(value: unknown): { ok: true; value: string } | { ok: false; error: string } {
  const name = typeof value === "string" ? value.trim().replace(/\s+/g, " ") : "";
  if (!name) return { ok: false, error: "Enter a name for the App." };
  if (name.length > 34) return { ok: false, error: "GitHub allows at most 34 characters." };
  if (!/^[A-Za-z0-9][A-Za-z0-9 ._-]*$/.test(name)) return { ok: false, error: "Use letters, digits, spaces, dots, dashes or underscores." };
  return { ok: true, value: name };
}

export const newState = () => randomBytes(24).toString("base64url");

interface Sealed {
  purpose: "quod-gh-manifest";
  state: string;
}

export async function sealState(state: string, password: string): Promise<string> {
  return sealData({ purpose: "quod-gh-manifest", state } satisfies Sealed, { password, ttl: MANIFEST_TTL });
}

/**
 * True only when the cookie unseals (right password, not expired), was sealed
 * for this purpose, and its state equals the one GitHub sent back.
 */
export async function stateMatches(cookie: string | undefined, returned: string | null | undefined, password: string): Promise<boolean> {
  if (!cookie || typeof returned !== "string" || !/^[A-Za-z0-9_-]{16,128}$/.test(returned)) return false;
  try {
    const v = await unsealData<Partial<Sealed>>(cookie, { password, ttl: MANIFEST_TTL });
    if (v?.purpose !== "quod-gh-manifest" || typeof v.state !== "string" || v.state.length !== returned.length) return false;
    // Constant-time enough for a single-use random value; still avoid early exit.
    let diff = 0;
    for (let i = 0; i < returned.length; i++) diff |= v.state.charCodeAt(i) ^ returned.charCodeAt(i);
    return diff === 0;
  } catch {
    return false;
  }
}

/** The values the operator pastes into Vercel, from the conversion response. */
export function envFromConversion(c: { id: number; slug?: string | null; client_id: string; client_secret: string; webhook_secret: string | null; pem: string }): [string, string, boolean][] {
  return [
    ["GITHUB_APP_ID", String(c.id), false],
    ["GITHUB_APP_SLUG", c.slug ?? "", false],
    ["GITHUB_APP_CLIENT_ID", c.client_id, false],
    ["GITHUB_APP_CLIENT_SECRET", c.client_secret, true],
    ["GITHUB_APP_WEBHOOK_SECRET", c.webhook_secret ?? "", true],
    ["GITHUB_APP_PRIVATE_KEY", c.pem, true],
  ];
}
