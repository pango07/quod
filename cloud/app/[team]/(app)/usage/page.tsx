import { unstable_rethrow } from "next/navigation";
import { MinutesChart } from "@/components/charts";
import { ciState } from "@/components/github-ci";
import { LiveRefresh } from "@/components/live/parts";
import { SavingsSummary } from "@/components/savings-summary";
import { GitHubUsage } from "@/components/usage-view";
import { RangeToggle } from "@/components/range-toggle";
import { Stat } from "@/components/ui";
import { api } from "@/lib/api";
import type { GitHubLink } from "@/lib/api/types";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { teamJobMinutes, type JobMinutes } from "@/lib/github/pages";
import { teamGitHub } from "@/lib/github/team-data";
import { skippedExecution } from "@/lib/job-savings";
import { overviewCi } from "@/lib/overview-ci";
import { parseRange, RANGE_FROM } from "@/lib/params";
import { lite } from "@/lib/savings";
import { lastUtcDays, testMinutesPerDay } from "@/lib/test-runs";

export const metadata = { title: "Usage" };

export default async function UsagePage({ params, searchParams }: PageProps<"/[team]/usage">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team } = await requireTeam(slug);
  const [ov, gh, ci, history, recent] = await Promise.all([
    api().getOverview(team.slug, range), teamGitHub(team.orgId), overviewCi(team, range),
    section(api().listTestHistory(team.slug, {days:Number(range.slice(0,-1))})),
    section(api().listTestRuns(team.slug, {limit:200})),
  ]);
  // A served, empty history is authoritative; fallback only for older/unavailable servers.
  const testRuns = history.ok && history.value !== null ? history.value : recent.ok && recent.value ? recent.value.map(lite) : null;
  const days = lastUtcDays(Number(range.slice(0,-1)), Date.parse(ov.asOf));
  const from = Date.parse(`${days[0]}T00:00:00Z`);
  const skipped = skippedExecution(testRuns, from, Date.parse(ov.asOf));
  const testDays = skipped && testRuns ? testMinutesPerDay(testRuns.map(r => ({...r, timing:{...r.timing, selectMs:null}})), days) : null;
  const execution = skipped ? {minutes:skipped.minutes, note:`Estimated skipped work, before overhead · ${skipped.measured}/${skipped.runs} test runs${history.ok && history.value !== null ? "" : " · recent 200 only"}`} : undefined;
  const t = ov.totals;
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
      <LiveRefresh scope={{kind:"team"}} types={["job.finished", "run.finished", "test.run"]} />
      <SavingsSummary totals={t} ci={ci} hasQuodData={quod} execution={execution} />
      <section aria-label="Plan" className="stats sm">
        <Stat label="Plan" value={team.plan} note={team.plan === "Hobby" ? "Free for personal projects" : "Billed per team"} />
      </section>
      <section className="card" aria-labelledby="usage-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="usage-h" className="h2">
            {testDays ? "Test execution per day" : "Step execution per day"}
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--blue)" }} />
              {testDays ? "≈ avoided" : "Execution avoided"}
            </span>
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Ran
            </span>
          </div>
        </div>
        {chartDays.length ? <MinutesChart days={chartDays} from={RANGE_FROM[range]} source={testDays ? "tests" : "steps"} /> : <p className="muted">No execution measurements yet.</p>}
        <p className="muted" style={{fontSize:13}}>
          The chart shows {testDays ? "test command time and estimated skipped-file execution" : "reported step execution"}. Whole-job savings also account for recording, audits, setup and cleanup. Dollar savings require billing rules and runner rates.
        </p>
      </section>
    </div>
  );
}
