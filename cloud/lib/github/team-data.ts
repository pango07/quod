import "server-only";
import { cache } from "react";
import { readGitHubState, type GhRepo } from "@/lib/api/composite";
import { isSampleOrg } from "@/lib/api/fixture-data";
import { resolveApiMode } from "@/lib/api/mode";
import type { GitHubLink } from "@/lib/api/types";
import { dbConfigured, getDb } from "@/lib/db";
import type { Db } from "@/lib/db/types";
import { githubAppConfig } from "./env";
import type { TeamId } from "./team";

// The team's GitHub data source for the dashboard pages (Runs, Runners,
// Usage, a GitHub run): the same link the composite CloudApi sees, by org id.

export type TeamGitHub =
  /** The demo session's sample team: its CI is simulated, never GitHub's. */
  | { sample: true; link: undefined; source: null }
  /** `source` only when linked. */
  | { sample: false; link: GitHubLink; source: { teamId: TeamId; db: Db; repos: GhRepo[] } | null };

/** Per request, by org id (from the membership-checked team context). */
export const teamGitHub = cache(async (orgId: string): Promise<TeamGitHub> => {
  if (resolveApiMode(process.env).kind === "fixture" && isSampleOrg(orgId)) return { sample: true, link: undefined, source: null };
  const g = await readGitHubState(orgId, { configured: () => dbConfigured(), appReady: () => githubAppConfig().ok, db: () => getDb() });
  return { sample: false, link: g.link, source: g.teamId !== null && g.link.state === "linked" ? { teamId: g.teamId, db: g.db, repos: g.repos } : null };
});
