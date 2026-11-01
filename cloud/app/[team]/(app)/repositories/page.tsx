import Link from "next/link";
import { CiEmpty, ciState } from "@/components/github-ci";
import { RepoTable } from "@/components/repo-table";
import { EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Repositories" };

export default async function RepositoriesPage({ params }: PageProps<"/[team]/repositories">) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const [ov, gh] = await Promise.all([api().getOverview(team.slug, "7d"), teamGitHub(team.orgId)]);
  const state = gh.sample ? "sample" : ciState(gh.link, ov.repos.length > 0);
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Repositories</h1>
        {gh.sample || state === "connect" ? (
          <Link className="btn solid" href={`/${team.slug}/connect`} style={{ fontSize: 14 }}>
            {gh.sample ? "Connect repository" : "Connect GitHub"}
          </Link>
        ) : null}
      </div>
      <section className="card" aria-label="Repositories" style={{ overflow: "hidden" }}>
        {ov.repos.length ? (
          <RepoTable team={team.slug} repos={ov.repos} asOf={ov.asOf} savedLabel="Saved, 7 days" />
        ) : state === "waiting" ? (
          <EmptyState
            title="No repositories yet"
            action={
              <Link className="btn lg" href={`/${team.slug}/settings#github-h`}>
                Manage GitHub
              </Link>
            }
          >
            GitHub is connected, but the Quod CI App can&apos;t see any repositories. Choose some on GitHub, from Settings.
          </EmptyState>
        ) : (
          <CiEmpty
            team={team.slug}
            state={state}
            what="repositories"
            sample={<EmptyState title="No repositories yet">Connect GitHub to see your repositories and their CI live.</EmptyState>}
          />
        )}
      </section>
    </div>
  );
}
