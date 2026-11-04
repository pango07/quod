"use server";

// Server actions behind our sign-in pages. Every input is checked here (the
// forms are only a convenience), errors are generic so they never say whether
// an account exists, returnTo is always a same-origin path, and no password,
// code or token is logged.

import { getWorkOS, switchToOrganization, withAuth } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { authMode } from "./config";
import {
  attempt,
  clearPending,
  clientId,
  clientInfo,
  GENERIC,
  RATE_LIMITED,
  readPending,
  savePending,
  startRedirect,
  TRY_AGAIN,
  userIdForEmail,
  organizationsOf,
  type Outcome,
  STEP_PATH,
} from "./flow";
import { invitationProvesEmail, mayAcceptInvitation } from "./invite";
import { RateLimiter, TtlCache } from "./limits";
import { invalidateMemberships } from "./session";
import { checkCode, checkEmail, checkId, checkName, checkNewPassword, checkPassword, checkToken, sanitizeReturnTo } from "./validate";

export interface FormState {
  error?: string;
  /** Which field the error is about, so the form can focus it. */
  field?: string;
  notice?: string;
  /** Sign-in: the step after the email. */
  step?: "password" | "code";
  email?: string;
  /** Non-secret values to put back in the form after an error (never passwords or codes). */
  values?: Record<string, string>;
  /** The pending step is gone: the form offers a way back instead of a dead end. */
  expired?: boolean;
}

const DEMO: FormState = { error: "Demo session: WorkOS is not configured here, so nothing was sent. Open the dashboard instead." };
const isDemo = () => authMode() === "demo";

/** Ends an action: navigate, or show the message. */
function settle(o: Outcome, extra: FormState = {}): FormState {
  if (o.kind === "error") return { ...extra, error: o.message };
  redirect(o.to);
}

const str = (form: FormData, key: string) => {
  const v = form.get(key);
  return typeof v === "string" ? v : "";
};

const status = (e: unknown) => (e as { status?: number })?.status;

function invitationFrom(form: FormData): string | undefined {
  if (!form.get("invitationToken")) return undefined;
  const t = checkToken(form.get("invitationToken"));
  return t.ok ? t.value : undefined;
}

const codeSent = (email: string) => `We emailed a 6-digit code to ${email}. It works for 10 minutes.`;
const CODE_FAILED = "That code didn't work. Check it, or send a new one.";

// ---------------------------------------------------------------- Invitations as proof of the inbox

/** The invitation behind a token, or null (a lookup failure only means no shortcut). */
async function invitationFor(token: string | undefined) {
  if (!token) return null;
  try {
    return await getWorkOS().userManagement.findInvitationByToken(token);
  } catch (e) {
    console.warn("[quod-cloud] invitation lookup failed", { status: status(e) });
    return null;
  }
}

/**
 * Someone holding a live invitation for this email has read that inbox, so
 * the email-code step is redundant for them (WorkOS's own rule for AuthKit
 * invitations). When a correct password still lands on "verify your email"
 * (an account made before the invitation, or by an earlier attempt), mark
 * the email verified and sign in again. Anything unexpected keeps the code step.
 */
async function skipCodeForInvitee(first: Outcome, email: string, run: () => Promise<Outcome>): Promise<Outcome> {
  if (first.kind !== "pending" || first.to !== STEP_PATH["verify-email"]) return first;
  try {
    const userId = (await readPending())?.userId ?? (await userIdForEmail(email));
    if (!userId) return first;
    await getWorkOS().userManagement.updateUser({ userId, emailVerified: true });
  } catch (e) {
    console.warn("[quod-cloud] invited email not marked verified", { status: status(e) });
    return first;
  }
  return run();
}

// ---------------------------------------------------------------- Anonymous-traffic guards

// Per server instance (best effort on serverless); WorkOS still rate-limits for real.
const ssoByDomain = new TtlCache<string | null>(5 * 60_000, 5000);
const emailStepLimit = new RateLimiter(20, 60_000);
const codeLimit = new RateLimiter(5, 60_000);

async function ipKey(): Promise<string> {
  return (await clientInfo()).ipAddress ?? "unknown";
}

