import { MinutesChart } from "@/components/charts";
import { CiEmpty, type CiState } from "@/components/github-ci";
import { Stat } from "@/components/ui";
import type { Range } from "@/lib/api/types";
import { formatCount } from "@/lib/format";
import type { JobMinutes } from "@/lib/github/pages";
import { RANGE_FROM } from "@/lib/params";

/**
 * Usage for a team whose CI comes from GitHub, before Quod runs in it: job
 * time from GitHub's jobs (started to completed), and Quod's numbers marked
 * as appearing once Quod runs.
 */
export function GitHubUsage({ team, plan, range, state, week, inRange }: { team: string; plan: string; range: Range; state: CiState; week: JobMinutes | null; inRange: JobMinutes | null }) {
  const days = range === "7d" ? 7 : range === "30d" ? 30 : 90;
  const has = !!inRange && inRange.jobs > 0;
  return (
    <>
      <section aria-label="Usage totals" className="stats">
        <Stat label="CI minutes this week" value={week ? formatCount(week.totalMinutes) : "–"} note="job time on GitHub, last 7 days" />
        <Stat label={`CI minutes, ${days} days`} value={inRange ? formatCount(inRange.totalMinutes) : "–"} note={inRange ? `${formatCount(inRange.jobs)} ${inRange.jobs === 1 ? "job" : "jobs"}` : "from your GitHub jobs"} />
        <Stat label="CI minutes replayed" value="–" muted note="Appears once Quod runs in your CI" />
        <Stat label="Plan" value={plan} note={plan === "Hobby" ? "Free for personal projects" : "Billed per team"} />
      </section>
      <section className="card" aria-labelledby="usage-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="usage-h" className="h2">
            CI minutes per day
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Job time
            </span>
          </div>
        </div>
        {has && inRange ? (
          <MinutesChart days={inRange.days.map((d) => ({ day: d.day, replayedMinutes: 0, ranMinutes: d.minutes }))} from={RANGE_FROM[range]} />
        ) : (
          <CiEmpty team={team} state={state === "data" ? "waiting" : state} what="CI minutes" />
        )}
        <p className="muted" style={{ fontSize: 13 }}>
          Job time is how long your jobs ran, from GitHub. GitHub bills hosted runners by the minute, rounded up per job, so its invoice can be a little higher.
        </p>
      </section>
    </>
  );
}
