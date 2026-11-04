import "server-only";
import { getWorkOS } from "@workos-inc/authkit-nextjs";
import { authMode } from "./config";
import { DEMO_MEMBERS } from "./demo";
import type { Member, Role } from "./types";

// Members, invitations and the Admin Portal live in WorkOS. Quod stores none of it.

export async function listMembers(orgId: string): Promise<Member[]> {
  if (authMode() === "demo") return DEMO_MEMBERS[orgId] ?? [];
  const workos = getWorkOS();
  const [memberships, invitations] = await Promise.all([
    workos.userManagement.listOrganizationMemberships({ organizationId: orgId, limit: 100 }).then((p) => p.autoPagination()),
    workos.userManagement.listInvitations({ organizationId: orgId, limit: 100 }).then((p) => p.autoPagination()),
  ]);
  const users = await Promise.all(memberships.map((m) => (m.user ? Promise.resolve(m.user) : workos.userManagement.getUser(m.userId))));
  const members: Member[] = memberships.map((m, i) => ({
    id: m.id,
    name: [users[i].firstName, users[i].lastName].filter(Boolean).join(" ") || null,
    email: users[i].email,
    role: m.role?.slug ?? "member",
    status: m.status === "pending" ? "invited" : m.status === "active" ? "active" : "inactive",
    directoryManaged: m.directoryManaged,
  }));
  // An invitation can also show up as a pending membership; list each email once.
  const listed = new Set(members.map((m) => m.email.toLowerCase()));
  const pending: Member[] = invitations
    .filter((inv) => inv.state === "pending" && !listed.has(inv.email.toLowerCase()))
    .map((inv) => ({ id: inv.id, name: null, email: inv.email, role: inv.roleSlug ?? "member", status: "invited", directoryManaged: false }));
  return [...members.filter((m) => m.status !== "inactive"), ...pending];
}

export type InviteResult = { ok: true; message: string } | { ok: false; message: string };

export async function inviteMember(orgId: string, inviterUserId: string, email: string, role: Role): Promise<InviteResult> {
  if (authMode() === "demo") return { ok: true, message: `Demo session: no email was sent to ${email}.` };
  await getWorkOS().userManagement.sendInvitation({ email, organizationId: orgId, roleSlug: role, inviterUserId });
  return { ok: true, message: `Invitation sent to ${email}.` };
}

/**
 * Pending invitations addressed to this email, with their team names. For "/"
 * when the viewer has no team yet: someone who signed in without their
 * invitation link (an expired step, another browser) can still join. A failed
 * read is an empty list; joinTeam checks each invitation again before accepting.
 */
export async function pendingInvitationsFor(email: string): Promise<{ id: string; teamName: string }[]> {
  if (authMode() === "demo") return [];
  try {
    const workos = getWorkOS();
    const page = await workos.userManagement.listInvitations({ email, limit: 20 });
    const now = Date.now();
    const open = page.data.filter((inv) => inv.state === "pending" && inv.organizationId && new Date(inv.expiresAt).getTime() > now && inv.email.toLowerCase() === email.toLowerCase());
    return await Promise.all(
      open.map(async (inv) => ({ id: inv.id, teamName: (await workos.organizations.getOrganization(inv.organizationId as string)).name })),
    );
  } catch (e) {
    console.warn("[quod-cloud] pending invitations not read", { status: (e as { status?: number })?.status });
    return [];
  }
}

export type PortalIntent = "sso" | "dsync" | "domain_verification" | "audit_logs";

/** A short-lived WorkOS Admin Portal link for the organization's IT admin. */
export async function adminPortalLink(orgId: string, intent: PortalIntent, returnUrl: string): Promise<string | null> {
  if (authMode() === "demo") return null;
  const { link } = await getWorkOS().adminPortal.generateLink({ organization: orgId, intent, returnUrl });
  return link;
}
