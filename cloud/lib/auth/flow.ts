import "server-only";
import { getWorkOS, saveSession } from "@workos-inc/authkit-nextjs";
import { AuthenticationException, type AuthenticationResponse } from "@workos-inc/node";
import { sealData, unsealData } from "iron-session";
import { cookies, headers } from "next/headers";
import { assertCookieFits, UNSUPPORTED_MESSAGE, UNSUPPORTED_STEPS } from "./limits";

// Our own sign-in on the WorkOS User Management APIs. The session cookie is
// AuthKit's (saveSession), so the proxy, withAuth and requireTeam work as before.
// Nothing here logs a password, a code or a token.

const PENDING_COOKIE = "quod-auth-pending";
const OAUTH_COOKIE = "quod-auth-oauth"; // suffixed with the flow's state id, one cookie per flow
const TTL = 600; // seconds

const password = () => {
  const p = process.env.WORKOS_COOKIE_PASSWORD;
  if (!p || p.length < 32) throw new Error("WORKOS_COOKIE_PASSWORD is not set");
  return p;
};

export const clientId = () => process.env.WORKOS_CLIENT_ID ?? "";

/** The redirect URI, read at runtime (not inlined at build). */
export function redirectUri(): string {
  return process.env["NEXT_PUBLIC_WORKOS_REDIRECT_URI"] ?? "";
}

const secure = () => redirectUri().startsWith("https://");

/** Client IP and user agent, for WorkOS Radar. */
export async function clientInfo(): Promise<{ ipAddress?: string; userAgent?: string }> {
  const h = await headers();
  const ip = (h.get("x-forwarded-for")?.split(",")[0] ?? h.get("x-real-ip") ?? "").trim();
  const ua = h.get("user-agent") ?? "";
  return { ipAddress: ip || undefined, userAgent: ua.slice(0, 512) || undefined };
}

const log = (what: string, e: unknown) => {
  const err = e as { status?: number; code?: string; rawData?: { code?: string; error?: string } };
  console.warn(`[quod-cloud] ${what}`, { status: err?.status, code: err?.code ?? err?.rawData?.code ?? err?.rawData?.error });
};

// ---------------------------------------------------------------- Sealed cookies

type Purpose = "quod-pending" | "quod-oauth";

async function setSealed(name: string, purpose: Purpose, value: object) {
  const sealed = await sealData({ ...value, purpose }, { password: password(), ttl: TTL });
  assertCookieFits(name, sealed);
  (await cookies()).set(name, sealed, { httpOnly: true, secure: secure(), sameSite: "lax", path: "/", maxAge: TTL });
}

