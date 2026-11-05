import type { TeamTotals } from "@/lib/api/types";
import type { OverviewCi } from "@/lib/overview-ci";
import { formatCount, formatDuration, formatMinutes, formatPercent, formatUsd } from "@/lib/format";
import { savingsSummary } from "@/lib/savings";
import { Stat } from "./ui";

export function SavingsSummary({totals, ci, hasQuodData = true}: {totals:TeamTotals; ci:OverviewCi; hasQuodData?:boolean}) {
  const savings = savingsSummary(totals.jobSavings);
  const timeLabel = savings ? savings.seconds < 0 ? "Extra runner time" : "Runner time saved" : hasQuodData ? "Execution avoided" : "Time saved";
  return (
    <section aria-label="CI duration and savings" className="stats savings-stats">
      <Stat label="Median CI duration" value={ci.medianSeconds === null ? "—" : formatDuration(ci.medianSeconds)}
        note={ci.measuredRuns ? `${ci.sample ? "Sample: " : ""}${formatCount(ci.measuredRuns)} recent completed workflows` : "Waiting for completed workflows"} />
      <Stat label={timeLabel} value={savings ? formatDuration(Math.abs(savings.seconds)) : hasQuodData ? formatMinutes(totals.savedMinutes) : "Measuring"}
        note={savings ? `${savings.estimated ? "Estimated · " : ""}${savings.timeShare === null ? "" : formatPercent(Math.abs(savings.timeShare)) + (savings.seconds < 0 ? " more · " : " less · ")}${savings.jobs} compared jobs`
          : hasQuodData ? "Estimated skipped work, before overhead" : "After Quod observes your CI"} />
      <Stat label={savings?.costUsd !== null && savings?.costUsd !== undefined && savings.costUsd < 0 ? "Est. extra runner cost" : "Est. runner cost saved"}
        value={savings?.costUsd === null || savings?.costUsd === undefined ? "Measuring" : formatUsd(Math.abs(savings.costUsd))}
        note={savings?.costUsd === null || savings?.costUsd === undefined ? "Not enough billing data yet" : `${formatCount(Math.abs(savings.minutes))} ${savings.minutes < 0 ? "extra" : "fewer"} rounded runner-minutes`} />
    </section>
  );
}

export function RecentCi({ci}: {ci:OverviewCi}) {
  return (
    <section className="card" aria-labelledby="recent-ci-h" style={{overflow:"hidden"}}>
      <div className="card-head"><h2 id="recent-ci-h" className="h2">Recent CI</h2>{ci.sample ? <span className="chip small">Sample data</span> : null}</div>
      {ci.runs.length ? ci.runs.map(run => (
        <a key={run.id} className="trow" href={run.url} style={{display:"flex", alignItems:"center", gap:16}}>
          <span className="cell-main" style={{flex:1, minWidth:0}}><span className="ellipsis">{run.title}</span><span className="cell-sub">{run.repo}</span></span>
          <span className="muted" style={{fontSize:13}}>{run.status}</span>
          <span className="mono" style={{whiteSpace:"nowrap"}}>{formatDuration(run.seconds)}</span>
        </a>
      )) : <p className="muted" style={{padding:20}}>Completed workflows and their durations will appear here.</p>}
    </section>
  );
}
