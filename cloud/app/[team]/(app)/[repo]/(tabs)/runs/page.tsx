import { notFound } from "next/navigation";
import { ciState } from "@/components/github-ci";
import { RunsView } from "@/components/runs-view";
import { QuodNote } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { githubRunsFor } from "@/lib/github/runs-data";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Runs" };

export default async function RepoRunsPage({ params, searchParams }: PageProps<"/[team]/[repo]/runs">) {
  const [{ team: slug, repo }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const [{ asOf, runs, quod }, gh, summary] = await Promise.all([api().listRuns(team.slug, { repo }), teamGitHub(team.orgId), section(api().getRepo(team.slug, repo))]);
  // Not found only when quod-server and GitHub both say so (a server that is down proves nothing).
  if (summary.ok && !summary.value && runs.length === 0) notFound();
  const ghRepo = gh.source?.repos.find((r) => r.slug === repo);
  const { link, data } = ghRepo ? await githubRunsFor(gh, sp, ghRepo) : { link: gh.link, data: null };
  return (
    <div className="page">
      <h1 className="sr-only">Runs of {repo}</h1>
      <QuodNote status={quod} />
      <RunsView team={team.slug} asOf={asOf} state={ciState(link, runs.length > 0 || !!data?.any)} quodRuns={runs} github={data} base={`/${team.slug}/${repo}/runs`} repo={repo} />
    </div>
  );
}
