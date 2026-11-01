import "server-only";
import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { connection } from "next/server";
import { cache } from "react";
import { authMode } from "./config";
import { DEMO_VIEWER } from "./demo";
import { TtlCache } from "./limits";
import type { Membership, Viewer } from "./types";

/**
 * The signed-in viewer for this request, read once per request. Throws when
 * sign-in is misconfigured (production with no WorkOS config never gets here
 * as a guest). Sends a signed-out viewer to our own /sign-in (the proxy
 * normally does that first).
 */
export const getViewer = cache(() => loadViewer());

/** getViewer without the per-request memo: for long-lived streams that re-check membership. */
export async function loadViewer(): Promise<Viewer> {
  // Per request, never prerendered: a runtime prefetch has the cookies but must not
  // read the session or the membership cache's clock (Date.now()) into a prerender.
  await connection();
  if (authMode() === "demo") return DEMO_VIEWER;

  const auth = await withAuth();
  const { user } = auth;
  if (!user) redirect("/sign-in");
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
}

/**
 * Memberships change rarely but are read on every page view and live refresh,
 * so they are kept 60 s per user, in memory, per server instance. Actions that
 * change them (team switch, invitations, role changes) call invalidateMemberships.
 */
const membershipCache = new TtlCache<Membership[]>(60_000, 10_000);

export function invalidateMemberships(userId: string): void {
  membershipCache.delete(userId);
}

/** Active WorkOS Organization memberships, with each organization's role for this user. */
async function listMemberships(userId: string): Promise<Membership[]> {
  const hit = membershipCache.get(userId);
  if (hit) return hit;
  const page = await getWorkOS().userManagement.listOrganizationMemberships({ userId, statuses: ["active"], limit: 100 });
  const all = await page.autoPagination();
  const memberships = all.map((m) => ({ orgId: m.organizationId, orgName: m.organizationName, role: m.role?.slug ?? "member" }));
  membershipCache.set(userId, memberships);
  return memberships;
}

/** The viewer's access token, for quod-server. Undefined in the demo session. */
export async function accessToken(): Promise<string | undefined> {
  if (authMode() === "demo") return undefined;
  const { accessToken } = await withAuth();
  return accessToken;
}
