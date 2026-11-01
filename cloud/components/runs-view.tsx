import Link from "next/link";
import { CiEmpty, GitHubRunsTable, Pager, RunFilters, type CiState } from "@/components/github-ci";
import { RunsTable } from "@/components/runs-table";
import { EmptyState } from "@/components/ui";
import type { RunSummary } from "@/lib/api/types";
import type { GhRun, RunQuery } from "@/lib/github/pages";

export interface GitHubRunsData {
  runs: GhRun[];
  hasMore: boolean;
  query: RunQuery;
  /** Does the team (or repo) have any GitHub run at all, filters aside? */
  any: boolean;
  repos: { slug: string; fullName: string }[];
  /** lowercase full name -> repo slug on this dashboard */
  slugOf: Record<string, string>;
}

/**
 * The Runs list (team-wide, or one repo's): Quod's runs once there are any,
 * the team's GitHub workflow runs when linked, and otherwise the empty state
 * for where the team stands (sample, connect, waiting for a first run).
 */
export function RunsView({
  team,
  asOf,
  state,
  quodRuns,
  github,
  base,
  repo,
}: {
  team: string;
  asOf: string;
  state: CiState;
  quodRuns: RunSummary[];
  github: GitHubRunsData | null;
  /** This page's path, for filters and paging. */
  base: string;
  /** Set on a repo's Runs tab: no repo column or filter. */
  repo?: string;
}) {
  const showGitHub = github !== null && github.any;
  if (!quodRuns.length && !showGitHub) {
    return (
      <section className="card" aria-label="Runs" style={{ overflow: "hidden" }}>
        <CiEmpty
          team={team}
          state={state}
          what="runs"
          sample={<EmptyState title="No runs yet">Runs show up here after the first push.</EmptyState>}
        />
      </section>
    );
  }
  const q = github?.query;
  const filtered = !!q && (!!q.branch || !!q.status || (!repo && !!q.repoSlug));
  return (
    <>
      {quodRuns.length ? (
        <section className="card" aria-labelledby="quod-runs-h" style={{ overflow: "hidden" }}>
          {showGitHub ? (
            <div className="card-head">
              <h2 id="quod-runs-h" className="h2">
                With Quod
              </h2>
            </div>
          ) : (
            <h2 id="quod-runs-h" className="sr-only">
              Runs
            </h2>
          )}
          <RunsTable team={team} runs={quodRuns} asOf={asOf} showRepo={!repo} />
        </section>
      ) : null}
      {showGitHub && github && q ? (
        <section className="card" aria-labelledby="gh-runs-h" style={{ overflow: "hidden" }}>
          {quodRuns.length ? null : (
            <h2 id="gh-runs-h" className="sr-only">
              Workflow runs from GitHub
            </h2>
          )}
          <div className="card-head" style={{ flexDirection: "column", alignItems: "stretch", gap: 12 }}>
            {quodRuns.length ? (
              <h2 id="gh-runs-h" className="h2">
                Workflow runs from GitHub
              </h2>
            ) : null}
            <RunFilters action={base} repos={repo ? undefined : github.repos} current={{ repo: q.repoSlug ?? undefined, branch: q.branch ?? undefined, status: q.status }} />
          </div>
          {github.runs.length ? (
            <GitHubRunsTable team={team} runs={github.runs} slugOf={github.slugOf} asOf={asOf} showRepo={!repo} />
          ) : (
            <p className="trow muted" style={{ display: "block" }}>
              {filtered ? (
                <>
                  No runs match these filters.{" "}
                  <Link href={base} style={{ textDecoration: "underline" }}>
                    Show all runs
                  </Link>
                </>
              ) : (
                "No more runs."
              )}
            </p>
          )}
          <div style={{ padding: "12px 20px", borderTop: "1px solid var(--rule)" }}>
            <Pager
              base={base}
              query={{ repo: repo ? undefined : (q.repoSlug ?? undefined), branch: q.branch ?? undefined, status: q.status ?? undefined }}
              page={q.page}
              hasMore={github.hasMore}
            />
          </div>
        </section>
      ) : null}
    </>
  );
}