// ---------------------------------------------------------------- Sign in

/** Does this email's domain belong to an organization with an active SSO connection? Cached 5 minutes per domain. */
async function ssoOrganizationFor(email: string): Promise<string | null> {
  const domain = email.split("@")[1];
  const cached = ssoByDomain.get(domain);
  if (cached !== undefined) return cached;
  const workos = getWorkOS();
  let found: string | null = null;
  const orgs = await workos.organizations.listOrganizations({ domains: [domain], limit: 5 });
  for (const org of orgs.data) {
    const verified = org.domains.some((d) => d.domain.toLowerCase() === domain && d.state === "verified");
    if (!verified) continue;
    const connections = await workos.sso.listConnections({ organizationId: org.id, limit: 10 });
    if (connections.data.some((c) => c.state === "active")) {
      found = org.id;
      break;
    }
  }
  ssoByDomain.set(domain, found);
  return found;
}

/** Step 1: the email. SSO domains go straight to their identity provider. */
export async function continueWithEmail(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return { step: "password", email: email.value };
  if (!emailStepLimit.take(await ipKey())) return { error: RATE_LIMITED, field: "email" };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  let url: string | null = null;
  try {
    const org = await ssoOrganizationFor(email.value);
    if (org) url = await startRedirect({ organizationId: org }, returnTo, { loginHint: email.value });
  } catch (e) {
    // The lookup is a convenience: on failure, carry on to the password step (SSO-only orgs answer sso_required there).
    console.warn("[quod-cloud] SSO lookup failed", { status: status(e) });
  }
  if (url) redirect(url);
  return { step: "password", email: email.value };
}

export async function signInWithPassword(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email", step: "password" };
  const pw = checkPassword(form.get("password"));
  if (!pw.ok) return { error: pw.error, field: "password", step: "password", email: email.value };
  if (isDemo()) return { ...DEMO, step: "password", email: email.value };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitationToken = invitationFrom(form);
  const info = await clientInfo();
  const run = () =>
    attempt(
      () => getWorkOS().userManagement.authenticateWithPassword({ clientId: clientId(), email: email.value, password: pw.value, invitationToken, ...info }),
      { returnTo, email: email.value, invitationToken },
      "That email and password don't match. Try again, or sign in with a code.",
    );
  let outcome = await run();
  if (invitationToken && outcome.kind === "pending" && invitationProvesEmail(await invitationFor(invitationToken), email.value)) {
    outcome = await skipCodeForInvitee(outcome, email.value, run);
  }
  return settle(outcome, { step: "password", email: email.value, field: "password" });
}

/** Magic Auth: email a one-time code (it also creates the account if there is none). */
export async function sendSignInCode(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return { ...DEMO, step: "password", email: email.value };
  if (!codeLimit.take(await ipKey())) return { error: RATE_LIMITED, step: "code", email: email.value };
  try {
    await getWorkOS().userManagement.createMagicAuth({ email: email.value, invitationToken: invitationFrom(form), ...(await clientInfo()) });
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED, step: "password", email: email.value };
    console.warn("[quod-cloud] magic auth code not sent", { status: status(e) });
    return { error: TRY_AGAIN, step: "password", email: email.value };
  }
  return { step: "code", email: email.value, notice: codeSent(email.value) };
}

export async function signInWithCode(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code", step: "code", email: email.value };
  if (isDemo()) return { ...DEMO, step: "code", email: email.value };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitationToken = invitationFrom(form);
  const info = await clientInfo();
  const outcome = await attempt(
    () => getWorkOS().userManagement.authenticateWithMagicAuth({ clientId: clientId(), email: email.value, code: code.value, invitationToken, ...info }),
    { returnTo, email: email.value, invitationToken },
    "That code didn't work. Check it, or send a new one.",
  );
  return settle(outcome, { step: "code", email: email.value, field: "code" });
}

