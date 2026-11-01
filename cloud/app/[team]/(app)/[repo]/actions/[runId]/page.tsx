import { notFound } from "next/navigation";
import { GitHubRunView } from "@/components/github-run-view";
import { GitHubRunLive } from "@/components/live/github-run-live";
import { requireTeam } from "@/lib/dal";
import { getTeamRun } from "@/lib/github/pages";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Workflow run" };

// A GitHub workflow run, by GitHub's run id (run numbers are per workflow on
// GitHub, so not unique in a repo; Quod's runs keep /[repo]/runs/[n]).
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
  const asOf = new Date().toISOString();
  const live = found.run.status !== "completed" || found.jobs.some((j) => j.status !== "completed");
  return (
    <div className="page tight">
      {live ? <GitHubRunLive runId={found.run.id} /> : null}
      <GitHubRunView team={team.slug} repo={repo.slug} run={found.run} jobs={found.jobs} asOf={asOf} />
    </div>
  );
}
