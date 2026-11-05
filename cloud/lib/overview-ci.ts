import "server-only";
import { unstable_rethrow } from "next/navigation";
import { api } from "./api";
import type { Range } from "./api/types";
import { median } from "./format";
import { listTeamRuns } from "./github/pages";
import { teamGitHub } from "./github/team-data";

export interface OverviewCi {
  medianSeconds: number | null;
  measuredRuns: number;
  sample: boolean;
  runs: { id:string; title:string; repo:string; seconds:number; status:string; url:string }[];
}

/** Actual workflow elapsed time from GitHub, never a sum of concurrent steps/jobs. */
export async function overviewCi(team: {slug:string; orgId:string}, range:Range): Promise<OverviewCi> {
  const empty: OverviewCi = { medianSeconds:null, measuredRuns:0, sample:false, runs:[] };
  try {
    const gh = await teamGitHub(team.orgId);
    const cutoff = Date.now() - Number(range.slice(0,-1))*86400000;
    let runs: OverviewCi["runs"];
    if (gh.sample) {
      const data = await api().listRuns(team.slug, {limit:100});
      runs = data.runs.filter(r => r.status !== "running" && Date.parse(r.startedAt) >= cutoff)
        .map(r => ({id:`${r.repoSlug}-${r.number}`, title:r.title, repo:r.repoSlug, seconds:r.tookSeconds, status:r.status, url:`/${team.slug}/${r.repoSlug}/runs/${r.number}`}));
    } else if (gh.source) {
      const {teamId, db} = gh.source;
      const data = await listTeamRuns(teamId, db, {perPage:100});
      runs = data.runs.filter(r => r.status === "completed" && r.startedAt && Date.parse(r.startedAt) >= cutoff && r.durationSeconds !== null)
        .map(r => ({id:String(r.id), title:r.title, repo:r.repoFullName, seconds:r.durationSeconds!, status:r.outcome, url:r.htmlUrl}));
    } else return empty;
    runs = runs.filter(r => Number.isFinite(r.seconds) && r.seconds >= 0);
    return {medianSeconds:runs.length ? median(runs.map(r => r.seconds)) : null, measuredRuns:runs.length, sample:gh.sample, runs:runs.slice(0,6)};
  } catch (e) {
    unstable_rethrow(e);
    console.warn("[quod-cloud] overview duration unavailable", {error:e instanceof Error ? e.name : "unknown"});
    return empty;
  }
}