/** The sign-in form: one action, the button pressed says which step ran. */
export async function signIn(prev: FormState, form: FormData): Promise<FormState> {
  switch (str(form, "intent")) {
    case "email":
      return continueWithEmail(prev, form);
    case "password":
      return signInWithPassword(prev, form);
    case "send-code":
      return sendSignInCode(prev, form);
    case "code":
      return signInWithCode(prev, form);
    default:
      return { error: GENERIC };
  }
}

const PROVIDERS = { github: "GitHubOAuth", google: "GoogleOAuth" } as const;

/** "Continue with GitHub / Google". */
export async function continueWithProvider(_prev: FormState, form: FormData): Promise<FormState> {
  const provider = PROVIDERS[str(form, "provider") as keyof typeof PROVIDERS];
  if (!provider) return { error: GENERIC };
  if (isDemo()) return DEMO;
  let url: string;
  try {
    url = await startRedirect({ provider }, sanitizeReturnTo(form.get("returnTo")), { invitationToken: invitationFrom(form) });
  } catch (e) {
    console.warn("[quod-cloud] could not start provider sign-in", { status: status(e) });
    return { error: TRY_AGAIN };
  }
  redirect(url);
}

// ---------------------------------------------------------------- Sign up

/**
 * New account: create the user, then sign in (which usually asks for the email
 * code). A taken email lands on the same "check your email" page: unless the
 * password is already theirs, we email that address a Magic Auth code, which
 * the same page accepts. Only the inbox owner can tell the difference.
 */
export async function signUp(_prev: FormState, form: FormData): Promise<FormState> {
  const values = { firstName: str(form, "firstName").slice(0, 100), lastName: str(form, "lastName").slice(0, 100), email: str(form, "email").slice(0, 254) };
  const first = checkName(form.get("firstName"), "first name");
  if (!first.ok) return { error: first.error, field: "firstName", values };
  const last = checkName(form.get("lastName"), "last name");
  if (!last.ok) return { error: last.error, field: "lastName", values };
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email", values };
  const pw = checkNewPassword(form.get("password"));
  if (!pw.ok) return { error: pw.error, field: "password", values };
  if (isDemo()) return { ...DEMO, values };
  if (!emailStepLimit.take(await ipKey())) return { error: RATE_LIMITED, values };

  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitationToken = invitationFrom(form);
  const info = await clientInfo();
  const um = getWorkOS().userManagement;
  const ctx = { returnTo, email: email.value, invitationToken };
  const signInNow = () => um.authenticateWithPassword({ clientId: clientId(), email: email.value, password: pw.value, invitationToken, ...info });
  // Invited: the invitation email already proved the inbox, so no second code (see invite.ts).
  const invited = invitationProvesEmail(await invitationFor(invitationToken), email.value);

  let exists: boolean;
  try {
    exists = (await userIdForEmail(email.value)) !== undefined;
  } catch (e) {
    console.warn("[quod-cloud] sign-up lookup failed", { status: status(e) });
    return { error: status(e) === 429 ? RATE_LIMITED : TRY_AGAIN, values };
  }

  if (!exists) {
    try {
      await um.createUser({ email: email.value, password: pw.value, firstName: first.value, lastName: last.value, ...(invited ? { emailVerified: true } : {}), ...info });
    } catch (e) {
      if (status(e) === 429) return { error: RATE_LIMITED, values };
      // Most often the password policy (WorkOS checks breached and weak passwords).
      console.warn("[quod-cloud] create user failed", { status: status(e), code: (e as { code?: string })?.code });
      return { error: "We couldn't create the account with that password. Try a longer one that you don't use anywhere else.", field: "password", values };
    }
    return settle(await attempt(signInNow, ctx), { values });
  }

  // Taken email. Their own password simply signs them in; anything else gets the neutral code page.
  let tried = await attempt(signInNow, ctx);
  if (invited) tried = await skipCodeForInvitee(tried, email.value, () => attempt(signInNow, ctx));
  if (tried.kind !== "error") return settle(tried, { values });
  try {
    await um.createMagicAuth({ email: email.value, invitationToken, ...info });
  } catch (e) {
    console.warn("[quod-cloud] magic auth code not sent", { status: status(e) });
  }
  await savePending({ step: "verify-email", via: "magic", token: "", returnTo, email: email.value, invitationToken });
  redirect(STEP_PATH["verify-email"]);
}

