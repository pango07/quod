import "server-only";
import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { cache } from "react";
import { authMode } from "./config";
import { DEMO_VIEWER } from "./demo";
import type { Membership, Viewer } from "./types";

/**
 * The signed-in viewer for this request, read once per request. Throws when
 * sign-in is misconfigured (production with no WorkOS config never gets here
 * as a guest). Redirects to AuthKit when there is no session.
 */
export const getViewer = cache(async (): Promise<Viewer> => {
  if (authMode() === "demo") return DEMO_VIEWER;

  const auth = await withAuth({ ensureSignedIn: true });
  const { user } = auth;
  const memberships = await listMemberships(user.id);
  const name = [user.firstName, user.lastName].filter(Boolean).join(" ") || user.email;
  return {
    id: user.id,
    name,
    email: user.email,
    avatarUrl: user.profilePictureUrl ?? null,
    demo: false,
    currentOrgId: auth.organizationId ?? null,
    memberships,
  };
});

/** Active WorkOS Organization memberships, with each organization's role for this user. */
async function listMemberships(userId: string): Promise<Membership[]> {
  const page = await getWorkOS().userManagement.listOrganizationMemberships({ userId, statuses: ["active"], limit: 100 });
  const all = await page.autoPagination();
  return all.map((m) => ({ orgId: m.organizationId, orgName: m.organizationName, role: m.role?.slug ?? "member" }));
}

/** The viewer's access token, for quod-server. Undefined in the demo session. */
export async function accessToken(): Promise<string | undefined> {
  if (authMode() === "demo") return undefined;
  const { accessToken } = await withAuth();
  return accessToken;
}
