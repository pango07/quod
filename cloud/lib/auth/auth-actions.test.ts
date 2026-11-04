import { AuthenticationException } from "@workos-inc/node";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The sign-in server actions with WorkOS, AuthKit, next/headers and
// next/navigation mocked: a cookie jar in memory, redirect() throws.
const jar = new Map<string, string>();
const um = {
  listUsers: vi.fn(),
  createUser: vi.fn(),
  updateUser: vi.fn(),
  authenticateWithPassword: vi.fn(),
  authenticateWithEmailVerification: vi.fn(),
  authenticateWithMagicAuth: vi.fn(),
  createMagicAuth: vi.fn(),
  sendVerificationEmail: vi.fn(),
  findInvitationByToken: vi.fn(),
  getInvitation: vi.fn(),
  acceptInvitation: vi.fn(),
};
const saveSession = vi.fn();
const withAuth = vi.fn();
const switchToOrganization = vi.fn();
const invalidateMemberships = vi.fn();

class Redirect extends Error {
  constructor(readonly to: string) {
    super(`redirect ${to}`);
  }
}

vi.mock("server-only", () => ({}));
vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Redirect(to);
  },
  unstable_rethrow: (e: unknown) => {
    if (e instanceof Redirect) throw e;
  },
}));
vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: (n: string) => (jar.has(n) ? { value: jar.get(n) } : undefined),
    set: (n: string, v: string) => void jar.set(n, v),
    delete: (n: string) => void jar.delete(n),
  }),
  headers: async () => new Headers({ "x-forwarded-for": "203.0.113.9", "user-agent": "test" }),
}));
vi.mock("@workos-inc/authkit-nextjs", () => ({
  saveSession: (...a: unknown[]) => saveSession(...a),
  withAuth: (...a: unknown[]) => withAuth(...a),
  switchToOrganization: (...a: unknown[]) => switchToOrganization(...a),
  getWorkOS: () => ({ userManagement: um, multiFactorAuth: {} }),
}));
vi.mock("./session", () => ({ invalidateMemberships: (...a: unknown[]) => invalidateMemberships(...a) }));

process.env.WORKOS_API_KEY = "sk_test_x";
process.env.WORKOS_CLIENT_ID = "client_x";
process.env.WORKOS_COOKIE_PASSWORD = "p".repeat(32);
process.env.NEXT_PUBLIC_WORKOS_REDIRECT_URI = "https://app.example.com/callback";

const actions = await import("./auth-actions");
const flow = await import("./flow");

const TEAMMATE = "teammate@example.com";
const TOKEN = "inviteTokenABC123";
const invitation = { id: "invitation_01ABCDEFGHJK", email: TEAMMATE, state: "pending", expiresAt: new Date(Date.now() + 86_400_000).toISOString(), organizationId: "org_01ABCDEFGHJK" };
const signedIn = { user: { id: "user_1", email: TEAMMATE }, accessToken: "at", refreshToken: "rt", authenticationMethod: "Password" };
const verificationRequired = () => new AuthenticationException(403, { code: "email_verification_required", email: TEAMMATE, email_verification_id: "ev_1", pending_authentication_token: "pending_tok" } as never, "req_1");
const wrongCode = () => Object.assign(new Error("invalid_grant"), { status: 400, code: "invalid_grant" });

function form(fields: Record<string, string>): FormData {
  const f = new FormData();
  for (const [k, v] of Object.entries(fields)) f.set(k, v);
  return f;
}
const signUpForm = (extra: Record<string, string> = {}) => form({ firstName: "Sam", lastName: "Lee", email: TEAMMATE, password: "a long enough password", returnTo: "/", ...extra });

/** Runs an action and returns where it redirected, or its form state. */
async function run<T>(p: Promise<T>): Promise<{ to: string } | { state: T }> {
  try {
    return { state: await p };
  } catch (e) {
    if (e instanceof Redirect) return { to: e.to };
    throw e;
  }
}