// ---------------------------------------------------------------- Pending steps

const EXPIRED: FormState = { error: "This step expired, so that code can't be used here any more. Start again: sign in with your email, or open your invitation link again.", expired: true };

/**
 * The code from the email. The newest code is a Magic Auth code once "Send a
 * new code" was used; the first email's verification code still works while
 * WorkOS keeps its pending token, so both are tried, newest first.
 */
export async function verifyEmail(_prev: FormState, form: FormData): Promise<FormState> {
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code" };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending || pending.step !== "verify-email") return EXPIRED;
  const info = await clientInfo();
  const um = getWorkOS().userManagement;
  const ctx = { returnTo: pending.returnTo, email: pending.email, invitationToken: pending.invitationToken };
  const byMagic = () => attempt(() => um.authenticateWithMagicAuth({ clientId: clientId(), email: pending.email ?? "", code: code.value, invitationToken: pending.invitationToken, ...info }), ctx, CODE_FAILED);
  const byVerification = () => attempt(() => um.authenticateWithEmailVerification({ clientId: clientId(), code: code.value, pendingAuthenticationToken: pending.token, ...info }), ctx, CODE_FAILED);
  let outcome: Outcome;
  if (pending.via === "magic") {
    outcome = await byMagic();
    // Only a plain "wrong code" falls back; a rate limit or a server error is reported as is.
    if (outcome.kind === "error" && outcome.message === CODE_FAILED && pending.token) outcome = await byVerification();
  } else {
    outcome = await byVerification();
  }
  return settle(outcome, { field: "code" });
}

/**
 * "Send a new code". A Magic Auth code: it needs only the email (not WorkOS's
 * pending token, which can expire before the person gets back from their
 * inbox), it carries the invitation, and signing in with it verifies the email.
 * If Magic Auth is refused, fall back to WorkOS's verification email.
 */
export async function resendVerification(): Promise<FormState> {
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending?.email || pending.step !== "verify-email") return EXPIRED;
  if (!codeLimit.take(await ipKey())) return { error: RATE_LIMITED };
  const um = getWorkOS().userManagement;
  const sent = { notice: `A new code is on its way to ${pending.email}. Use the newest one; it works for 10 minutes.` };
  try {
    await um.createMagicAuth({ email: pending.email, invitationToken: pending.invitationToken, ...(await clientInfo()) });
    // Re-sealing also restarts the step's clock (setSealed sets `purpose` again).
    await savePending({ ...pending, via: "magic" });
    return sent;
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED };
    console.warn("[quod-cloud] magic auth code not sent for verification", { status: status(e) });
  }
  if (pending.via === "magic" || !pending.token) return { error: TRY_AGAIN };
  try {
    // WorkOS's email_verification_required answer does not always name the user; look it up by email.
    const userId = pending.userId ?? (await userIdForEmail(pending.email));
    if (!userId) return { error: TRY_AGAIN };
    await um.sendVerificationEmail({ userId });
    // Re-sealing restarts the step's clock, as on the Magic Auth path.
    await savePending(pending);
    return sent;
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED };
    console.warn("[quod-cloud] verification email not resent", { status: status(e) });
    return { error: TRY_AGAIN };
  }
}

export async function chooseOrganization(_prev: FormState, form: FormData): Promise<FormState> {
  const org = checkId(form.get("organizationId"), "org");
  if (!org.ok) return { error: org.error };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending?.userId || pending.step !== "select-org") return EXPIRED;
  try {
    if (!(await organizationsOf(pending.userId)).some((o) => o.id === org.value)) return { error: "Choose one of the options." };
  } catch (e) {
    console.warn("[quod-cloud] membership check failed", { status: status(e) });
    return { error: TRY_AGAIN };
  }
  const info = await clientInfo();
  const outcome = await attempt(
    () => getWorkOS().userManagement.authenticateWithOrganizationSelection({ clientId: clientId(), organizationId: org.value, pendingAuthenticationToken: pending.token, ...info }),
    { returnTo: pending.returnTo, email: pending.email },
  );
  return settle(outcome);
}

