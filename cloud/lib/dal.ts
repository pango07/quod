import "server-only";
import { notFound } from "next/navigation";
import { cache } from "react";
import { api } from "@/lib/api";
import type { Team } from "@/lib/api/types";
import { getViewer } from "@/lib/auth/session";
import { isAdmin, type Role, type Viewer } from "@/lib/auth/types";

// The data access layer: every page and action resolves the team through
// here, so membership is checked next to the data, not only in the UI.

export interface TeamWithRole extends Team {
  role: Role;
}

/** The viewer's teams: one per WorkOS Organization they belong to. */
export const getTeams = cache(async (): Promise<TeamWithRole[]> => {
  const viewer = await getViewer();
  const teams = await api().resolveTeams(viewer.memberships.map((m) => ({ orgId: m.orgId, name: m.orgName })));
  return teams.map((t) => ({ ...t, role: viewer.memberships.find((m) => m.orgId === t.orgId)?.role ?? "member" }));
});

export interface TeamContext {
  viewer: Viewer;
  team: TeamWithRole;
  teams: TeamWithRole[];
  admin: boolean;
}

/** The team in the URL, if the viewer belongs to it. A team they do not belong to is a 404, not a 403. */
export const requireTeam = cache(async (slug: string): Promise<TeamContext> => {
  const [viewer, teams] = await Promise.all([getViewer(), getTeams()]);
  const team = teams.find((t) => t.slug === slug);
  if (!team) notFound();
  return { viewer, team, teams, admin: isAdmin(team.role) };
});