beforeEach(() => {
  jar.clear();
  for (const f of [...Object.values(um), saveSession, withAuth, switchToOrganization, invalidateMemberships]) f.mockReset();
  saveSession.mockResolvedValue(undefined);
});
afterEach(() => vi.useRealTimers());

describe("sign-up from an invitation", () => {
  it("creates the invited teammate with the email already verified, so there is no code step", async () => {
    um.listUsers.mockResolvedValue({ data: [] });
    um.findInvitationByToken.mockResolvedValue(invitation);
    um.authenticateWithPassword.mockResolvedValue(signedIn);
    expect(await run(actions.signUp({}, signUpForm({ invitationToken: TOKEN })))).toEqual({ to: "/" });
    expect(um.createUser).toHaveBeenCalledWith(expect.objectContaining({ email: TEAMMATE, emailVerified: true }));
    expect(um.authenticateWithPassword).toHaveBeenCalledWith(expect.objectContaining({ invitationToken: TOKEN }));
  });

  it("keeps the code step when the invitation is for another email, used, or can't be read", async () => {
    for (const lookup of [() => um.findInvitationByToken.mockResolvedValue({ ...invitation, email: "other@example.com" }), () => um.findInvitationByToken.mockResolvedValue({ ...invitation, state: "accepted" }), () => um.findInvitationByToken.mockRejectedValue(Object.assign(new Error("nf"), { status: 404 }))]) {
      jar.clear();
      um.createUser.mockReset();
      um.listUsers.mockResolvedValue({ data: [] });
      lookup();
      um.authenticateWithPassword.mockRejectedValue(verificationRequired());
      expect(await run(actions.signUp({}, signUpForm({ invitationToken: TOKEN })))).toEqual({ to: "/verify-email" });
      expect(um.createUser.mock.calls[0][0]).not.toHaveProperty("emailVerified");
    }
  });

  it("never marks an email verified without an invitation", async () => {
    um.listUsers.mockResolvedValue({ data: [] });
    um.authenticateWithPassword.mockRejectedValue(verificationRequired());
    expect(await run(actions.signUp({}, signUpForm()))).toEqual({ to: "/verify-email" });
    expect(um.findInvitationByToken).not.toHaveBeenCalled();
    expect(um.createUser.mock.calls[0][0]).not.toHaveProperty("emailVerified");
    expect(um.updateUser).not.toHaveBeenCalled();
  });

  it("an account left unverified by an earlier try: the right password plus the invitation verifies it and signs in", async () => {
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    um.findInvitationByToken.mockResolvedValue(invitation);
    um.authenticateWithPassword.mockRejectedValueOnce(verificationRequired()).mockResolvedValueOnce(signedIn);
    expect(await run(actions.signUp({}, signUpForm({ invitationToken: TOKEN })))).toEqual({ to: "/" });
    expect(um.updateUser).toHaveBeenCalledWith({ userId: "user_1", emailVerified: true });
    expect(um.createUser).not.toHaveBeenCalled();
  });

  it("an existing account with a wrong password is never marked verified", async () => {
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    um.findInvitationByToken.mockResolvedValue(invitation);
    um.authenticateWithPassword.mockRejectedValue(Object.assign(new Error("invalid_credentials"), { status: 400 }));
    expect(await run(actions.signUp({}, signUpForm({ invitationToken: TOKEN })))).toEqual({ to: "/verify-email" });
    expect(um.updateUser).not.toHaveBeenCalled();
    expect(um.createMagicAuth).toHaveBeenCalledWith(expect.objectContaining({ email: TEAMMATE, invitationToken: TOKEN }));
  });

  it("a stale code-step cookie for another email never gets its user marked verified", async () => {
    await flow.savePending({ step: "verify-email", via: "verification", token: "old_tok", returnTo: "/", email: "someone@example.com", userId: "user_other" });
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    um.findInvitationByToken.mockResolvedValue(invitation);
    um.authenticateWithPassword.mockRejectedValueOnce(verificationRequired()).mockResolvedValueOnce(signedIn);
    expect(await run(actions.signUp({}, signUpForm({ invitationToken: TOKEN })))).toEqual({ to: "/" });
    expect(um.updateUser).toHaveBeenCalledWith({ userId: "user_1", emailVerified: true });
    expect(um.updateUser).not.toHaveBeenCalledWith(expect.objectContaining({ userId: "user_other" }));
  });

  it("signing in to accept (existing account, unverified) skips the code too", async () => {
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    um.findInvitationByToken.mockResolvedValue(invitation);
    um.authenticateWithPassword.mockRejectedValueOnce(verificationRequired()).mockResolvedValueOnce(signedIn);
    expect(await run(actions.signInWithPassword({}, form({ email: TEAMMATE, password: "pw", returnTo: "/", invitationToken: TOKEN })))).toEqual({ to: "/" });
    expect(um.updateUser).toHaveBeenCalledWith({ userId: "user_1", emailVerified: true });
  });
});

