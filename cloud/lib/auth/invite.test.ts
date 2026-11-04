import { describe, expect, it } from "vitest";
import { invitationProvesEmail, mayAcceptInvitation } from "./invite";

const NOW = Date.parse("2026-11-04T12:00:00Z");
const inv = { email: "Teammate@Example.com", state: "pending", expiresAt: "2026-11-10T00:00:00Z", organizationId: "org_01ABCDEFGHJK" };

describe("invitationProvesEmail", () => {
  it("accepts a pending, unexpired invitation for the same email, ignoring case and spaces", () => {
    expect(invitationProvesEmail(inv, "teammate@example.com", NOW)).toBe(true);
    expect(invitationProvesEmail(inv, " TEAMMATE@example.com ", NOW)).toBe(true);
  });

  it("refuses another email, a used, revoked or expired invitation, and no invitation", () => {
    expect(invitationProvesEmail(inv, "someone@example.com", NOW)).toBe(false);
    for (const state of ["accepted", "revoked", "expired"]) expect(invitationProvesEmail({ ...inv, state }, "teammate@example.com", NOW)).toBe(false);
    expect(invitationProvesEmail({ ...inv, expiresAt: "2026-11-04T11:59:59Z" }, "teammate@example.com", NOW)).toBe(false);
    expect(invitationProvesEmail({ ...inv, expiresAt: null }, "teammate@example.com", NOW)).toBe(false);
    expect(invitationProvesEmail({ ...inv, expiresAt: "not a date" }, "teammate@example.com", NOW)).toBe(false);
    expect(invitationProvesEmail(null, "teammate@example.com", NOW)).toBe(false);
    expect(invitationProvesEmail(inv, "", NOW)).toBe(false);
  });
});

describe("mayAcceptInvitation", () => {
  it("lets a signed-in user with that verified email accept", () => {
    expect(mayAcceptInvitation(inv, { email: "teammate@example.com", emailVerified: true }, NOW)).toBe(true);
  });

  it("refuses an unverified email, another user, or an invitation with no team", () => {
    expect(mayAcceptInvitation(inv, { email: "teammate@example.com", emailVerified: false }, NOW)).toBe(false);
    expect(mayAcceptInvitation(inv, { email: "owner@example.com", emailVerified: true }, NOW)).toBe(false);
    expect(mayAcceptInvitation({ ...inv, organizationId: null }, { email: "teammate@example.com", emailVerified: true }, NOW)).toBe(false);
    expect(mayAcceptInvitation(inv, null, NOW)).toBe(false);
  });
});
