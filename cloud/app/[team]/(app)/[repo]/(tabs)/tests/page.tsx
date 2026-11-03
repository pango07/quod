import { notFound } from "next/navigation";
import { LiveRefresh } from "@/components/live/parts";
import { SectionUnavailable } from "@/components/section";
import { TestRunsTable } from "@/components/test-runs";
import { EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Tests" };

export default async function RepoTestsPage({ params }: PageProps<"/[team]/[repo]/tests">) {
  const { team: slug, repo } = await params;
  const { team } = await requireTeam(slug);
  const [found, summary] = await Promise.all([section(api().listTestRuns(team.slug, { repo, limit: 100 })), section(api().getRepo(team.slug, repo))]);
  const runs = found.ok ? (found.value ?? []) : [];
  // Not found only when the repo is unknown and has no test runs either (a server that is down proves nothing).
  if (summary.ok && !summary.value && runs.length === 0) notFound();
  const asOf = new Date().toISOString();
  return (
    <div className="page">
      <h1 className="sr-only">Test runs of {repo}</h1>
      <LiveRefresh scope={{ kind: "repo", repo }} types={["test.run"]} />
      {!found.ok ? (
        <SectionUnavailable s={found} what="Test runs" />
      ) : runs.length ? (
        <section className="card" aria-labelledby="tests-h" style={{ overflow: "hidden" }}>
          <div className="card-head">
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <h2 id="tests-h" className="h2">
                Test runs
              </h2>
              <p className="muted" style={{ fontSize: 13, lineHeight: 1.5, maxWidth: "72ch" }}>
                Each run lists how many test files were skipped because nothing they depend on changed (their earlier result was reused), and how many ran.
              </p>
            </div>
          </div>
          <TestRunsTable team={team.slug} runs={runs} asOf={asOf} />
        </section>
      ) : (
        <section className="card">
          <EmptyState title="No test runs yet">
            Quod can skip test files whose code did not change since a recorded run. Test runs show up here once this repository&apos;s tests run with Quod.
          </EmptyState>
        </section>
      )}
    </div>
  );
}
