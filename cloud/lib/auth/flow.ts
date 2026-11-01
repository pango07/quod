import "server-only";
import { getWorkOS, saveSession } from "@workos-inc/authkit-nextjs";
import { AuthenticationException, type AuthenticationResponse } from "@workos-inc/node";
import { sealData, unsealData } from "iron-session";
import { cookies, headers } from "next/headers";

// Our own sign-in on the WorkOS User Management APIs. The session cookie is
// AuthKit's (saveSession), so the proxy, withAuth and requireTeam work as before.
// Nothing here logs a password, a code or a token.

const PENDING_COOKIE = "quod-auth-pending";
const OAUTH_COOKIE = "quod-auth-oauth";
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

// ---------------------------------------------------------------- Pending authentication

export type PendingStep = "verify-email" | "select-org" | "mfa-enroll" | "mfa-challenge";

export interface Pending {
  step: PendingStep;
  token: string;
  returnTo: string;
  email?: string;
  userId?: string;
  organizations?: { id: string; name: string }[];
  challengeId?: string;
  totp?: { qrCode: string; secret: string };
}

async function setSealed(name: string, value: unknown) {
  const sealed = await sealData(value, { password: password(), ttl: TTL });
  (await cookies()).set(name, sealed, { httpOnly: true, secure: secure(), sameSite: "lax", path: "/", maxAge: TTL });
}

async function readSealed<T>(name: string): Promise<T | null> {
  const raw = (await cookies()).get(name)?.value;
  if (!raw) return null;
  try {
    const v = await unsealData<T>(raw, { password: password(), ttl: TTL });
    return v && Object.keys(v as object).length ? v : null;
  } catch {
    return null;
  }
}

export const readPending = () => readSealed<Pending>(PENDING_COOKIE);
export const savePending = (p: Pending) => setSealed(PENDING_COOKIE, p);
export const clearPending = async () => (await cookies()).delete(PENDING_COOKIE);

// ---------------------------------------------------------------- Redirect flows (OAuth, SSO)

interface OAuthState {
  state: string;
  verifier: string;
  returnTo: string;
  invitationToken?: string;
}

export type RedirectTarget = { provider: "GitHubOAuth" | "GoogleOAuth" } | { organizationId: string } | { connectionId: string };

/** Starts GitHub, Google or SSO sign-in with PKCE. Returns the URL to send the browser to. */
export async function startRedirect(target: RedirectTarget, returnTo: string, opts: { invitationToken?: string; loginHint?: string } = {}): Promise<string> {
  const workos = getWorkOS();
  const pkce = await workos.pkce.generate();
  const state = crypto.randomUUID();
  await setSealed(OAUTH_COOKIE, { state, verifier: pkce.codeVerifier, returnTo, invitationToken: opts.invitationToken } satisfies OAuthState);
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

/** For /callback: the stored flow when `state` matches, consumed once. */
export async function takeOAuthState(state: string | null): Promise<OAuthState | null> {
  const stored = await readSealed<OAuthState>(OAUTH_COOKIE);
  if (!stored || !state || stored.state !== state) return null;
  (await cookies()).delete(OAUTH_COOKIE);
  return stored;
}

// ---------------------------------------------------------------- Outcomes

export type Outcome = { kind: "done"; to: string } | { kind: "pending"; to: string } | { kind: "redirect"; to: string } | { kind: "error"; message: string };

export const STEP_PATH: Record<PendingStep, string> = {
  "verify-email": "/verify-email",
  "select-org": "/sign-in/organization",
  "mfa-enroll": "/sign-in/mfa",
  "mfa-challenge": "/sign-in/mfa",
};

/** A signed-in response: set AuthKit's session cookie and go to returnTo. */
export async function finish(response: AuthenticationResponse, returnTo: string): Promise<{ kind: "done"; to: string }> {
  await saveSession(response, redirectUri() || "http://localhost:3000/callback");
  await clearPending();
  return { kind: "done", to: returnTo };
}

export const GENERIC = "That didn't work. Check what you typed and try again.";
export const RATE_LIMITED = "Too many attempts. Wait a minute, then try again.";

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
export async function fromError(e: unknown, ctx: { returnTo: string; email?: string; invitationToken?: string }, fallback = GENERIC): Promise<Outcome> {
  const err = e as ErrorShape;
  if (err?.status === 429) return { kind: "error", message: RATE_LIMITED };

  if (e instanceof AuthenticationException || (e as { name?: string })?.name === "AuthenticationException") {
    const ae = e as AuthenticationException;
    const raw = (ae.rawData ?? {}) as unknown as Record<string, unknown>;
    const token = ae.pendingAuthenticationToken ?? (raw.pending_authentication_token as string | undefined);
    const user = raw.user as { id?: string; email?: string } | undefined;
    const base = { token: token ?? "", returnTo: ctx.returnTo, email: user?.email ?? ctx.email, userId: user?.id };
    const code = (raw.code as string | undefined) ?? (raw.error as string | undefined);

    if (code === "sso_required") {
      const connection = (raw.connection_ids as string[] | undefined)?.[0];
      if (connection) return { kind: "redirect", to: await startRedirect({ connectionId: connection }, ctx.returnTo, { invitationToken: ctx.invitationToken, loginHint: ctx.email }) };
      return { kind: "error", message: "Your organization signs in with SSO. Ask your IT admin for the sign-in link." };
    }
    if (!token) return { kind: "error", message: fallback };

    if (code === "email_verification_required") {
      await savePending({ ...base, step: "verify-email" });
      return { kind: "pending", to: STEP_PATH["verify-email"] };
    }
    if (code === "organization_selection_required") {
      const organizations = ((raw.organizations as { id: string; name: string }[] | undefined) ?? []).slice(0, 50).map((o) => ({ id: o.id, name: o.name }));
      await savePending({ ...base, step: "select-org", organizations });
      return { kind: "pending", to: STEP_PATH["select-org"] };
    }
    if (code === "mfa_challenge") {
      const factor = (raw.authentication_factors as { id: string; type: string }[] | undefined)?.find((f) => f.type === "totp");
      if (!factor) return { kind: "error", message: fallback };
      const challenge = await getWorkOS().multiFactorAuth.challengeFactor({ authenticationFactorId: factor.id });
      await savePending({ ...base, step: "mfa-challenge", challengeId: challenge.id });
      return { kind: "pending", to: STEP_PATH["mfa-challenge"] };
    }
    if (code === "mfa_enrollment" && user?.id) {
      // Enroll an authenticator app (TOTP) for this user, then ask for its first code.
      const { data } = await getWorkOS().post<{
        authentication_factor: { totp: { qr_code: string; secret: string } };
        authentication_challenge: { id: string };
      }>(`/user_management/users/${encodeURIComponent(user.id)}/auth_factors`, { type: "totp", totp_issuer: "Quod", totp_user: user.email ?? ctx.email ?? "" });
      await savePending({
        ...base,
        step: "mfa-enroll",
        challengeId: data.authentication_challenge.id,
        totp: { qrCode: data.authentication_factor.totp.qr_code, secret: data.authentication_factor.totp.secret },
      });
      return { kind: "pending", to: STEP_PATH["mfa-enroll"] };
    }
  }

  // Log the kind of failure only: never the request body (it holds the password or code).
  console.warn("[quod-cloud] sign-in step failed", { status: err?.status, code: err?.code ?? (err?.rawData?.code as string | undefined) });
  return { kind: "error", message: fallback };
}
