import "server-only";
import { getWorkOS } from "@workos-inc/authkit-nextjs";
import { resolveTeamsLocally } from "@/lib/api/teams";
import { invitationProvesEmail } from "./invite";
import { invalidateMemberships } from "./session";

// Sign-up gives everyone a free team of their own (decision 2026-11-04), so
// nobody lands on "You are not in a team yet" with no way forward. The team is
// a WorkOS Organization like any other; the user is its admin.
//
// Idempotent by construction: the user's memberships are read first, and the
// organization carries a per-user externalId (unique in a WorkOS environment),
// so a double submit, a retry or two tabs find the same team instead of
// making a second one. Invited people never get one automatically: the
// invitation is their way in.

export interface TeamOwner {
  id: string;
  email: string;
  firstName?: string | null;
}

/** The WorkOS externalId of a user's own team: one per user, enforced by WorkOS. */
export const personalTeamExternalId = (userId: string) => `quod-personal:${userId}`;

const clean = (s: string) =>
  s
    .normalize("NFC")
    .replace(/[\p{Cc}\p{Cf}]/gu, "")
    .replace(/\s+/g, " ")
    .trim();

/** "Sam's team", from the first name, else the email's local part (a GitHub sign-up's login is usually there). */
export function personalTeamName(user: Pick<TeamOwner, "email" | "firstName">): string {
  const first = clean(user.firstName ?? "").slice(0, 40).trim();
  const local = clean(user.email.split("@")[0]?.split("+")[0] ?? "").slice(0, 40).trim();
  const who = first || local;
  return who ? `${who}'s team` : "My team";
}

/** Where a new team's owner goes next: connecting GitHub. The slug is the one the dashboard makes for a viewer's only team. */
export function onboardingPath(orgId: string, name: string): string {
  const [team] = resolveTeamsLocally([{ orgId, name }]);
  return `/${team.slug}/connect`;
}

export type PersonalTeam = { kind: "member"; orgId: string } | { kind: "invited" } | { kind: "created"; orgId: string; name: string };

const status = (e: unknown) => (e as { status?: number })?.status;

async function activeOrgOf(userId: string): Promise<string | undefined> {
  const page = await getWorkOS().userManagement.listOrganizationMemberships({ userId, statuses: ["active"], limit: 1 });
  return page.data[0]?.organizationId;
}

/** Pending, unexpired invitations to this email. Unlike pendingInvitationsFor, a failed read throws: never create a team on a guess. */
async function hasOpenInvitation(email: string): Promise<boolean> {
  // Every page: an open invitation on a later page must still stop the team being made.
  const all = await (await getWorkOS().userManagement.listInvitations({ email, limit: 100 })).autoPagination();
  const now = Date.now();
  return all.some((inv) => inv.organizationId && invitationProvesEmail(inv, email, now));
}

/** The user's own organization: found by externalId, else created with it. A concurrent create loses to the unique externalId and finds the winner. */
async function personalOrg(user: TeamOwner): Promise<{ id: string; name: string }> {
  const orgs = getWorkOS().organizations;
  const externalId = personalTeamExternalId(user.id);
  const find = async () => {
    try {
      return await orgs.getOrganizationByExternalId(externalId);
    } catch (e) {
      if (status(e) === 404) return null;
      throw e;
    }
  };
  const existing = await find();
  if (existing) return existing;
  try {
    return await orgs.createOrganization({ name: personalTeamName(user), externalId, metadata: { quod_personal_team_of: user.id } }, { idempotencyKey: externalId });
  } catch (e) {
    const winner = await find();
    if (winner) return winner;
    throw e;
  }
}

/** Adds the user as admin. "Already a member" (a retry, another tab) is success when that membership is active. */
async function addAdmin(organizationId: string, userId: string): Promise<void> {
  const um = getWorkOS().userManagement;
  try {
    await um.createOrganizationMembership({ organizationId, userId, roleSlug: "admin" });
  } catch (e) {
    const page = await um.listOrganizationMemberships({ userId, organizationId, statuses: ["active"], limit: 1 });
    if (!page.data.length) throw e;
  }
}

/**
 * The user's team, made if they have none. Throws when WorkOS can't be read or
 * written; callers decide whether that is fatal.
 * - Already in a team: that team, nothing created.
 * - `honorInvitations` and an open invitation: nothing created (they join instead).
 * - Otherwise their own team (found again on a retry), with them as admin.
 */
export async function ensurePersonalTeam(user: TeamOwner, opts: { honorInvitations: boolean }): Promise<PersonalTeam> {
  const current = await activeOrgOf(user.id);
  if (current) return { kind: "member", orgId: current };
  if (opts.honorInvitations && (await hasOpenInvitation(user.email))) return { kind: "invited" };
  const org = await personalOrg(user);
  await addAdmin(org.id, user.id);
  invalidateMemberships(user.id);
  return { kind: "created", orgId: org.id, name: org.name };
}
