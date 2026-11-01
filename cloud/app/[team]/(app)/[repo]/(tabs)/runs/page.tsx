import { notFound } from "next/navigation";
import { RunsTable } from "@/components/runs-table";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Runs" };

export default async function RepoRunsPage({ params }: PageProps<"/[team]/[repo]/runs">) {
  const { team: slug, repo } = await params;
  const { team } = await requireTeam(slug);
  const { asOf, runs } = await api().listRuns(team.slug, { repo });
  if (runs.length === 0 && !(await api().getRepo(team.slug, repo))) notFound();
  return (
    <div className="page">
      <h1 className="sr-only">Runs of {repo}</h1>
      <section className="card" aria-label="Runs" style={{ overflow: "hidden" }}>
        {runs.length ? <RunsTable team={team.slug} runs={runs} asOf={asOf} /> : <p className="trow muted">No runs yet.</p>}
      </section>
    </div>
  );
}
