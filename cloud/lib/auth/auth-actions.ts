"use server";

// Server actions behind our sign-in pages. Every input is checked here (the
// forms are only a convenience), errors are generic so they never say whether
// an account exists, returnTo is always a same-origin path, and no password,
// code or token is logged.

import { getWorkOS } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { authMode } from "./config";
import { clientId, clientInfo, fromError, GENERIC, readPending, startRedirect, type Outcome, finish, clearPending } from "./flow";
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
}

const DEMO: FormState = { error: "Demo session: WorkOS is not configured here, so nothing was sent. Open the dashboard instead." };
const isDemo = () => authMode() === "demo";

/** Ends a successful action: navigate, or show the message. */
function settle(o: Outcome, extra: FormState = {}): FormState {
  if (o.kind === "error") return { ...extra, error: o.message };
  redirect(o.to);
}

const str = (form: FormData, key: string) => {
  const v = form.get(key);
  return typeof v === "string" ? v : "";
};

// ---------------------------------------------------------------- Sign in

/** Does this email's domain belong to an organization with an active SSO connection? */
async function ssoOrganizationFor(email: string): Promise<string | null> {
  const domain = email.split("@")[1];
  const workos = getWorkOS();
  const orgs = await workos.organizations.listOrganizations({ domains: [domain], limit: 5 });
  for (const org of orgs.data) {
    const verified = org.domains.some((d) => d.domain.toLowerCase() === domain && d.state === "verified");
    if (!verified) continue;
    const connections = await workos.sso.listConnections({ organizationId: org.id, limit: 10 });
    if (connections.data.some((c) => c.state === "active")) return org.id;
  }
  return null;
}

/** Step 1: the email. SSO domains go straight to their identity provider. */
export async function continueWithEmail(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return { step: "password", email: email.value };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  let org: string | null = null;
  try {
    org = await ssoOrganizationFor(email.value);
  } catch (e) {
    console.warn("[quod-cloud] SSO lookup failed", { status: (e as { status?: number })?.status });
  }
  if (org) redirect(await startRedirect({ organizationId: org }, returnTo, { loginHint: email.value }));
  return { step: "password", email: email.value };
}

export async function signInWithPassword(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email", step: "password" };
  const pw = checkPassword(form.get("password"));
  if (!pw.ok) return { error: pw.error, field: "password", step: "password", email: email.value };
  if (isDemo()) return { ...DEMO, step: "password", email: email.value };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitation = form.get("invitationToken") ? checkToken(form.get("invitationToken")) : null;
  const invitationToken = invitation?.ok ? invitation.value : undefined;
  let outcome: Outcome;
  try {
    const res = await getWorkOS().userManagement.authenticateWithPassword({
      clientId: clientId(),
      email: email.value,
      password: pw.value,
      invitationToken,
      ...(await clientInfo()),
    });
    outcome = await finish(res, returnTo);
  } catch (e) {
    outcome = await fromError(e, { returnTo, email: email.value, invitationToken }, "That email and password don't match. Try again, or sign in with a code.");
  }
  return settle(outcome, { step: "password", email: email.value, field: "password" });
}

/** Magic Auth: email a one-time code. The answer is the same whether or not the account exists. */
export async function sendSignInCode(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return { ...DEMO, step: "password", email: email.value };
  const invitation = form.get("invitationToken") ? checkToken(form.get("invitationToken")) : null;
  try {
    await getWorkOS().userManagement.createMagicAuth({
      email: email.value,
      invitationToken: invitation?.ok ? invitation.value : undefined,
      ...(await clientInfo()),
    });
  } catch (e) {
    const status = (e as { status?: number })?.status;
    if (status === 429) return { error: "Too many codes asked for. Wait a minute, then try again.", step: "password", email: email.value };
    console.warn("[quod-cloud] magic auth code not sent", { status });
  }
  return { step: "code", email: email.value, notice: `If ${email.value} can sign in, a 6-digit code is on its way. It works for 10 minutes.` };
}

