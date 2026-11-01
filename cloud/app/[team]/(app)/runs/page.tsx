import Link from "next/link";
import { RunsTable } from "@/components/runs-table";
import { EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Runs" };

export default async function RunsPage({ params }: PageProps<"/[team]/runs">) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const { asOf, runs } = await api().listRuns(team.slug, { limit: 50 });
  return (
    <div className="page">
      <h1 className="h1">Runs</h1>
      <section className="card" aria-label="Recent runs" style={{ overflow: "hidden" }}>
        {runs.length ? (
          <RunsTable team={team.slug} runs={runs} asOf={asOf} showRepo />
        ) : (
          <EmptyState
            title="No runs yet"
            action={
              <Link className="btn solid lg" href={`/${team.slug}/connect`}>
                Connect repository
              </Link>
            }
          >
            Runs show up here after the first push to a connected repository.
          </EmptyState>
        )}
      </section>
    </div>
  );
}
