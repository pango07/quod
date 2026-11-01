import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Membership } from "@/lib/auth/types";
import type { GhRepoWithPermissions, GhUserInstallation } from "./api";
import { decideLink, verifyInstallationClaim, type GitHubForLinking, type LinkFacts } from "./link";

// Installation -> team linking. Without these checks anyone could claim
// another organization's installation by typing its id into the setup URL.

const ORG_INST: GhUserInstallation = { id: 77, account: { id: 500, login: "acme", type: "Organization" }, repository_selection: "selected" };
const USER_INST: GhUserInstallation = { id: 88, account: { id: 1, login: "jamie", type: "User" }, repository_selection: "all" };
const admin: Membership = { orgId: "org_acme", orgName: "Acme", role: "admin" };
const member: Membership = { orgId: "org_acme", orgName: "Acme", role: "member" };
const repo = (id: number, isAdmin: boolean): GhRepoWithPermissions => ({ id, name: `r${id}`, full_name: `acme/r${id}`, private: true, permissions: { admin: isAdmin, push: true, pull: true } });

const facts = (o: Partial<LinkFacts> = {}): LinkFacts => ({
  team: { orgId: "org_acme" },
  memberships: [admin],
  installationId: 77,
  githubUser: { id: 1, login: "jamie" },
  userInstallations: [ORG_INST, USER_INST],
  linkedTeamId: null,
  installationRepos: { items: [{ id: 1 }, { id: 2 }], complete: true },
  userRepos: { items: [repo(1, true), repo(2, true)], complete: true },
  ...o,
});

describe("decideLink", () => {
  it("links an org installation for a team admin who administers every repo in it", () => {
    expect(decideLink(facts())).toMatchObject({ ok: true, installation: { id: 77 } });
  });

  it("links the user's own account installation without the repo check", () => {
    expect(decideLink(facts({ installationId: 88, userRepos: { items: [], complete: true } }))).toMatchObject({ ok: true });
  });

  it("refuses a viewer with no current team", () => {
    expect(decideLink(facts({ team: null }))).toEqual({ ok: false, reason: "no-team" });
  });

  it("refuses a non-member of the team", () => {
    expect(decideLink(facts({ memberships: [{ orgId: "org_other", orgName: "Other", role: "admin" }] }))).toEqual({ ok: false, reason: "not-member" });
    expect(decideLink(facts({ memberships: [] }))).toEqual({ ok: false, reason: "not-member" });
  });

  it("refuses a team member who is not an admin", () => {
    expect(decideLink(facts({ memberships: [member] }))).toEqual({ ok: false, reason: "not-admin" });
    expect(decideLink(facts({ memberships: [{ ...admin, role: "owner" }] }))).toEqual({ ok: false, reason: "not-admin" });
  });

  it("refuses an installation that is not in the user's /user/installations", () => {
    expect(decideLink(facts({ installationId: 12345 }))).toEqual({ ok: false, reason: "not-in-user-installations" });
    expect(decideLink(facts({ userInstallations: [] }))).toEqual({ ok: false, reason: "not-in-user-installations" });
  });

  it("refuses an installation linked to another team (and allows re-linking to the same team)", () => {
    expect(decideLink(facts({ linkedTeamId: "org_other" }))).toEqual({ ok: false, reason: "linked-elsewhere" });
    expect(decideLink(facts({ linkedTeamId: "org_acme" }))).toMatchObject({ ok: true });
  });

  it("refuses an org member who can see the installation but does not administer all its repos", () => {
    expect(decideLink(facts({ userRepos: { items: [repo(1, true), repo(2, false)], complete: true } }))).toEqual({ ok: false, reason: "not-repo-admin" });
    // A repo the user cannot see at all.
    expect(decideLink(facts({ userRepos: { items: [repo(1, true)], complete: true } }))).toEqual({ ok: false, reason: "not-repo-admin" });
  });

  it("refuses someone else's user-account installation unless they administer its repos", () => {
    const someoneElse = { githubUser: { id: 2, login: "mallory" }, installationId: 88 };
    expect(decideLink(facts({ ...someoneElse, installationRepos: { items: [{ id: 5 }], complete: true }, userRepos: { items: [repo(5, false)], complete: true } }))).toEqual({ ok: false, reason: "not-repo-admin" });
  });

  it("refuses when the repo lists were cut off", () => {
    expect(decideLink(facts({ installationRepos: { items: [{ id: 1 }], complete: false } }))).toEqual({ ok: false, reason: "too-many-repos" });
  });
});

describe("verifyInstallationClaim", () => {
  let gh: { [K in keyof GitHubForLinking]: ReturnType<typeof vi.fn> };
  beforeEach(() => {
    gh = {
      exchangeCode: vi.fn(async () => "ghu_token"),
      getUser: vi.fn(async () => ({ id: 1, login: "jamie" })),
      listUserInstallations: vi.fn(async () => [ORG_INST]),
      listUserInstallationRepos: vi.fn(async () => ({ items: [repo(1, true)], complete: true })),
      listInstallationRepos: vi.fn(async () => ({ items: [repo(1, true)], complete: true })),
      revoke: vi.fn(async () => undefined),
    };
  });
  const run = (o: { memberships?: Membership[]; installationId?: number; linked?: string | null } = {}) =>
    verifyInstallationClaim({
      code: "code123",
      installationId: o.installationId ?? 77,
      viewer: { id: "user_1", memberships: o.memberships ?? [admin] },
      team: { orgId: "org_acme" },
      linkedTeamId: async () => o.linked ?? null,
      gh: gh as unknown as GitHubForLinking,
    });

  it("returns a claim for the verified installation and revokes the user token", async () => {
    const r = await run();
    expect(r).toMatchObject({ ok: true, claim: { installationId: 77, teamOrgId: "org_acme", userId: "user_1", githubUserId: 1, repoCount: 1, account: { login: "acme" } } });
    expect(gh.revoke).toHaveBeenCalledWith("ghu_token");
  });

  it("makes no GitHub call for a non-admin or non-member", async () => {
    expect(await run({ memberships: [member] })).toEqual({ ok: false, reason: "not-admin" });
    expect(await run({ memberships: [] })).toEqual({ ok: false, reason: "not-member" });
    expect(gh.exchangeCode).not.toHaveBeenCalled();
  });

  it("refuses a spoofed installation_id that the user's token cannot see, and still revokes", async () => {
    expect(await run({ installationId: 4242 })).toEqual({ ok: false, reason: "not-in-user-installations" });
    expect(gh.listInstallationRepos).not.toHaveBeenCalled();
    expect(gh.revoke).toHaveBeenCalledWith("ghu_token");
  });

  it("refuses an installation another team has linked", async () => {
    expect(await run({ linked: "org_other" })).toEqual({ ok: false, reason: "linked-elsewhere" });
  });

  it("revokes the token when GitHub fails part way", async () => {
    gh.listUserInstallations.mockRejectedValue(new Error("boom"));
    await expect(run()).rejects.toThrow("boom");
    expect(gh.revoke).toHaveBeenCalledWith("ghu_token");
  });
});
