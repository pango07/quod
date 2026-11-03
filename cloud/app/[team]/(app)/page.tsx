import Link from "next/link";
import { LiveRefresh, LiveTotals, NowRunning } from "@/components/live/parts";
import { GitHubStatusCard } from "@/components/github-status";
import { QuodNote } from "@/components/section";
import { RangeToggle } from "@/components/range-toggle";
import { RepoTable } from "@/components/repo-table";
import { TestFilesLine, TestStatsCard } from "@/components/test-runs";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { hasTestActivity, spotChecksWithinWeek, testsByRepo } from "@/lib/test-runs";
import { testFilesLine } from "@/lib/test-suites";
import { formatCount } from "@/lib/format";
import { parseRange, RANGE_FROM } from "@/lib/params";
import { plainUi } from "@/lib/ui";

export const metadata = { title: "Overview" };

export default async function OverviewPage({ params, searchParams }: PageProps<"/[team]">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team, admin } = await requireTeam(slug);
  const [ov, activity, tests, suites, testRuns] = await Promise.all([
    api().getOverview(team.slug, range),
    api().getActivity(team.slug),
    section(api().getTestStats(team.slug)),
    section(api().listTestSuites(team.slug)),
    // The Repositories card's test numbers (7 days per repo); null (404/501) or down: the step columns as before.
    section(api().listTestRuns(team.slug, { limit: 200 })),
  ]);
  const repoTests = testRuns.ok && testRuns.value?.length ? testsByRepo(testRuns.value, Date.parse(ov.asOf)) : undefined;
  // Test runs: null from a server without them (404/501) or one that is down; then the card is simply not there.
  const testStats = tests.ok && hasTestActivity(tests.value) ? tests.value : null;
  const filesLine = suites.ok ? testFilesLine(suites.value) : null;
  // Shown whenever there are test runs, whatever the step-level data says (a team may run only `quod vitest`).
  // Spot-checks over the same 7 days as the rest, from the runs already read; without them, the stats' all-time count.
  const spotWeek = testRuns.ok && testRuns.value?.length ? spotChecksWithinWeek(testRuns.value, Date.parse(ov.asOf)) : null;
  // A team with test data sees test numbers; the step-level tiles stay only for teams with step data and no test data.
  const hasTests = !!(testStats || repoTests || filesLine);
  const testsCard = testStats ? (
    <TestStatsCard team={team.slug} stats={testStats} filesLine={filesLine} spotChecksWeek={spotWeek} />
  ) : filesLine ? (
    <section className="card" style={{ padding: 20 }}>
      <TestFilesLine team={team.slug} line={filesLine} />
    </section>
  ) : null;
  const base = `/${team.slug}`;

  const github = activity.github;
  if (ov.repos.length === 0) {
    return (
      <div className="page">
        <h1 className="h1">Overview</h1>
        <QuodNote status={ov.quod} />
        {/* No repos yet: say how CI gets here (GitHub App), not a dead dashboard. */}
        {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
        {github?.state === "linked" ? <NowRunning team={team.slug} initial={activity} /> : null}
        {testsCard ? <LiveRefresh scope={{ kind: "team" }} types={["test.run"]} /> : null}
        {testsCard}
        {testsCard ? null : github?.state === "linked" ? (
          <section className="card">
            <EmptyState
              title="No repositories yet"
              action={
                <Link className="btn lg" href={`${base}/settings#github-h`}>
                  Manage GitHub
                </Link>
              }
            >
              GitHub is connected, but the Quod CI App can&apos;t see any repositories. Choose some on GitHub, from Settings.
            </EmptyState>
          </section>
        ) : github ? null : (
          <section className="card">
            <EmptyState
              title="No repositories yet"
              action={
                <Link className="btn solid lg" href={`${base}/connect`}>
                  Connect repository
                </Link>
              }
            >
              Connect a repository and push. The first run only watches; from the second, steps whose inputs did not change come back in under a second.
            </EmptyState>
          </section>
        )}
      </div>
    );
  }

  const t = ov.totals;
  const plain = await plainUi(sp);
  const paused = ov.trust.paused;
  // Repos known only from GitHub have no Quod numbers yet.
  const quod = ov.repos.some((r) => r.hasQuodData !== false);
  return (
    <div className="page">
      <div className="row-between" style={{ gap: 16 }}>
        <h1 className="h1">Overview</h1>
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          <RangeToggle base={base} current={range} />
          {/* The sample team's demo flow; a real team connects from the GitHub card or Settings. */}
          {github ? null : (
            <Link className="btn solid" href={`${base}/connect`} style={{ fontSize: 14 }}>
              Connect repository
            </Link>
          )}
        </div>
      </div>

      <QuodNote status={ov.quod} />
      {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
      <NowRunning team={team.slug} initial={activity} />
      <LiveRefresh scope={{ kind: "team" }} />
      <LiveRefresh scope={{ kind: "team" }} types={["test.run"]} />

      {quod ? (
        <LiveTotals key={range} totals={t} days={ov.days} from={RANGE_FROM[range]} reseed={!activity.simulated} plain={plain} tiles={!hasTests} />
      ) : hasTests ? null : (
        <section className="card" style={{ padding: 20 }}>
          <h2 className="h2">{plain ? "Time saved and skips" : "Time saved and replays"}</h2>
          <p className="muted" style={{ fontSize: 14, marginTop: 6, lineHeight: 1.6 }}>
            Appears once Quod runs in one of these repositories. Until then, this page shows their CI from GitHub as it runs.
          </p>
        </section>
      )}

      {testsCard}

      <div className="split">
        <section className="card" aria-labelledby="repos-h" style={{ flex: "999 1 560px", overflow: "hidden" }}>
          <div className="card-head">
            <h2 id="repos-h" className="h2">
              Repositories
            </h2>
            <Link href={`${base}/repositories`} className="link-muted">
              View all
            </Link>
          </div>
          <RepoTable team={team.slug} repos={ov.repos} asOf={ov.asOf} tests={repoTests} />
        </section>

        {quod ? (
        <div style={{ flex: "1 1 300px", display: "flex", flexDirection: "column", gap: 16 }}>
          <section className="card" aria-labelledby="trust-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 14 }}>
            <div className="row-between">
              <h2 id="trust-h" className="h2">
                Trust
              </h2>
              <Link href={`${base}/trust`} className="link-muted">
                Details
              </Link>
            </div>
            <dl style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: 14, margin: 0 }}>
              {(plain
                ? [
                    ["Ready to skip or skipping", ov.trust.proven],
                    ["Learning", ov.trust.proving],
                    ["Re-checks with the same output", ov.trust.spotChecksPassed],
                  ]
                : [
                    ["Proven steps", ov.trust.proven],
                    ["Still proving", ov.trust.proving],
                    ["Spot checks passed", ov.trust.spotChecksPassed],
                  ]
              ).map(([k, v]) => (
                <div key={k} style={{ display: "flex", justifyContent: "space-between" }}>
                  <dt className="muted">{k}</dt>
                  <dd className="mono" style={{ margin: 0 }}>
                    {formatCount(v as number)}
                  </dd>
                </div>
              ))}
            </dl>
            {paused.length > 0 ? (
              <Link href={`${base}/${paused[0].repoSlug}/steps/${paused[0].key}`} className="notice">
                <Dot tone="amber" />
                <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                  <span style={{ fontWeight: 500 }}>{paused.length === 1 ? "1 step paused" : `${paused.length} steps paused`}</span>
                  <span className="soft">
                    {paused[0].repoName} · {paused[0].job} / {paused[0].command.replace(/^npx /, "")} gave a different result on a re-check. Quod runs it every time until it is consistent
                    again.
                  </span>
                </span>
              </Link>
            ) : (
              <p className="muted" style={{ fontSize: 13 }}>
                {plain ? "Nothing paused. Every re-check found the same output." : "No paused steps. Every spot check matched."}
              </p>
            )}
          </section>
          <section className="card" aria-labelledby="runners-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
            <div className="row-between">
              <h2 id="runners-h" className="h2">
                Runners
              </h2>
              <Link href={`${base}/connect`} className="link-muted">
                Add
              </Link>
            </div>
            {ov.runners.length ? null : (
              // quod-server does not list runners (they come from GitHub's jobs): point there.
              <p className="muted" style={{ fontSize: 13 }}>
                <Link href={`${base}/runners`} style={{ textDecoration: "underline" }}>
                  Runners seen in your CI
                </Link>
              </p>
            )}
            <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 12 }}>
              {ov.runners.map((r) => (
                <li key={r.name} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 10, fontSize: 13 }}>
                  <span style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                    <Dot tone={r.status === "running" ? "blue" : r.status === "offline" ? "dim" : "green"} />
                    <span className="mono ellipsis">{r.name}</span>
                  </span>
                  <span className="faint" style={{ whiteSpace: "nowrap" }}>
                    {r.kind === "github-hosted" ? "via quodci/setup" : `Self-hosted · ${r.status}`}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        </div>
        ) : null}
      </div>
    </div>
  );
}
