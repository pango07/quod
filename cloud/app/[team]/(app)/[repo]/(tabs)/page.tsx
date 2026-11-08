import Link from "next/link";
import { notFound } from "next/navigation";
import { RunTimeChart } from "@/components/charts";
import { RerunButton } from "@/components/rerun-button";
import { StepsTable } from "@/components/steps-table";
import { ObservedCiComparisons } from "@/components/observed-ci-comparisons";
import { LiveRefresh, NowRunning } from "@/components/live/parts";
import { ProtectionBadge, QuodNote, SectionUnavailable } from "@/components/section";
import { Dot, EmptyState, OutcomeChip, StepDot } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { plainUi } from "@/lib/ui";
import { observedCiComparisons } from "@/lib/observed-ci-comparisons";
import { CI_COMPARISON } from "@/lib/ci-comparisons";
import { runnerRate } from "@/lib/team-settings";
import { formatAgo, formatCount, formatDuration, median, shortSha } from "@/lib/format";
import { parseStepFilter } from "@/lib/params";
import { exitLabel, formatSaved, formatSavedMs, legacyRecordingIds, roleLabel, runRole, SAVED_EXPLAINED, suiteLabel, testRunHref } from "@/lib/test-runs";

export const metadata = { title: "Repository" };

const SHOWN = 6;

