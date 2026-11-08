import { unstable_rethrow } from "next/navigation";
import { MinutesChart } from "@/components/charts";
import { ciState } from "@/components/github-ci";
import { GitHubUsage } from "@/components/usage-view";
import { RangeToggle } from "@/components/range-toggle";
import { Stat } from "@/components/ui";
import { LiveRefresh } from "@/components/live/parts";
import { ObservedCiComparisons } from "@/components/observed-ci-comparisons";
import { SavingsSummary } from "@/components/savings-summary";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import type { GitHubLink } from "@/lib/api/types";
import { requireTeam } from "@/lib/dal";
import { teamJobMinutes, type JobMinutes } from "@/lib/github/pages";
import { teamGitHub } from "@/lib/github/team-data";
import { skippedExecution } from "@/lib/job-savings";
import { observedCiComparisons } from "@/lib/observed-ci-comparisons";
import { overviewCi } from "@/lib/overview-ci";
import { runnerRate } from "@/lib/team-settings";
import { parseRange, RANGE_FROM } from "@/lib/params";
import { lite } from "@/lib/savings";
import { lastUtcDays, testMinutesPerDay } from "@/lib/test-runs";

export const metadata = { title: "Usage" };

export default async function UsagePage({ params, searchParams }: PageProps<"/[team]/usage">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team } = await requireTeam(slug);
  const [ov, gh, ci, comparisons, history, recent, rate] = await Promise.all([
    api().getOverview(team.slug, range), teamGitHub(team.orgId), overviewCi(team, range), observedCiComparisons(team, range),
    section(api().listTestHistory(team.slug, {days:Number(range.slice(0,-1))})),
    section(api().listTestRuns(team.slug, {limit:200})),
    runnerRate(team.orgId),
  ]);
  const valuationRate = rate.canSave ? rate : undefined;
  const t = ov.totals;
  // Match Overview: an empty served history is authoritative, not a reason to use an older list.
  const testRuns = history.ok && history.value !== null ? history.value : recent.ok && recent.value ? recent.value.map(lite) : null;
  const days = lastUtcDays(Number(range.slice(0,-1)), Date.parse(ov.asOf));
  const skipped = skippedExecution(testRuns, Date.parse(`${days[0]}T00:00:00Z`), Date.parse(ov.asOf));
  const testDays = skipped && testRuns ? testMinutesPerDay(testRuns.map(r => ({...r, timing:{...r.timing, selectMs:null}})), days) : null;
  const execution = skipped ? {minutes:skipped.minutes, note:`Estimated skipped work, before overhead · ${skipped.measured}/${skipped.runs} test runs${history.ok && history.value !== null ? "" : " · recent 200 only"}`} : undefined;
  const quod = ov.repos.some((r) => r.hasQuodData !== false) || skipped !== null;
  if (!gh.sample && !quod) {
    // Before Quod runs: CI minutes from GitHub's jobs.
    let link: GitHubLink = gh.link;
    let week: JobMinutes | null = null;
    let inRange: JobMinutes | null = null;
    if (gh.source) {
      try {
        const now = new Date();
        [week, inRange] = await Promise.all([
          teamJobMinutes(gh.source.teamId, gh.source.db, { now, days: 7 }),
          teamJobMinutes(gh.source.teamId, gh.source.db, { now, days: range === "7d" ? 7 : range === "30d" ? 30 : 90 }),
        ]);
      } catch (e) {
        unstable_rethrow(e);
        link = { state: "unavailable" };
      }
    }
    return (
      <div className="page">
        <div className="row-between">
          <h1 className="h1">Usage</h1>
          <RangeToggle base={`/${team.slug}/usage`} current={range} />
        </div>
        <GitHubUsage team={team.slug} plan={team.plan} range={range} state={ciState(link, !!inRange?.jobs)} week={week} inRange={inRange} />
        <LiveRefresh scope={{ kind: "team" }} types={["job.finished", "run.finished", "test.run"]} />
        <ObservedCiComparisons data={comparisons} valuationRate={valuationRate} />
      </div>
    );
  }
  const chartDays = testDays ?? ov.days;
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Usage</h1>
        <RangeToggle base={`/${team.slug}/usage`} current={range} />
      </div>
      <LiveRefresh scope={{ kind: "team" }} types={["job.finished", "run.finished", "test.run"]} />
      <SavingsSummary totals={t} ci={ci} execution={execution} valuationRate={valuationRate} />
      <section aria-label="Plan" className="stats">
        <Stat label="Plan" value={team.plan} note={team.plan === "Hobby" ? "Free for personal projects" : "Billed per team"} />
      </section>
      <ObservedCiComparisons data={comparisons} valuationRate={valuationRate} />
      <section className="card" aria-labelledby="usage-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="usage-h" className="h2">
            {testDays ? "Test execution per day" : "Step execution per day"}
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--blue)" }} />
              Avoided (estimated)
            </span>
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Ran
            </span>
          </div>
        </div>
        {chartDays.length ? <MinutesChart days={chartDays} from={RANGE_FROM[range]} source={testDays ? "tests" : "steps"} /> : <p className="muted">No execution observations yet.</p>}
        <p className="muted" style={{ fontSize: 13 }}>Execution avoided is an estimate before setup and cleanup overhead, not a reduction in the CI invoice. Observed full-job comparisons are shown separately above.</p>
      </section>
    </div>
  );
}
