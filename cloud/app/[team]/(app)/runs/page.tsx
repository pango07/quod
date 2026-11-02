import { ciState } from "@/components/github-ci";
import { RunsView } from "@/components/runs-view";
import { QuodNote } from "@/components/section";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { githubRunsFor } from "@/lib/github/runs-data";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Runs" };

export default async function RunsPage({ params, searchParams }: PageProps<"/[team]/runs">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const [{ asOf, runs, quod }, gh] = await Promise.all([api().listRuns(team.slug, { limit: 50 }), teamGitHub(team.orgId)]);
  const { link, data } = await githubRunsFor(gh, sp);
  return (
    <div className="page">
      <h1 className="h1">Runs</h1>
      <QuodNote status={quod} />
      <RunsView team={team.slug} asOf={asOf} state={ciState(link, runs.length > 0 || !!data?.any)} quodRuns={runs} github={data} base={`/${team.slug}/runs`} />
    </div>
  );
}