export default async function RepoPage({ params, searchParams }: PageProps<"/[team]/[repo]">) {
  const [{ team: slug, repo: repoSlug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const [found, activity, testReports, comparisons, rate] = await Promise.all([
    section(api().getRepo(team.slug, repoSlug)),
    api().getActivity(team.slug, { repo: repoSlug }),
    section(api().listTestRuns(team.slug, { repo: repoSlug, limit: 100 })),
    observedCiComparisons(team, "7d"),
    runnerRate(team.orgId),
  ]);
  const valuationRate = rate.canSave ? rate : undefined;
  // quod-server down and GitHub does not know the repo either: say so, do not 404 a repo that may exist.
  if (!found.ok) return <SectionUnavailable s={found} what="This repository" page />;
  const repo = found.value;
  if (!repo) notFound();
  const base = `/${team.slug}/${repo.summary.slug}`;
  const plain = await plainUi(sp);
  const run = repo.latestRun;
  const tests = testReports.ok ? (testReports.value ?? []) : [];
  const latestTest = tests[0]; // The API returns newest first, across suites and branches.
  const testResult = latestTest ? exitLabel(latestTest.exit) : null;
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
      <LiveRefresh scope={{ kind: "repo", repo: repo.summary.slug }} types={["job.finished", "run.finished", "test.run"]} />
      <QuodNote status={repo.quod} />
      {repo.summary.protection ? (
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", fontSize: 13 }}>
          {/* The team banner above says what to do; this says which state this repo is in. */}
          <span className="faint">Default branch</span>
          <ProtectionBadge p={repo.summary.protection} />
        </div>
      ) : null}
      <NowRunning team={team.slug} initial={activity} repo={repo.summary.slug} />
      {repo.summary.fullName === CI_COMPARISON.repository ? <ObservedCiComparisons data={comparisons} valuationRate={valuationRate} /> : null}
      {!testReports.ok ? <SectionUnavailable s={testReports} what="Test reports" /> : testReports.value === null ? (
        <SectionUnavailable s={{ ok: false, kind: "not-available", message: "Test reports are not supported by this server." }} what="Test reports" />
      ) : null}
      {latestTest && testResult ? (
        <section className="card" aria-labelledby="latest-test-h" style={{ overflow: "hidden" }}>
          <div className="card-head ruled">
            <h2 id="latest-test-h" className="h2">Latest test run</h2>
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              <Link className="btn" href={`${base}/tests`}>All test runs</Link>
              <Link className="btn solid" href={testRunHref(team.slug, latestTest)}>View test run</Link>
            </div>
          </div>
          <div style={{ padding: 20 }}>
            <dl style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(140px, 1fr))", gap: 20, fontSize: 14, margin: 0 }}>
              <Fact k="Test duration">{formatSavedMs(latestTest.timing.wallMs)}</Fact>
              <Fact k="Files ran">{formatCount(latestTest.files.ran)}</Fact>
              <Fact k="Files reused">{formatCount(latestTest.files.replayed)}</Fact>
              <Fact k="Result"><span style={{ display: "flex", alignItems: "center", gap: 8 }}><Dot tone={testResult.tone} />{testResult.text}</span></Fact>
              <Fact k="Suite">{suiteLabel(latestTest)}</Fact>
              <Fact k="Reported"><time dateTime={latestTest.at}>{formatAgo(latestTest.at, repo.asOf)}</time></Fact>
              <Fact k="Source" wide>
                <span className="mono">{latestTest.branch ?? "Branch not reported"}{latestTest.sha ? ` · ${shortSha(latestTest.sha)}` : ""}</span>
                {latestTest.githubUrl ? <a href={latestTest.githubUrl} className="link-muted" target="_blank" rel="noreferrer">View on GitHub</a> : null}
              </Fact>
            </dl>
            <p className="muted" style={{ fontSize: 13, marginTop: 20, lineHeight: 1.6 }}>
              {roleLabel(runRole(latestTest, legacyRecordingIds(tests)))}. Test duration covers the test command; setup and cleanup add to the full CI job.
              {latestTest.files.replayed === 0 ? " No test files were reused in this run." : latestTest.timing.savedMs !== null ? ` Estimated test execution avoided: ${formatSaved(latestTest.timing.savedMs)}. ${SAVED_EXPLAINED}` : ""}
            </p>
          </div>
        </section>
      ) : null}
      {run || (!latestTest && testReports.ok && testReports.value !== null) ? (
        <section className="card" aria-labelledby="latest-h" style={{ overflow: "hidden" }}>
          <div className="card-head ruled">
            <h2 id="latest-h" className="h2">
              Latest step run
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
                  <li key={i} className="step-line" data-state={s.outcome}>
                    <StepDot outcome={s.outcome} />
                    <span className="ellipsis">{s.name}</span>
                    <OutcomeChip outcome={s.outcome} plain={plain} />
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
                    {run.status === "running" ? <span className="sr-only">(updates live)</span> : null}
                    {run.status === "passed" ? "Passed" : run.status === "failed" ? "Failed" : "Running"}
                  </span>
                </Fact>
                <Fact k="Step execution">
                  {formatDuration(run.tookSeconds)} <span className="faint">· {formatDuration(run.wouldHaveTakenSeconds)} without replay (estimated)</span>
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
              <EmptyState title="No Quod reports received">
                Check that Cloud reporting is enabled in your Quod action. Running CI on GitHub alone does not send test results here.{" "}
                {repo.summary.github?.latestRun ? (
                  <>
                    <Link href={`${base}/actions/${repo.summary.github.latestRun.id}`} style={{ textDecoration: "underline" }}>
                      Latest GitHub run
                    </Link>
                    {" · "}
                    <Link href={`${base}/runs`} style={{ textDecoration: "underline" }}>
                      All runs
                    </Link>
                  </>
                ) : null}
              </EmptyState>
          )}
        </section>
      ) : null}

      {repo.runTimes.length > 0 ? (
        <section className="card" aria-labelledby="time-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="row-between">
            <h2 id="time-h" className="h2">
              Step execution, last {repo.runTimes.length} runs on {repo.defaultBranch}
            </h2>
            <div className="legend" aria-hidden="true">
              <span>
                <span className="swatch" style={{ background: "var(--text)" }} />
                Observed steps
              </span>
              <span>
                <span className="swatch" style={{ border: "1px dashed #666" }} />
                Without replay (estimated)
              </span>
            </div>
          </div>
          <RunTimeChart runs={repo.runTimes} />
          <p className="muted" style={{ fontSize: 13 }}>
            Median step execution: <span style={{ color: "var(--text)" }}>{formatDuration(withQ)}</span> observed, <span style={{ color: "var(--text)" }}>{formatDuration(without)}</span> without replay (estimated).
            {" "}These totals cover reported Quod commands, excluding other work in the job. They are separate from complete GitHub job durations.
          </p>
        </section>
      ) : null}

      {repo.steps.length > 0 ? <StepsTable team={team.slug} steps={repo.steps} initial={parseStepFilter(sp.state)} /> : null}
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