describe("the email code step", () => {
  const pendingVerification = () => flow.savePending({ step: "verify-email", via: "verification", token: "pending_tok", returnTo: "/", email: TEAMMATE, invitationToken: TOKEN });

  it("'Send a new code' sends a Magic Auth code that carries the invitation, and the step remembers it", async () => {
    await pendingVerification();
    const out = await actions.resendVerification();
    expect(out.notice).toContain(TEAMMATE);
    expect(um.createMagicAuth).toHaveBeenCalledWith(expect.objectContaining({ email: TEAMMATE, invitationToken: TOKEN }));
    expect(um.sendVerificationEmail).not.toHaveBeenCalled();
    expect(await flow.readPending()).toMatchObject({ via: "magic", token: "pending_tok", invitationToken: TOKEN });
  });

  it("falls back to WorkOS's verification email when Magic Auth is refused", async () => {
    await pendingVerification();
    um.createMagicAuth.mockRejectedValue(Object.assign(new Error("magic auth disabled"), { status: 422 }));
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    expect((await actions.resendVerification()).notice).toBeDefined();
    expect(um.sendVerificationEmail).toHaveBeenCalledWith({ userId: "user_1" });
  });

  it("falls back to the verification email even after an earlier resend switched the step to Magic Auth", async () => {
    await flow.savePending({ step: "verify-email", via: "magic", token: "pending_tok", returnTo: "/", email: TEAMMATE, invitationToken: TOKEN });
    um.createMagicAuth.mockRejectedValue(Object.assign(new Error("magic auth disabled"), { status: 422 }));
    um.listUsers.mockResolvedValue({ data: [{ id: "user_1" }] });
    expect((await actions.resendVerification()).notice).toBeDefined();
    expect(um.sendVerificationEmail).toHaveBeenCalledWith({ userId: "user_1" });
  });

  it("after a resend, the new code signs in, and the first email's code still works", async () => {
    await pendingVerification();
    await actions.resendVerification();
    um.authenticateWithMagicAuth.mockResolvedValueOnce(signedIn);
    expect(await run(actions.verifyEmail({}, form({ code: "123 456" })))).toEqual({ to: "/" });
    expect(um.authenticateWithMagicAuth).toHaveBeenCalledWith(expect.objectContaining({ email: TEAMMATE, code: "123456", invitationToken: TOKEN }));

    await pendingVerification();
    await actions.resendVerification();
    um.authenticateWithMagicAuth.mockRejectedValueOnce(wrongCode());
    um.authenticateWithEmailVerification.mockResolvedValueOnce(signedIn);
    expect(await run(actions.verifyEmail({}, form({ code: "654321" })))).toEqual({ to: "/" });
    expect(um.authenticateWithEmailVerification).toHaveBeenCalledWith(expect.objectContaining({ code: "654321", pendingAuthenticationToken: "pending_tok" }));
  });

  it("a wrong code says so and keeps the step", async () => {
    await pendingVerification();
    um.authenticateWithEmailVerification.mockRejectedValue(wrongCode());
    expect(await run(actions.verifyEmail({}, form({ code: "000000" })))).toEqual({ state: { field: "code", error: "That code didn't work. Check it, or send a new one." } });
    expect(await flow.readPending()).not.toBeNull();
  });

  it("the step outlives a slow inbox: still there after 20 minutes, gone after 30", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Date.parse("2026-11-04T12:00:00Z"));
    await pendingVerification();
    vi.setSystemTime(Date.parse("2026-11-04T12:20:00Z"));
    expect(await flow.readPending()).toMatchObject({ step: "verify-email" });
    vi.setSystemTime(Date.parse("2026-11-04T12:31:00Z"));
    expect(await flow.readPending()).toBeNull();
  });

  it("an expired step says how to start again, and resending sends nothing", async () => {
    const verify = await actions.verifyEmail({}, form({ code: "123456" }));
    const resend = await actions.resendVerification();
    for (const s of [verify, resend]) expect(s).toMatchObject({ expired: true });
    expect(um.createMagicAuth).not.toHaveBeenCalled();
  });

  it("'Start over' from an invited step goes back to the invitation", async () => {
    await pendingVerification();
    expect(await run(actions.startOver())).toEqual({ to: `/invite?invitation_token=${TOKEN}` });
    expect(await flow.readPending()).toBeNull();
    expect(await run(actions.startOver())).toEqual({ to: "/sign-in" });
  });
});

