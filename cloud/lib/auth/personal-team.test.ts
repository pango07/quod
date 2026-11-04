import { beforeEach, describe, expect, it, vi } from "vitest";

// personal-team.ts with WorkOS mocked: the name, the slug it gets, and the
// idempotency rules (memberships first, a unique externalId, races).
const um = { listOrganizationMemberships: vi.fn(), listInvitations: vi.fn(), createOrganizationMembership: vi.fn() };
const orgs = { getOrganizationByExternalId: vi.fn(), createOrganization: vi.fn() };
const invalidateMemberships = vi.fn();

vi.mock("server-only", () => ({}));
vi.mock("@workos-inc/authkit-nextjs", () => ({ getWorkOS: () => ({ userManagement: um, organizations: orgs }) }));
vi.mock("./session", () => ({ invalidateMemberships: (...a: unknown[]) => invalidateMemberships(...a) }));

const { ensurePersonalTeam, onboardingPath, personalTeamName } = await import("./personal-team");

const sam = { id: "user_9", email: "sam@example.com", firstName: "Sam" };
const notFound = () => Object.assign(new Error("not found"), { status: 404 });
const conflict = () => Object.assign(new Error("external_id taken"), { status: 409 });
const openInvitation = { id: "invitation_1", email: sam.email, state: "pending", expiresAt: new Date(Date.now() + 86_400_000).toISOString(), organizationId: "org_acme" };

beforeEach(() => {
  for (const f of [...Object.values(um), ...Object.values(orgs), invalidateMemberships]) f.mockReset();
  um.listOrganizationMemberships.mockResolvedValue({ data: [] });
  um.listInvitations.mockResolvedValue({ data: [] });
  um.createOrganizationMembership.mockResolvedValue({ id: "om_1" });
  orgs.getOrganizationByExternalId.mockRejectedValue(notFound());
  orgs.createOrganization.mockResolvedValue({ id: "org_sam", name: "Sam's team" });
});

describe("the team's name and slug", () => {
  it("is named after the person, else their email's local part", () => {
    expect(personalTeamName({ email: "sam@example.com", firstName: "Sam" })).toBe("Sam's team");
    expect(personalTeamName({ email: "octocat+ci@users.noreply.github.com", firstName: "" })).toBe("octocat's team");
    expect(personalTeamName({ email: "x@example.com", firstName: "  Ana\u0000 \n María  " })).toBe("Ana María's team");
    expect(personalTeamName({ email: "@example.com", firstName: null })).toBe("My team");
    expect(personalTeamName({ email: "a@b.c", firstName: "N".repeat(300) }).length).toBeLessThanOrEqual(47);
  });

  it("gets a valid team slug, clear of the app's own paths", () => {
    expect(onboardingPath("org_1", "Sam's team")).toBe("/sams-team/connect");
    expect(onboardingPath("org_1", "Setup")).toBe("/setup-2/connect");
    expect(onboardingPath("org_1", "李's team")).toBe("/s-team/connect");
    expect(onboardingPath("org_1", "'''")).toBe("/team/connect");
  });
});