export async function signInWithCode(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code", step: "code", email: email.value };
  if (isDemo()) return { ...DEMO, step: "code", email: email.value };
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitation = form.get("invitationToken") ? checkToken(form.get("invitationToken")) : null;
  const invitationToken = invitation?.ok ? invitation.value : undefined;
  let outcome: Outcome;
  try {
    const res = await getWorkOS().userManagement.authenticateWithMagicAuth({
      clientId: clientId(),
      email: email.value,
      code: code.value,
      invitationToken,
      ...(await clientInfo()),
    });
    outcome = await finish(res, returnTo);
  } catch (e) {
    outcome = await fromError(e, { returnTo, email: email.value, invitationToken }, "That code didn't work. Check it, or send a new one.");
  }
  return settle(outcome, { step: "code", email: email.value, field: "code" });
}

const PROVIDERS = { github: "GitHubOAuth", google: "GoogleOAuth" } as const;

/** "Continue with GitHub / Google". */
export async function continueWithProvider(_prev: FormState, form: FormData): Promise<FormState> {
  const provider = PROVIDERS[str(form, "provider") as keyof typeof PROVIDERS];
  if (!provider) return { error: GENERIC };
  if (isDemo()) return DEMO;
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitation = form.get("invitationToken") ? checkToken(form.get("invitationToken")) : null;
  redirect(await startRedirect({ provider }, returnTo, { invitationToken: invitation?.ok ? invitation.value : undefined }));
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

// ---------------------------------------------------------------- Sign up

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
  const returnTo = sanitizeReturnTo(form.get("returnTo"));
  const invitation = form.get("invitationToken") ? checkToken(form.get("invitationToken")) : null;
  const invitationToken = invitation?.ok ? invitation.value : undefined;
  const info = await clientInfo();
  const workos = getWorkOS();

  try {
    await workos.userManagement.createUser({ email: email.value, password: pw.value, firstName: first.value, lastName: last.value, ...info });
  } catch (e) {
    const status = (e as { status?: number })?.status;
    if (status === 429) return { error: "Too many attempts. Wait a minute, then try again." };
    // An existing account is not announced: carry on, and the sign-in below either works or fails generically.
    console.warn("[quod-cloud] create user failed", { status, code: (e as { code?: string })?.code });
  }

  let outcome: Outcome;
  try {
    const res = await workos.userManagement.authenticateWithPassword({ clientId: clientId(), email: email.value, password: pw.value, invitationToken, ...info });
    outcome = await finish(res, returnTo);
  } catch (e) {
    outcome = await fromError(
      e,
      { returnTo, email: email.value, invitationToken },
      "We couldn't create that account. If you already have one, sign in or reset your password.",
    );
  }
  return settle(outcome, { values });
}

// ---------------------------------------------------------------- Pending steps

export async function verifyEmail(_prev: FormState, form: FormData): Promise<FormState> {
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code" };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending || pending.step !== "verify-email") return { error: "This step expired. Sign in again." };
  let outcome: Outcome;
  try {
    const res = await getWorkOS().userManagement.authenticateWithEmailVerification({
      clientId: clientId(),
      code: code.value,
      pendingAuthenticationToken: pending.token,
      ...(await clientInfo()),
    });
    outcome = await finish(res, pending.returnTo);
  } catch (e) {
    outcome = await fromError(e, { returnTo: pending.returnTo, email: pending.email }, "That code didn't work. Check it, or send a new one.");
  }
  return settle(outcome, { field: "code" });
}

export async function resendVerification(): Promise<FormState> {
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending?.userId || pending.step !== "verify-email") return { error: "This step expired. Sign in again." };
  try {
    await getWorkOS().userManagement.sendVerificationEmail({ userId: pending.userId });
  } catch (e) {
    const status = (e as { status?: number })?.status;
    if (status === 429) return { error: "Too many codes asked for. Wait a minute, then try again." };
    console.warn("[quod-cloud] verification email not sent", { status });
  }
  return { notice: "A new code is on its way." };
}

