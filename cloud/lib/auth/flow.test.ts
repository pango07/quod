import { AuthenticationException } from "@workos-inc/node";
import { beforeEach, describe, expect, it, vi } from "vitest";

// flow.ts with WorkOS, AuthKit and next/headers mocked: a cookie jar in memory.
const jar = new Map<string, string>();
const saveSession = vi.fn();
const listUsers = vi.fn();
const createUserAuthFactor = vi.fn();

vi.mock("server-only", () => ({}));
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
  getWorkOS: () => ({
    userManagement: { listUsers: (...a: unknown[]) => listUsers(...a) },
    multiFactorAuth: { createUserAuthFactor: (...a: unknown[]) => createUserAuthFactor(...a) },
  }),
}));

process.env.WORKOS_COOKIE_PASSWORD = "p".repeat(32);
const flow = await import("./flow");
const { UNSUPPORTED_MESSAGE } = await import("./limits");

const authError = (rawData: Record<string, unknown>) => new AuthenticationException(403, rawData as never, "req_1");
const ctx = { returnTo: "/quodci", email: "dev@example.com" };
const response = {
  user: { id: "user_1", email: "dev@example.com" },
  accessToken: "at",
  refreshToken: "rt",
  authenticationMethod: "Password",
  oauthTokens: { accessToken: "github-token", refreshToken: "x", expiresAt: 0, scopes: [] },
} as never;

beforeEach(() => {
  jar.clear();
  saveSession.mockReset();
  listUsers.mockReset();
  createUserAuthFactor.mockReset();
});

describe("pending steps (review M1, m1)", () => {
  it("takes the email from an email_verification_required answer that has no user", async () => {
    const out = await flow.fromError(authError({ code: "email_verification_required", email: "new@example.com", email_verification_id: "ev_1", pending_authentication_token: "tok" }), ctx);
    expect(out).toEqual({ kind: "pending", to: "/verify-email" });
    expect(await flow.readPending()).toMatchObject({ step: "verify-email", email: "new@example.com", token: "tok", returnTo: "/quodci" });
  });

  it("finds the user by email when a step needs the id and WorkOS didn't send it", async () => {
    listUsers.mockResolvedValue({ data: [{ id: "user_9" }] });
    expect(await flow.userIdForEmail("new@example.com")).toBe("user_9");
    expect(listUsers).toHaveBeenCalledWith({ email: "new@example.com", limit: 1 });
  });

  it("answers steps we don't support with an explicit message, never 'wrong password'", async () => {
    for (const code of ["mfa_verification", "radar_email_challenge", "radar_sms_challenge"]) {
      const out = await flow.fromError(authError({ code, pending_authentication_token: "tok" }), ctx, "That email and password don't match.");
      expect(out).toEqual({ kind: "error", message: UNSUPPORTED_MESSAGE });
    }
  });

  it("keeps organization selection to the user id (the list is read when the page renders)", async () => {
    const orgs = Array.from({ length: 50 }, (_, i) => ({ id: `org_${"x".repeat(26)}${i}`, name: "A long organization name ".repeat(3) }));
    const out = await flow.fromError(authError({ code: "organization_selection_required", pending_authentication_token: "tok", user: { id: "user_1" }, organizations: orgs }), ctx);
    expect(out.kind).toBe("pending");
    const p = await flow.readPending();
    expect(p).toMatchObject({ step: "select-org", userId: "user_1" });
    expect(JSON.stringify(p)).not.toContain("organization name");
  });

  it("stores the otpauth URI for MFA enrollment, not the QR image", async () => {
    createUserAuthFactor.mockResolvedValue({
      authenticationFactor: { totp: { uri: "otpauth://totp/Quod:dev?secret=S", secret: "S", qrCode: `data:image/png;base64,${"A".repeat(4000)}` } },
      authenticationChallenge: { id: "auth_challenge_1" },
    });
    const out = await flow.fromError(authError({ code: "mfa_enrollment", pending_authentication_token: "tok", user: { id: "user_1", email: "dev@example.com" } }), ctx);
    expect(out.kind).toBe("pending");
    expect(await flow.readPending()).toMatchObject({ step: "mfa-enroll", challengeId: "auth_challenge_1", totp: { uri: "otpauth://totp/Quod:dev?secret=S", secret: "S" } });
    expect(JSON.stringify(await flow.readPending())).not.toContain("data:image");
  });

  it("turns a failure while starting a step into 'try again'", async () => {
    createUserAuthFactor.mockRejectedValue(new Error("network"));
    const out = await flow.fromError(authError({ code: "mfa_enrollment", pending_authentication_token: "tok", user: { id: "user_1" } }), ctx);
    expect(out).toEqual({ kind: "error", message: flow.TRY_AGAIN });
  });
});

describe("attempt (review m2, m3)", () => {
  it("seals only the session fields, never upstream OAuth tokens", async () => {
    const out = await flow.attempt(async () => response, ctx);
    expect(out).toEqual({ kind: "done", to: "/quodci" });
    const sealed = saveSession.mock.calls[0][0];
    expect(Object.keys(sealed).sort()).toEqual(["accessToken", "authenticationMethod", "impersonator", "refreshToken", "user"]);
    expect(JSON.stringify(sealed)).not.toContain("github-token");
  });

  it("says 'try again', not 'wrong password', when saving the session fails after WorkOS said yes", async () => {
    saveSession.mockRejectedValue(new Error("JWKS fetch failed"));
    const out = await flow.attempt(async () => response, ctx, "That email and password don't match.");
    expect(out).toEqual({ kind: "error", message: flow.TRY_AGAIN });
  });
});

describe("sealed cookies (review m6, m9)", () => {
  it("never accepts an OAuth blob as a pending step, or a state that doesn't match its cookie", async () => {
    // Seal an OAuth flow, then present its value as the pending cookie.
    const { sealData } = await import("iron-session");
    jar.set("quod-auth-pending", await sealData({ purpose: "quod-oauth", step: "verify-email", token: "t", returnTo: "/" }, { password: "p".repeat(32), ttl: 600 }));
    expect(await flow.readPending()).toBeNull();
    expect(await flow.takeOAuthState("not-a-uuid")).toBeNull();
    expect(await flow.takeOAuthState("3f2b8c1e-0000-4000-8000-000000000000")).toBeNull();
  });
});