describe("ensurePersonalTeam", () => {
  it("creates the team with a per-user externalId and idempotency key, and makes the person its admin", async () => {
    expect(await ensurePersonalTeam(sam, { honorInvitations: true })).toEqual({ kind: "created", orgId: "org_sam", name: "Sam's team" });
    expect(orgs.createOrganization).toHaveBeenCalledWith({ name: "Sam's team", externalId: "quod-personal:user_9", metadata: { quod_personal_team_of: "user_9" } }, { idempotencyKey: "quod-personal:user_9" });
    expect(um.createOrganizationMembership).toHaveBeenCalledWith({ organizationId: "org_sam", userId: "user_9", roleSlug: "admin" });
    expect(invalidateMemberships).toHaveBeenCalledWith("user_9");
  });

  it("creates nothing for someone already in a team", async () => {
    um.listOrganizationMemberships.mockResolvedValue({ data: [{ organizationId: "org_acme" }] });
    expect(await ensurePersonalTeam(sam, { honorInvitations: true })).toEqual({ kind: "member", orgId: "org_acme" });
    expect(orgs.getOrganizationByExternalId).not.toHaveBeenCalled();
    expect(orgs.createOrganization).not.toHaveBeenCalled();
  });

  it("an open invitation wins over a new team, unless the person asked for one", async () => {
    um.listInvitations.mockResolvedValue({ data: [openInvitation] });
    expect(await ensurePersonalTeam(sam, { honorInvitations: true })).toEqual({ kind: "invited" });
    expect(orgs.createOrganization).not.toHaveBeenCalled();
    expect((await ensurePersonalTeam(sam, { honorInvitations: false })).kind).toBe("created");
  });

  it("an expired or someone else's invitation doesn't count", async () => {
    um.listInvitations.mockResolvedValue({ data: [{ ...openInvitation, expiresAt: new Date(Date.now() - 1000).toISOString() }, { ...openInvitation, state: "accepted" }] });
    expect((await ensurePersonalTeam(sam, { honorInvitations: true })).kind).toBe("created");
  });

  it("creates nothing when memberships or invitations can't be read", async () => {
    um.listInvitations.mockRejectedValue(Object.assign(new Error("down"), { status: 503 }));
    await expect(ensurePersonalTeam(sam, { honorInvitations: true })).rejects.toThrow("down");
    um.listOrganizationMemberships.mockRejectedValue(Object.assign(new Error("down"), { status: 503 }));
    await expect(ensurePersonalTeam(sam, { honorInvitations: false })).rejects.toThrow("down");
    expect(orgs.createOrganization).not.toHaveBeenCalled();
  });

  it("a retry after a half-finished attempt reuses the organization it made", async () => {
    orgs.getOrganizationByExternalId.mockResolvedValue({ id: "org_sam", name: "Sam's team" });
    expect(await ensurePersonalTeam(sam, { honorInvitations: true })).toEqual({ kind: "created", orgId: "org_sam", name: "Sam's team" });
    expect(orgs.createOrganization).not.toHaveBeenCalled();
    expect(um.createOrganizationMembership).toHaveBeenCalledTimes(1);
  });

  it("two at once: the loser's create is refused by the unique externalId and it adopts the winner's team", async () => {
    orgs.getOrganizationByExternalId.mockRejectedValueOnce(notFound()).mockResolvedValueOnce({ id: "org_sam", name: "Sam's team" });
    orgs.createOrganization.mockRejectedValue(conflict());
    um.createOrganizationMembership.mockRejectedValue(Object.assign(new Error("already a member"), { status: 400 }));
    um.listOrganizationMemberships.mockResolvedValueOnce({ data: [] }).mockResolvedValueOnce({ data: [{ organizationId: "org_sam" }] });
    expect(await ensurePersonalTeam(sam, { honorInvitations: true })).toEqual({ kind: "created", orgId: "org_sam", name: "Sam's team" });
    expect(um.listOrganizationMemberships).toHaveBeenLastCalledWith({ userId: "user_9", organizationId: "org_sam", statuses: ["active"], limit: 1 });
  });

  it("a create that fails for another reason is reported, not hidden", async () => {
    orgs.createOrganization.mockRejectedValue(Object.assign(new Error("boom"), { status: 500 }));
    await expect(ensurePersonalTeam(sam, { honorInvitations: true })).rejects.toThrow("boom");
    expect(um.createOrganizationMembership).not.toHaveBeenCalled();
  });

  it("a membership that can't be made, and isn't there, is reported", async () => {
    um.createOrganizationMembership.mockRejectedValue(Object.assign(new Error("nope"), { status: 403 }));
    await expect(ensurePersonalTeam(sam, { honorInvitations: true })).rejects.toThrow("nope");
    expect(invalidateMemberships).not.toHaveBeenCalled();
  });
});
