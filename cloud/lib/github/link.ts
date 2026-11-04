import { createHash } from "node:crypto";
import type { Membership } from "@/lib/auth/types";
import type { GhRepoWithPermissions, GhUser, GhUserInstallation } from "./api";

// Linking a GitHub App installation to a Quod team. GitHub's setup URL
// carries an installation_id anyone can type, so it is never trusted on its
// own (GitHub's docs say the same). An installation is linked only when:
//
// 1. the signed-in viewer is a member of the team, and an admin of it;
// 2. the installation is in GET /user/installations for the viewer's GitHub
//    user token (from the OAuth code GitHub sends after the install);
// 3. it is not already linked to another team;
// 4. the GitHub user owns everything the installation can see: it is their
//    own account, or they are an admin of every repository in it. GitHub
//    lists an organization's installation for any member with read access
//    to one of its repos, so (2) alone would let that member bring the whole
//    organization's CI into their own Quod team.

export type LinkRefusal =
  | "no-team"
  | "not-member"
  | "not-admin"
  | "not-in-user-installations"
  | "linked-elsewhere"
  | "not-repo-admin"
  | "too-many-repos"
  | "changed";

export const REFUSAL_MESSAGE: Record<LinkRefusal, string> = {
  "no-team": "You are not in a Quod team yet. Create your free team from the dashboard (or ask a teammate to invite you), then open this link again.",
  "not-member": "You are not a member of that Quod team.",
  "not-admin": "Only a team admin can link a GitHub installation. Ask an admin of this team, or switch to a team where you are an admin.",
  "not-in-user-installations": "GitHub does not list this installation for your GitHub account, so it cannot be linked. Install the App from your own account or organization, then try again.",
  "linked-elsewhere": "This installation is already linked to another Quod team. An admin there can uninstall the App on GitHub to release it.",
  "not-repo-admin": "To link an organization's installation you need admin access to every repository it covers. Ask an organization owner to link it.",
  "too-many-repos": "This installation covers more repositories than Quod can check at once. Contact support@quodci.com.",
  changed: "The installation's repositories changed on GitHub while you were confirming. Connect again so Quod can check the new ones.",
};

export interface LinkFacts {
  /** The team to link to (the viewer's current team), or null if they have none. */
  team: { orgId: string } | null;
  memberships: Membership[];
  installationId: number;
  githubUser: GhUser;
  userInstallations: GhUserInstallation[];
  /** Which team the installation is linked to now (from the database). */
  linkedTeamId: string | null;
  /** Every repo in the installation (installation token), and whether the list is complete. */
  installationRepos: { items: { id: number }[]; complete: boolean };
  /** The installation's repos as the user sees them, with their permissions. */
  userRepos: { items: GhRepoWithPermissions[]; complete: boolean };
}

export type LinkDecision = { ok: true; installation: GhUserInstallation } | { ok: false; reason: LinkRefusal };

/** The authorization rule, pure. */
export function decideLink(f: LinkFacts): LinkDecision {
  if (!f.team) return { ok: false, reason: "no-team" };
  const m = f.memberships.find((x) => x.orgId === f.team!.orgId);
  if (!m) return { ok: false, reason: "not-member" };
  if (m.role !== "admin") return { ok: false, reason: "not-admin" };

  const inst = f.userInstallations.find((i) => i.id === f.installationId);
  if (!inst) return { ok: false, reason: "not-in-user-installations" };
  if (f.linkedTeamId && f.linkedTeamId !== f.team.orgId) return { ok: false, reason: "linked-elsewhere" };

  const ownAccount = inst.account?.type === "User" && inst.account.id === f.githubUser.id;
  if (!ownAccount) {
    if (!f.installationRepos.complete || !f.userRepos.complete) return { ok: false, reason: "too-many-repos" };
    const adminOf = new Set(f.userRepos.items.filter((r) => r.permissions?.admin === true).map((r) => r.id));
    if (!f.installationRepos.items.every((r) => adminOf.has(r.id))) return { ok: false, reason: "not-repo-admin" };
  }
  return { ok: true, installation: inst };
}

/** A short fingerprint of a set of repo ids (order does not matter). */
export function repoDigest(ids: number[]): string {
  return createHash("sha256").update([...new Set(ids)].sort((a, b) => a - b).join(",")).digest("base64url").slice(0, 22);
}

