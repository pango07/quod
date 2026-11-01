// GitHub App configuration, read from the environment at request time.
// The secrets (private key, webhook secret, client secret) are environment
// variables only: never in the database, never logged.

type Env = Partial<Record<string, string | undefined>>;

export interface GitHubAppConfig {
  appId: string;
  slug: string;
  clientId: string;
  clientSecret: string;
  privateKey: string;
  webhookSecret: string;
}

export const GITHUB_APP_ENV = [
  "GITHUB_APP_ID",
  "GITHUB_APP_SLUG",
  "GITHUB_APP_CLIENT_ID",
  "GITHUB_APP_CLIENT_SECRET",
  "GITHUB_APP_PRIVATE_KEY",
  "GITHUB_APP_WEBHOOK_SECRET",
] as const;

const v = (env: Env, k: string) => env[k]?.trim() ?? "";

/** A PEM pasted into a one-line env field has literal "\n"s; restore them. */
export function normalizePem(pem: string): string {
  const t = pem.trim();
  return t.includes("\\n") && !t.includes("\n") ? t.replace(/\\n/g, "\n") : t;
}

/** The App's config, or null with the missing names. */
export function githubAppConfig(env: Env = process.env): { ok: true; config: GitHubAppConfig } | { ok: false; missing: string[] } {
  const missing = GITHUB_APP_ENV.filter((k) => !v(env, k));
  if (missing.length) return { ok: false, missing };
  return {
    ok: true,
    config: {
      appId: v(env, "GITHUB_APP_ID"),
      slug: v(env, "GITHUB_APP_SLUG"),
      clientId: v(env, "GITHUB_APP_CLIENT_ID"),
      clientSecret: v(env, "GITHUB_APP_CLIENT_SECRET"),
      privateKey: normalizePem(v(env, "GITHUB_APP_PRIVATE_KEY")),
      webhookSecret: v(env, "GITHUB_APP_WEBHOOK_SECRET"),
    },
  };
}

/** Lower-cased emails from QUOD_OPERATOR_EMAILS (comma or space separated). */
export function operatorEmails(env: Env = process.env): Set<string> {
  return new Set(
    (env.QUOD_OPERATOR_EMAILS ?? "")
      .split(/[\s,]+/)
      .map((e) => e.trim().toLowerCase())
      .filter((e) => e.includes("@")),
  );
}

export function isOperator(email: string | null | undefined, env: Env = process.env): boolean {
  return typeof email === "string" && email.length > 0 && operatorEmails(env).has(email.trim().toLowerCase());
}

/**
 * The app's public origin: APP_URL when set (https://app.quodci.com), else
 * the request's own origin (so localhost works).
 */
export function appOrigin(requestOrigin: string, env: Env = process.env): string {
  const configured = v(env, "APP_URL");
  if (configured) {
    try {
      return new URL(configured).origin;
    } catch {
      // fall through to the request origin
    }
  }
  return new URL(requestOrigin).origin;
}