export async function chooseOrganization(_prev: FormState, form: FormData): Promise<FormState> {
  const org = checkId(form.get("organizationId"), "org");
  if (!org.ok) return { error: org.error };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending || pending.step !== "select-org") return { error: "This step expired. Sign in again." };
  if (!pending.organizations?.some((o) => o.id === org.value)) return { error: "Choose one of the options." };
  let outcome: Outcome;
  try {
    const res = await getWorkOS().userManagement.authenticateWithOrganizationSelection({
      clientId: clientId(),
      organizationId: org.value,
      pendingAuthenticationToken: pending.token,
      ...(await clientInfo()),
    });
    outcome = await finish(res, pending.returnTo);
  } catch (e) {
    outcome = await fromError(e, { returnTo: pending.returnTo, email: pending.email });
  }
  return settle(outcome);
}

export async function verifyTotp(_prev: FormState, form: FormData): Promise<FormState> {
  const code = checkCode(form.get("code"));
  if (!code.ok) return { error: code.error, field: "code" };
  if (isDemo()) return DEMO;
  const pending = await readPending();
  if (!pending?.challengeId || (pending.step !== "mfa-challenge" && pending.step !== "mfa-enroll")) return { error: "This step expired. Sign in again." };
  let outcome: Outcome;
  try {
    const res = await getWorkOS().userManagement.authenticateWithTotp({
      clientId: clientId(),
      code: code.value,
      pendingAuthenticationToken: pending.token,
      authenticationChallengeId: pending.challengeId,
      ...(await clientInfo()),
    });
    outcome = await finish(res, pending.returnTo);
  } catch (e) {
    outcome = await fromError(e, { returnTo: pending.returnTo, email: pending.email }, "That code didn't work. Codes change every 30 seconds; try the current one.");
  }
  return settle(outcome, { field: "code" });
}

export async function startOver(): Promise<void> {
  await clearPending();
  redirect("/sign-in");
}

// ---------------------------------------------------------------- Password reset

export async function requestPasswordReset(_prev: FormState, form: FormData): Promise<FormState> {
  const email = checkEmail(form.get("email"));
  if (!email.ok) return { error: email.error, field: "email" };
  if (isDemo()) return DEMO;
  try {
    await getWorkOS().userManagement.createPasswordReset({ email: email.value });
  } catch (e) {
    const status = (e as { status?: number })?.status;
    if (status === 429) return { error: "Too many attempts. Wait a minute, then try again." };
    // No account, or anything else: the answer below is the same.
    console.warn("[quod-cloud] password reset not created", { status });
  }
  return { notice: `If ${email.value} has an account, a link to reset its password is on its way. It works for 15 minutes.` };
}

export async function resetPassword(_prev: FormState, form: FormData): Promise<FormState> {
  const token = checkToken(form.get("token"));
  if (!token.ok) return { error: token.error };
  const pw = checkNewPassword(form.get("password"));
  if (!pw.ok) return { error: pw.error, field: "password" };
  if (isDemo()) return DEMO;
  const workos = getWorkOS();
  let email: string;
  try {
    const { user } = await workos.userManagement.resetPassword({ token: token.value, newPassword: pw.value });
    email = user.email;
  } catch (e) {
    const status = (e as { status?: number })?.status;
    if (status === 429) return { error: "Too many attempts. Wait a minute, then try again." };
    console.warn("[quod-cloud] password reset failed", { status });
    return { error: "This link expired or was already used. Ask for a new one." };
  }
  let outcome: Outcome;
  try {
    const res = await workos.userManagement.authenticateWithPassword({ clientId: clientId(), email, password: pw.value, ...(await clientInfo()) });
    outcome = await finish(res, "/");
  } catch (e) {
    outcome = await fromError(e, { returnTo: "/", email });
    if (outcome.kind === "error") redirect("/sign-in?reset=1");
  }
  return settle(outcome);
}