async function readSealed<T extends object>(name: string, purpose: Purpose): Promise<T | null> {
  const raw = (await cookies()).get(name)?.value;
  if (!raw) return null;
  try {
    const v = await unsealData<T & { purpose?: Purpose }>(raw, { password: password(), ttl: TTL });
    // A blob sealed for one purpose is never accepted as the other.
    return v && v.purpose === purpose ? v : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- Pending authentication

export type PendingStep = "verify-email" | "select-org" | "mfa-enroll" | "mfa-challenge";

/** Only small values: the org list and QR code are fetched or drawn when the page renders. */
export interface Pending {
  step: PendingStep;
  /** WorkOS pending_authentication_token; empty for a Magic Auth verify step. */
  token: string;
  returnTo: string;
  email?: string;
  userId?: string;
  /** verify-email: "magic" means the code is a Magic Auth code (sign-up with a taken email). */
  via?: "verification" | "magic";
  invitationToken?: string;
  challengeId?: string;
  /** mfa-enroll: the otpauth:// URI (drawn as a QR code) and its secret. */
  totp?: { uri: string; secret: string };
}

export const readPending = () => readSealed<Pending>(PENDING_COOKIE, "quod-pending");
export const savePending = (p: Pending) => setSealed(PENDING_COOKIE, "quod-pending", p);
export const clearPending = async () => (await cookies()).delete(PENDING_COOKIE);

/** The WorkOS user for an email, when a step needs the id and WorkOS didn't send it. */
export async function userIdForEmail(email: string | undefined): Promise<string | undefined> {
  if (!email) return undefined;
  const page = await getWorkOS().userManagement.listUsers({ email, limit: 1 });
  return page.data[0]?.id;
}

/** The organizations a pending sign-in may pick from, read from WorkOS (never from the cookie). */
export async function organizationsOf(userId: string): Promise<{ id: string; name: string }[]> {
  const page = await getWorkOS().userManagement.listOrganizationMemberships({ userId, statuses: ["active"], limit: 100 });
  return page.data.map((m) => ({ id: m.organizationId, name: m.organizationName }));
}

// ---------------------------------------------------------------- Redirect flows (OAuth, SSO)

interface OAuthState {
  state: string;
  verifier: string;
  returnTo: string;
  invitationToken?: string;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export type RedirectTarget = { provider: "GitHubOAuth" | "GoogleOAuth" } | { organizationId: string } | { connectionId: string };

/** Starts GitHub, Google or SSO sign-in with PKCE. Returns the URL to send the browser to. */
export async function startRedirect(target: RedirectTarget, returnTo: string, opts: { invitationToken?: string; loginHint?: string } = {}): Promise<string> {
  const workos = getWorkOS();
  const pkce = await workos.pkce.generate();
  const state = crypto.randomUUID();
  await setSealed(`${OAUTH_COOKIE}-${state}`, "quod-oauth", { state, verifier: pkce.codeVerifier, returnTo, invitationToken: opts.invitationToken } satisfies OAuthState);
  return workos.userManagement.getAuthorizationUrl({
    ...target,
    clientId: clientId(),
    redirectUri: redirectUri(),
    state,
    loginHint: opts.loginHint,
    codeChallenge: pkce.codeChallenge,
    codeChallengeMethod: "S256",
  });
}

/** For /callback: the flow stored under this state, consumed once. Two tabs never share a cookie. */
export async function takeOAuthState(state: string | null): Promise<OAuthState | null> {
  if (!state || !UUID.test(state)) return null;
  const name = `${OAUTH_COOKIE}-${state}`;
  const stored = await readSealed<OAuthState>(name, "quod-oauth");
  (await cookies()).delete(name);
  return stored && stored.state === state ? stored : null;
}

// ---------------------------------------------------------------- Outcomes

export type Outcome = { kind: "done"; to: string } | { kind: "pending"; to: string } | { kind: "redirect"; to: string } | { kind: "error"; message: string };

export const STEP_PATH: Record<PendingStep, string> = {
  "verify-email": "/verify-email",
  "select-org": "/sign-in/organization",
  "mfa-enroll": "/sign-in/mfa",
  "mfa-challenge": "/sign-in/mfa",
};

export const GENERIC = "That didn't work. Check what you typed and try again.";
export const RATE_LIMITED = "Too many attempts. Wait a minute, then try again.";
export const TRY_AGAIN = "Something went wrong on our side. Try again in a moment.";

/** A signed-in response: seal only what the session needs (never upstream OAuth tokens), then go to returnTo. */
export async function finish(r: AuthenticationResponse, returnTo: string): Promise<{ kind: "done"; to: string }> {
  await saveSession(
    { accessToken: r.accessToken, refreshToken: r.refreshToken, user: r.user, impersonator: r.impersonator, authenticationMethod: r.authenticationMethod },
    redirectUri() || "http://localhost:3000/callback",
  );
  await clearPending();
  return { kind: "done", to: returnTo };
}

export interface Ctx {
  returnTo: string;
  email?: string;
  invitationToken?: string;
}

/**
 * Runs a WorkOS authenticate call, then saves the session. The two are kept
 * apart: a failure after WorkOS said yes (JWKS, network, cookie) says "try
 * again", never "wrong password".
 */
export async function attempt(run: () => Promise<AuthenticationResponse>, ctx: Ctx, fallback = GENERIC): Promise<Outcome> {
  let res: AuthenticationResponse;
  try {
    res = await run();
  } catch (e) {
    return fromError(e, ctx, fallback);
  }
  try {
    return await finish(res, ctx.returnTo);
  } catch (e) {
    log("session not saved after a successful sign-in", e);
    return { kind: "error", message: TRY_AGAIN };
  }
}

interface ErrorShape {
  status?: number;
  code?: string;
  rawData?: Record<string, unknown>;
}

/**
 * Turns a WorkOS error into the next step. Pending-authentication codes become
 * a pending step (stored sealed in a cookie); everything else becomes a
 * generic message that never says whether an account exists.
 */
export async function fromError(e: unknown, ctx: Ctx, fallback = GENERIC): Promise<Outcome> {
  const err = e as ErrorShape;
  if (err?.status === 429) return { kind: "error", message: RATE_LIMITED };

  if (!(e instanceof AuthenticationException || (e as { name?: string })?.name === "AuthenticationException")) {
    log("sign-in step failed", e);
    return { kind: "error", message: fallback };
  }

  const ae = e as AuthenticationException;
  const raw = (ae.rawData ?? {}) as unknown as Record<string, unknown>;
  const code = (raw.code as string | undefined) ?? (raw.error as string | undefined) ?? ae.code;
  const token = ae.pendingAuthenticationToken ?? (raw.pending_authentication_token as string | undefined) ?? "";
  const user = raw.user as { id?: string; email?: string } | undefined;
  // email_verification_required carries `email` (and not always `user`); others carry `user`.
  const email = (raw.email as string | undefined) ?? user?.email ?? ctx.email;
  const base = { token, returnTo: ctx.returnTo, email, userId: user?.id, invitationToken: ctx.invitationToken };

  try {
    if (code === "sso_required") {
      const connection = (raw.connection_ids as string[] | undefined)?.[0];
      if (connection) return { kind: "redirect", to: await startRedirect({ connectionId: connection }, ctx.returnTo, { invitationToken: ctx.invitationToken, loginHint: email }) };
      return { kind: "error", message: "Your organization signs in with SSO. Ask your IT admin for the sign-in link." };
    }
    if (code && UNSUPPORTED_STEPS.has(code)) {
      console.warn("[quod-cloud] unsupported sign-in step", { code });
      return { kind: "error", message: UNSUPPORTED_MESSAGE };
    }
    if (!token) {
      log("pending step without a token", e);
      return { kind: "error", message: fallback };
    }
    if (code === "email_verification_required") {
      await savePending({ ...base, step: "verify-email", via: "verification" });
      return { kind: "pending", to: STEP_PATH["verify-email"] };
    }
    if (code === "organization_selection_required") {
      // Only the user id goes in the cookie; the page lists their organizations from WorkOS.
      const userId = base.userId ?? (await userIdForEmail(email));
      if (!userId) return { kind: "error", message: TRY_AGAIN };
      await savePending({ ...base, userId, step: "select-org" });
      return { kind: "pending", to: STEP_PATH["select-org"] };
    }
    const mfa = getWorkOS().multiFactorAuth;
    if (code === "mfa_challenge") {
      const factor = (raw.authentication_factors as { id: string; type: string }[] | undefined)?.find((f) => f.type === "totp");
      if (!factor) return { kind: "error", message: UNSUPPORTED_MESSAGE };
      const challenge = await mfa.challengeFactor({ authenticationFactorId: factor.id });
      await savePending({ ...base, step: "mfa-challenge", challengeId: challenge.id });
      return { kind: "pending", to: STEP_PATH["mfa-challenge"] };
    }
    if (code === "mfa_enrollment") {
      const userId = base.userId ?? (await userIdForEmail(email));
      if (!userId) return { kind: "error", message: TRY_AGAIN };
      const { authenticationFactor, authenticationChallenge } = await mfa.createUserAuthFactor({ userId, type: "totp", totpIssuer: "Quod", totpUser: email });
      await savePending({
        ...base,
        userId,
        step: "mfa-enroll",
        challengeId: authenticationChallenge.id,
        totp: { uri: authenticationFactor.totp.uri, secret: authenticationFactor.totp.secret },
      });
      return { kind: "pending", to: STEP_PATH["mfa-enroll"] };
    }
  } catch (inner) {
    log(`could not start the ${code} step`, inner);
    return { kind: "error", message: TRY_AGAIN };
  }

  console.warn("[quod-cloud] unknown sign-in step", { code });
  return { kind: "error", message: UNSUPPORTED_MESSAGE };
}
