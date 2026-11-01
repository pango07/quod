import Link from "next/link";
import { notFound } from "next/navigation";
import { RunTimeChart } from "@/components/charts";
import { RerunButton } from "@/components/rerun-button";
import { StepsTable } from "@/components/steps-table";
import { Dot, EmptyState, OutcomeChip } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { formatDuration, median, shortSha } from "@/lib/format";
import { parseStepFilter } from "@/lib/params";

export const metadata = { title: "Repository" };

const SHOWN = 6;

export default async function RepoPage({ params, searchParams }: PageProps<"/[team]/[repo]">) {
  const [{ team: slug, repo: repoSlug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const repo = await api().getRepo(team.slug, repoSlug);
  if (!repo) notFound();
  const base = `/${team.slug}/${repo.summary.slug}`;
  const run = repo.latestRun;
  // One list for the card: runner setup and checkout once, other steps named by job when there are several.
  const jobs = run?.jobs ?? [];
  const steps = jobs.flatMap((j, ji) =>
    j.steps
      .filter((s) => ji === 0 || s.why.kind !== "not-memoized")
      .map((s) => (jobs.length > 1 && s.why.kind !== "not-memoized" ? { ...s, name: `${j.name.split(" / ")[0]}: ${s.name}` } : s)),
  );
  const withQ = median(repo.runTimes.map((r) => r.withQuodSeconds));
  const without = median(repo.runTimes.map((r) => r.withoutSeconds));

  return (
    <div className="page">
      <h1 className="sr-only">{repo.summary.fullName}</h1>
      <section className="card" aria-labelledby="latest-h" style={{ overflow: "hidden" }}>
        <div className="card-head ruled">
          <h2 id="latest-h" className="h2">
            Latest run
          </h2>
          {run ? (
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
              <RerunButton team={team.slug} repo={repo.summary.slug} run={run.number} />
              <Link className="btn solid" href={`${base}/runs/${run.number}`}>
                View run
              </Link>
            </div>
          ) : null}
        </div>
        {run ? (
          <div style={{ display: "flex", flexWrap: "wrap", gap: 24, padding: 20 }}>
            <ul
              aria-label={`Steps in run #${run.number}`}
              style={{ flex: "1 1 360px", minWidth: 0, border: "1px solid var(--rule)", borderRadius: 10, padding: 14, display: "flex", flexDirection: "column", gap: 10, background: "var(--well)", margin: 0, listStyle: "none" }}
            >
              {steps.slice(0, SHOWN).map((s, i) => (
                <li key={i} className="step-line">
                  <Dot tone={s.outcome === "replayed" ? "blue" : "gray"} size="lg" />
                  <span className="ellipsis">{s.name}</span>
                  <OutcomeChip outcome={s.outcome} />
                  <span className="mono" style={{ textAlign: "right", color: "var(--muted)" }}>
                    {formatDuration(s.seconds)}
                  </span>
                </li>
              ))}
              {steps.length > SHOWN ? (
                <li style={{ fontSize: 13 }}>
                  <Link href={`${base}/runs/${run.number}`} className="link-muted">
                    {steps.length - SHOWN} more {steps.length - SHOWN === 1 ? "step" : "steps"} in this run
                  </Link>
                </li>
              ) : null}
            </ul>
            <dl style={{ flex: "1 1 320px", display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: "18px 24px", alignContent: "start", fontSize: 14, margin: 0 }}>
              <Fact k="Run">
                <span className="mono">#{run.number}</span>
              </Fact>
              <Fact k="Status">
                <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <Dot tone={run.status === "passed" ? "green" : run.status === "failed" ? "red" : "blue"} />
                  {run.status === "passed" ? "Passed" : run.status === "failed" ? "Failed" : "Running"}
                </span>
              </Fact>
              <Fact k="Duration">
                {formatDuration(run.tookSeconds)} <span className="faint">instead of {formatDuration(run.wouldHaveTakenSeconds)}</span>
              </Fact>
              <Fact k="Runner">
                <span className="mono">{run.runner}</span>
              </Fact>
              <Fact k="Source" wide>
                <span className="mono" style={{ fontSize: 13 }}>
                  {run.branch} · {shortSha(run.sha)} {run.title}
                </span>
              </Fact>
            </dl>
          </div>
        ) : (
          <EmptyState title="No runs yet">Push to {repo.defaultBranch}. The first run only watches and notes what each step reads.</EmptyState>
        )}
      </section>

      {repo.runTimes.length > 0 ? (
        <section className="card" aria-labelledby="time-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="row-between">
            <h2 id="time-h" className="h2">
              Run time, last {repo.runTimes.length} runs on {repo.defaultBranch}
            </h2>
            <div className="legend" aria-hidden="true">
              <span>
                <span className="swatch" style={{ background: "var(--text)" }} />
                With Quod
              </span>
              <span>
                <span className="swatch" style={{ border: "1px dashed #666" }} />
                Without (recorded)
              </span>
            </div>
          </div>
          <RunTimeChart runs={repo.runTimes} />
          <p className="muted" style={{ fontSize: 13 }}>
            Median run: <span style={{ color: "var(--text)" }}>{formatDuration(withQ)}</span> with Quod, <span style={{ color: "var(--text)" }}>{formatDuration(without)}</span> without.
            {repo.summary.language === "Rust" ? " Docs-only commits replay almost everything; commits that change Rust code still build." : " Commits that change only docs replay almost everything; code changes still run the steps that read them."}
          </p>
        </section>
      ) : null}

      <StepsTable team={team.slug} steps={repo.steps} initial={parseStepFilter(sp.state)} />
    </div>
  );
}

function Fact({ k, children, wide }: { k: string; children: React.ReactNode; wide?: boolean }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4, gridColumn: wide ? "1 / -1" : undefined, minWidth: 0 }}>
      <dt style={{ fontSize: 12, color: "var(--faint)" }}>{k}</dt>
      <dd style={{ margin: 0, overflowWrap: "anywhere" }}>{children}</dd>
    </div>
  );
}
