// Which sign-in the app uses, decided from the environment alone.
//
// The rule (DECISIONS.md, 2026-11-01, Quod Cloud app):
// - All four WorkOS variables set and valid: WorkOS AuthKit.
// - None of them set, QUOD_AUTH=demo, AND NODE_ENV is not "production": the
//   local demo session, so the app can be run and screenshotted without an
//   account. The demo is a positive opt-in (cloud/.env.development sets it
//   for `next dev` only); a missing variable never turns it on, whatever
//   NODE_ENV a host happens to set.
// - Anything else is misconfigured, and the app refuses to serve. Production
//   never runs the demo, even with QUOD_AUTH=demo (a second guard). A partial
//   WorkOS config refuses everywhere, so a typo never drops anyone into the demo.
//
// This module is pure (no Next.js imports) so next.config.ts, the proxy,
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
    const demo = env.QUOD_AUTH === "demo";
    if (demo && !production) return { kind: "demo" };
    return {
      kind: "misconfigured",
      problems: [
        demo
          ? "QUOD_AUTH=demo is refused in production: set the WorkOS variables."
          : `Missing ${WORKOS_ENV.join(", ")}. Set them, or QUOD_AUTH=demo for a local demo session (never in production).`,
      ],
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
