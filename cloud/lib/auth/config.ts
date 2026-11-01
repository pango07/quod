// Which sign-in the app uses, decided from the environment alone.
//
// The rule (DECISIONS.md, 2026-11-01, Quod Cloud app):
// - All four WorkOS variables set and valid: WorkOS AuthKit.
// - None of them set AND NODE_ENV is not "production": the local demo
//   session, so the app can be run and screenshotted without an account.
// - Anything else is misconfigured, and the app refuses to serve. In
//   production that includes "nothing set": missing config never becomes a
//   way in. A partial config also refuses in development, so a typo never
//   silently drops someone into the demo.
//
// This module is pure (no Next.js imports) so the proxy, instrumentation,
// server code and the unit test all share the one rule.

export const WORKOS_ENV = [
  "WORKOS_API_KEY",
  "WORKOS_CLIENT_ID",
  "WORKOS_COOKIE_PASSWORD",
  "NEXT_PUBLIC_WORKOS_REDIRECT_URI",
] as const;

export type WorkosEnvName = (typeof WORKOS_ENV)[number];

export type AuthMode =
  | { kind: "workos" }
  | { kind: "demo" }
  | { kind: "misconfigured"; problems: string[] };

type Env = Partial<Record<string, string | undefined>>;

const present = (v: string | undefined) => typeof v === "string" && v.trim() !== "";

export function resolveAuthMode(env: Env): AuthMode {
  const set = WORKOS_ENV.filter((k) => present(env[k]));
  const production = env.NODE_ENV === "production";

  if (set.length === 0) {
    if (!production) return { kind: "demo" };
    return {
      kind: "misconfigured",
      problems: [`Missing ${WORKOS_ENV.join(", ")}. Production never falls back to the demo session.`],
    };
  }

  const problems: string[] = [];
  const missing = WORKOS_ENV.filter((k) => !present(env[k]));
  if (missing.length) problems.push(`Missing ${missing.join(", ")}.`);

  const password = env.WORKOS_COOKIE_PASSWORD ?? "";
  if (present(password) && password.length < 32) {
    problems.push("WORKOS_COOKIE_PASSWORD must be at least 32 characters.");
  }

  const redirect = env.NEXT_PUBLIC_WORKOS_REDIRECT_URI ?? "";
  if (present(redirect)) {
    try {
      const url = new URL(redirect);
      if (production && url.protocol !== "https:" && url.hostname !== "localhost") {
        problems.push("NEXT_PUBLIC_WORKOS_REDIRECT_URI must use https in production.");
      }
    } catch {
      problems.push("NEXT_PUBLIC_WORKOS_REDIRECT_URI is not a URL.");
    }
  }

  return problems.length ? { kind: "misconfigured", problems } : { kind: "workos" };
}

export class AuthConfigError extends Error {
  constructor(problems: string[]) {
    super(`Quod Cloud sign-in is not configured. ${problems.join(" ")} See cloud/.env.example.`);
    this.name = "AuthConfigError";
  }
}

/** The mode for this process. Throws when the app must not serve. */
export function authMode(env: Env = process.env): "workos" | "demo" {
  const mode = resolveAuthMode(env);
  if (mode.kind === "misconfigured") throw new AuthConfigError(mode.problems);
  return mode.kind;
}
