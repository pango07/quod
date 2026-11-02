import { notFound } from "next/navigation";
import { GitHubRunView } from "@/components/github-run-view";
import { GitHubRunLive } from "@/components/live/github-run-live";
import { api } from "@/lib/api";
import { quodMarks, sameGitHubRun } from "@/lib/api/github-merge";
import { section } from "@/lib/api/section";
import type { QuodStatus } from "@/lib/api/types";
import { requireTeam } from "@/lib/dal";
import { getTeamRun } from "@/lib/github/pages";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Workflow run" };

// A GitHub workflow run, by GitHub's run id (run numbers are per workflow on
// GitHub, so not unique in a repo; Quod's runs keep /[repo]/runs/[n]). Where
// Quod ran in it, each step says whether it was replayed or ran.
export default async function GitHubRunPage({ params }: PageProps<"/[team]/[repo]/actions/[runId]">) {
  const { team: slug, repo: repoSlug, runId } = await params;
  if (!/^\d{1,15}$/.test(runId)) notFound();
  const { team } = await requireTeam(slug);
  const gh = await teamGitHub(team.orgId);
  const repo = gh.source?.repos.find((r) => r.slug === repoSlug);
  if (!gh.source || !repo) notFound();
  const found = await getTeamRun(gh.source.teamId, gh.source.db, Number(runId));
  // The run must belong to this repo (and so to this team).
  if (!found || found.run.repoId !== repo.id) notFound();
  // Quod's record of the same run: by run number, and only if it names this GitHub run.
  const quod = found.run.runNumber ? await section(api().getRun(team.slug, repo.slug, found.run.runNumber)) : null;
  const quodRun = quod?.ok && quod.value && sameGitHubRun(quod.value.githubUrl, found.run.id) ? quod.value : null;
  const quodStatus: QuodStatus | undefined = quod && !quod.ok && quod.kind === "unavailable" ? { state: "unavailable", message: quod.message } : undefined;
  const asOf = new Date().toISOString();
  const live = found.run.status !== "completed" || found.jobs.some((j) => j.status !== "completed");
  return (
    <div className="page tight">
      {live ? <GitHubRunLive runId={found.run.id} /> : null}
      <GitHubRunView team={team.slug} repo={repo.slug} run={found.run} jobs={found.jobs} asOf={asOf} quod={quodMarks(found.jobs, quodRun)} quodStatus={quodStatus} />
    </div>
  );
}
