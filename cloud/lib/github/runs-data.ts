import "server-only";
import { unstable_rethrow } from "next/navigation";
import type { GitHubRunsData } from "@/components/runs-view";
import type { GhRepo } from "@/lib/api/composite";
import type { GitHubLink } from "@/lib/api/types";
import { listTeamRuns, parseRunQuery, teamHasRuns } from "./pages";
import type { TeamGitHub } from "./team-data";

/**
 * The GitHub side of a Runs page: one page of the team's (or one repo's)
 * workflow runs, filtered by the URL. `link` turns "unavailable" when the
 * database fails, so the page says so instead of crashing.
 */
export async function githubRunsFor(
  gh: TeamGitHub,
  sp: Record<string, string | string[] | undefined>,
  only?: GhRepo,
): Promise<{ link: GitHubLink | undefined; data: GitHubRunsData | null }> {
  if (!gh.source) return { link: gh.link, data: null };
  const { teamId, db, repos } = gh.source;
  const query = parseRunQuery(only ? { ...sp, repo: only.slug } : sp, repos);
  try {
    const page = await listTeamRuns(teamId, db, { repoId: query.repoId, branch: query.branch, status: query.status, page: query.page });
    const filtered = query.page > 1 || !!query.branch || !!query.status || (!only && !!query.repoId);
    // Filters (or paging) that show nothing: does the team have runs at all?
    const any = page.runs.length > 0 || (filtered && (only ? (await listTeamRuns(teamId, db, { repoId: only.id, perPage: 1 })).runs.length > 0 : await teamHasRuns(teamId, db)));
    return {
      link: gh.link,
      data: {
        runs: page.runs,
        hasMore: page.hasMore,
        query,
        any,
        repos: repos.map((r) => ({ slug: r.slug, fullName: r.fullName })),
        slugOf: Object.fromEntries(repos.map((r) => [r.fullName.toLowerCase(), r.slug])),
      },
    };
  } catch (e) {
    unstable_rethrow(e);
    console.warn("[quod-cloud] github runs unavailable", { error: e instanceof Error ? e.name : "unknown" });
    return { link: { state: "unavailable" }, data: null };
  }
}