export async function verifyTotp(_prev: FormState, form: FormData): Promise<FormState> {
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code" };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending?.challengeId || (pending.step !== "mfa-challenge" && pending.step !== "mfa-enroll")) return EXPIRED;
  const challengeId = pending.challengeId;
  const info = await clientInfo();
  const outcome = await attempt(
    () => getWorkOS().userManagement.authenticateWithTotp({ clientId: clientId(), code: code.value, pendingAuthenticationToken: pending.token, authenticationChallengeId: challengeId, ...info }),
    { returnTo: pending.returnTo, email: pending.email },
    "That code didn't work. Codes change every 30 seconds; try the current one.",
  );
  return settle(outcome, { field: "code" });
}

/** Leaves a pending step. An invited person goes back to their invitation, not to a sign-in page that has forgotten it. */
export async function startOver(): Promise<void> {
  const invitation = (await readPending())?.invitationToken;
  await clearPending();
  const token = invitation ? checkToken(invitation) : null;
  redirect(token?.ok ? `/invite?invitation_token=${encodeURIComponent(token.value)}` : "/sign-in");
}

// ---------------------------------------------------------------- Joining a team while signed in

/**
 * A signed-in person accepts an invitation addressed to their own verified
 * email (the invite page while signed in, or "/" with no team yet). WorkOS
 * asks custom flows to check that match before accepting; this does, from
 * WorkOS's copy of the invitation, never from the form.
 */
export async function joinTeam(_prev: FormState, form: FormData): Promise<FormState> {
  const id = checkId(form.get("invitationId"), "invitation");
  if (!id.ok) return { error: "This invitation can't be used. Ask for a new one." };
  if (isDemo()) return DEMO;
  const { user } = await withAuth();
  if (!user) redirect("/sign-in");
  const um = getWorkOS().userManagement;
  let organizationId: string;
  try {
    const inv = await um.getInvitation(id.value);
    if (!mayAcceptInvitation(inv, user) || !inv.organizationId) {
      return { error: "This invitation is for a different email, or it has expired or been used. Ask for a new one." };
    }
    organizationId = inv.organizationId;
    await um.acceptInvitation(inv.id);
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED };
    console.warn("[quod-cloud] invitation not accepted", { status: status(e) });
    return { error: TRY_AGAIN };
  }
  invalidateMemberships(user.id);
  // Scope the session to the new team, then "/" opens it.
  await switchToOrganization(organizationId, { returnTo: "/" });
  redirect("/");
}

// ---------------------------------------------------------------- Password reset

export async function requestPasswordReset(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return DEMO;
  if (!codeLimit.take(await ipKey())) return { error: RATE_LIMITED, field: "email" };
  try {
    await getWorkOS().userManagement.createPasswordReset({ email: email.value });
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED };
    // No account, or anything else: the answer below is the same.
    console.warn("[quod-cloud] password reset not created", { status: status(e) });
  }
  return { notice: `If ${email.value} has an account, a link to reset its password is on its way. It works for 15 minutes.` };
}

export async function resetPassword(_prev: FormState, form: FormData): Promise<FormState> {
  const token = checkToken(form.get("token"));
  if (!token.ok) return { error: token.error };
  const pw = checkNewPassword(form.get("password"));
  if (!pw.ok) return { error: pw.error, field: "password" };
  if (isDemo()) return DEMO;
  const um = getWorkOS().userManagement;
  let email: string;
  try {
    email = (await um.resetPassword({ token: token.value, newPassword: pw.value })).user.email;
  } catch (e) {
    if (status(e) === 429) return { error: RATE_LIMITED };
    console.warn("[quod-cloud] password reset failed", { status: status(e) });
    return { error: "This link expired or was already used, or the password was refused. Ask for a new link, or try a longer password." };
  }
  const info = await clientInfo();
  const outcome = await attempt(() => um.authenticateWithPassword({ clientId: clientId(), email, password: pw.value, ...info }), { returnTo: "/", email });
  if (outcome.kind === "error") redirect("/sign-in?reset=1");
  return settle(outcome);
}