describe("joining a team while signed in", () => {
  const join = () => run(actions.joinTeam({}, form({ invitationId: invitation.id })));

  it("accepts an invitation for the signed-in user's verified email and opens the team", async () => {
    withAuth.mockResolvedValue({ user: { id: "user_1", email: TEAMMATE, emailVerified: true } });
    um.getInvitation.mockResolvedValue(invitation);
    expect(await join()).toEqual({ to: "/" });
    expect(um.acceptInvitation).toHaveBeenCalledWith(invitation.id);
    expect(invalidateMemberships).toHaveBeenCalledWith("user_1");
    expect(switchToOrganization).toHaveBeenCalledWith(invitation.organizationId, { returnTo: "/" });
  });

  it("a failed team switch after joining says so instead of crashing; the membership stays", async () => {
    withAuth.mockResolvedValue({ user: { id: "user_1", email: TEAMMATE, emailVerified: true } });
    um.getInvitation.mockResolvedValue(invitation);
    switchToOrganization.mockRejectedValue(Object.assign(new Error("boom"), { status: 500 }));
    const out = await join();
    expect(out).toEqual({ state: { error: expect.stringContaining("You joined the team") } });
    expect(um.acceptInvitation).toHaveBeenCalledWith(invitation.id);
    expect(invalidateMemberships).toHaveBeenCalledWith("user_1");
  });

  it("refuses someone else's invitation, or an unverified email", async () => {
    for (const user of [{ id: "user_2", email: "owner@example.com", emailVerified: true }, { id: "user_1", email: TEAMMATE, emailVerified: false }]) {
      withAuth.mockResolvedValue({ user });
      um.getInvitation.mockResolvedValue(invitation);
      const out = await join();
      expect(out).toMatchObject({ state: { error: expect.stringContaining("different email") } });
    }
    expect(um.acceptInvitation).not.toHaveBeenCalled();
  });

  it("refuses a malformed id without asking WorkOS", async () => {
    expect(await run(actions.joinTeam({}, form({ invitationId: "../org_1" })))).toMatchObject({ state: { error: expect.any(String) } });
    expect(um.getInvitation).not.toHaveBeenCalled();
  });
});
