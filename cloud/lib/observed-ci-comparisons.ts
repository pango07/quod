import "server-only";
import { unstable_rethrow } from "next/navigation";
import type { Range } from "./api/types";
import { CI_COMPARISON, compareCiJobs, isComparisonRun, type CiComparison } from "./ci-comparisons";
import type { Db } from "./db/types";
import { getTeamRun, listTeamRuns } from "./github/pages";
import type { TeamId } from "./github/team";
import { teamGitHub } from "./github/team-data";

export interface ObservedCiComparisons {
  visible: boolean;
  available: boolean;
  inspectedRuns: number;
  pairs: CiComparison[];
}

/** A bounded read of the linked team's latest 100 repository runs, at most 12 details. */
export async function readCiComparisons(teamId: TeamId, db: Db, range: Range, now = new Date()): Promise<ObservedCiComparisons> {
  const cutoff = now.getTime() - Number(range.slice(0, -1)) * 86_400_000;
  const { runs } = await listTeamRuns(teamId, db, { repoId: CI_COMPARISON.repoId, perPage: 100 });
  const candidates = runs.filter(run => isComparisonRun(run) && run.startedAt && Date.parse(run.startedAt) >= cutoff && Date.parse(run.startedAt) <= now.getTime()).slice(0, 12);
  const observations = await Promise.all(candidates.map(async listed => {
    const detail = await getTeamRun(teamId, db, listed.id);
    // A new attempt may have begun between reads. Never combine either attempt's observations.
    if (!detail || detail.run.runAttempt !== listed.runAttempt || detail.run.sha !== listed.sha) return null;
    const pair = compareCiJobs(detail.run, detail.jobs);
    return pair && Date.parse(pair.finishedAt) <= now.getTime() ? pair : null;
  }));
  const pairs = observations.filter((pair): pair is CiComparison => pair !== null);
  return { visible: true, available: true, inspectedRuns: candidates.length, pairs };
}

/** Membership-checked team context; samples and unrelated repositories never acquire dogfood data. */
export async function observedCiComparisons(team: { orgId: string }, range: Range, repository?: string): Promise<ObservedCiComparisons> {
  const empty: ObservedCiComparisons = { visible: false, available: false, inspectedRuns: 0, pairs: [] };
  if (repository !== undefined && repository !== CI_COMPARISON.repository) return empty;
  let visible = false;
  try {
    const gh = await teamGitHub(team.orgId);
    if (!gh.source || !gh.source.repos.some(repo => repo.id === CI_COMPARISON.repoId && repo.fullName === CI_COMPARISON.repository)) return empty;
    visible = true;
    return await readCiComparisons(gh.source.teamId, gh.source.db, range);
  } catch (error) {
    unstable_rethrow(error);
    console.warn("[quod-cloud] observed CI comparison unavailable", { error: error instanceof Error ? error.name : "unknown" });
    return { ...empty, visible };
  }
}
