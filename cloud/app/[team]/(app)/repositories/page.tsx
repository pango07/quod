import Link from "next/link";
import { RepoTable } from "@/components/repo-table";
import { EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Repositories" };

export default async function RepositoriesPage({ params }: PageProps<"/[team]/repositories">) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const ov = await api().getOverview(team.slug, "7d");
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Repositories</h1>
        <Link className="btn solid" href={`/${team.slug}/connect`} style={{ fontSize: 14 }}>
          Connect repository
        </Link>
      </div>
      <section className="card" aria-label="Repositories" style={{ overflow: "hidden" }}>
        {ov.repos.length ? (
          <RepoTable team={team.slug} repos={ov.repos} asOf={ov.asOf} savedLabel="Saved, 7 days" />
        ) : (
          <EmptyState
            title="No repositories yet"
            action={
              <Link className="btn solid lg" href={`/${team.slug}/connect`}>
                Connect repository
              </Link>
            }
          >
            Connect GitHub to see your repositories and their CI live.
          </EmptyState>
        )}
      </section>
    </div>
  );
}