/**
 * The confirm step re-reads the installation's repos with the App's token.
 * The admin check covered exactly the claimed set; if it changed (a repo was
 * added in GitHub meanwhile, which the user may not administer), refuse and
 * have them verify again rather than link repos nobody checked.
 */
export function repoSetUnchanged(claim: Pick<LinkClaim, "repoCount" | "repoDigest">, ids: number[]): boolean {
  return new Set(ids).size === claim.repoCount && repoDigest(ids) === claim.repoDigest;
}

/** The verified result of a GitHub round trip, sealed in a short-lived cookie until the admin confirms. */
export interface LinkClaim {
  purpose: "quod-gh-claim";
  installationId: number;
  account: { id: number; login: string; type: string };
  repositorySelection: string;
  /** How many repos the installation covered when checked. The list itself is re-read with the App's token on confirm (keeps the cookie small). */
  repoCount: number;
  /** repoDigest of the checked repo ids: confirm refuses if the set changed since (m5). */
  repoDigest: string;
  teamOrgId: string;
  userId: string;
  githubUserId: number;
  githubLogin: string;
}

export interface GitHubForLinking {
  exchangeCode(code: string): Promise<string>;
  getUser(token: string): Promise<GhUser>;
  listUserInstallations(token: string): Promise<GhUserInstallation[]>;
  listUserInstallationRepos(token: string, installationId: number): Promise<{ items: GhRepoWithPermissions[]; complete: boolean }>;
  /** With the App's own credentials (installation token). */
  listInstallationRepos(installationId: number): Promise<{ items: GhRepoWithPermissions[]; complete: boolean }>;
  revoke(token: string): Promise<void>;
}

/**
 * Runs the GitHub round trip for a returned OAuth code and decides. The user
 * token is revoked before returning, whatever the outcome.
 */
export async function verifyInstallationClaim(input: {
  code: string;
  installationId: number;
  viewer: { id: string; memberships: Membership[] };
  team: { orgId: string } | null;
  linkedTeamId: (installationId: number) => Promise<string | null>;
  gh: GitHubForLinking;
}): Promise<{ ok: true; claim: LinkClaim } | { ok: false; reason: LinkRefusal }> {
  const { gh } = input;
  // Membership and role first: no GitHub calls for someone who could never link.
  const early = decideLink({
    team: input.team,
    memberships: input.viewer.memberships,
    installationId: input.installationId,
    githubUser: { id: -1, login: "" },
    userInstallations: [{ id: input.installationId, account: null }],
    linkedTeamId: null,
    installationRepos: { items: [], complete: true },
    userRepos: { items: [], complete: true },
  });
  if (!early.ok) return early;

  const token = await gh.exchangeCode(input.code);
  try {
    const [user, installations] = await Promise.all([gh.getUser(token), gh.listUserInstallations(token)]);
    if (!installations.some((i) => i.id === input.installationId)) return { ok: false, reason: "not-in-user-installations" };
    const [linkedTeamId, installationRepos, userRepos] = await Promise.all([
      input.linkedTeamId(input.installationId),
      gh.listInstallationRepos(input.installationId),
      gh.listUserInstallationRepos(token, input.installationId),
    ]);
    const d = decideLink({
      team: input.team,
      memberships: input.viewer.memberships,
      installationId: input.installationId,
      githubUser: user,
      userInstallations: installations,
      linkedTeamId,
      installationRepos,
      userRepos,
    });
    if (!d.ok) return d;
    const acct = d.installation.account;
    return {
      ok: true,
      claim: {
        purpose: "quod-gh-claim",
        installationId: d.installation.id,
        account: { id: acct?.id ?? 0, login: acct?.login ?? "", type: acct?.type ?? "" },
        repositorySelection: d.installation.repository_selection ?? "selected",
        repoCount: installationRepos.items.length,
        repoDigest: repoDigest(installationRepos.items.map((r) => r.id)),
        teamOrgId: input.team!.orgId,
        userId: input.viewer.id,
        githubUserId: user.id,
        githubLogin: user.login,
      },
    };
  } finally {
    await gh.revoke(token);
  }
}
