import type { TeamTotals } from "@/lib/api/types";
import type { OverviewCi } from "@/lib/overview-ci";
import { formatCount, formatDuration, formatMinutes, formatPercent, formatUsd } from "@/lib/format";
import { savingsSummary } from "@/lib/job-savings";
import { formatRunnerValue, modeledRunnerValue, type RunnerValueRate } from "@/lib/runner-value";
import { rateText } from "@/lib/savings";
import { Stat } from "./ui";

export function SavingsSummary({totals, ci, hasQuodData = true, execution, valuationRate}: {totals:TeamTotals; ci:OverviewCi; hasQuodData?:boolean; execution?:{minutes:number|null; note:string}; valuationRate?:RunnerValueRate}) {
  const savings = savingsSummary(totals.jobSavings);
  const seconds = savings ? savings.seconds : execution ? execution.minutes === null ? null : execution.minutes * 60 : hasQuodData ? totals.savedMinutes * 60 : null;
  const value = modeledRunnerValue(seconds, valuationRate?.usdPerMinute);
  const valueLabel = savings ? seconds !== null && seconds < 0 ? "Modeled extra runner value" : "Modeled runner value saved" : "Est. execution value avoided";
  const valueNote = valuationRate ? `${rateText(valuationRate.usdPerMinute)} · ${valuationRate.source === "team" ? "your team's rate" : "default valuation rate"} · ${savings ? `${savings.estimated ? "estimated" : "observed"} job comparison` : "before overhead"}` : "Waiting for your runner price";
  const timeLabel = savings ? savings.seconds < 0 ? "Extra runner time" : "Runner time saved" : hasQuodData ? "Execution avoided" : "Time saved";
  return (
    <section aria-label="CI duration and savings" className="stats savings-stats">
      <Stat label="Median CI duration" value={ci.medianSeconds === null ? "—" : formatDuration(ci.medianSeconds)}
        note={ci.measuredRuns ? `${ci.sample ? "Sample: " : ""}${formatCount(ci.measuredRuns)} recent completed workflows` : "Waiting for completed workflows"} />
      <Stat label={timeLabel} value={savings ? formatDuration(Math.abs(savings.seconds)) : execution ? execution.minutes === null ? "Measuring" : formatMinutes(execution.minutes) : hasQuodData ? formatMinutes(totals.savedMinutes) : "Measuring"}
        note={savings ? `${savings.estimated ? "Estimated · " : ""}${savings.timeShare === null ? "" : formatPercent(Math.abs(savings.timeShare)) + (savings.seconds < 0 ? " more · " : " less · ")}${savings.jobs} compared jobs`
          : execution?.note ?? (hasQuodData ? "Estimated skipped work, before overhead" : "After Quod observes your CI")} />
      <Stat label={valueLabel} value={value === null ? "Measuring" : formatRunnerValue(Math.abs(value))} note={valueNote} />
      {savings?.costUsd !== null && savings?.costUsd !== undefined ? <Stat label={savings.costUsd < 0 ? "Est. extra runner cost" : "Est. runner cost saved"}
        value={formatUsd(Math.abs(savings.costUsd))}
        note={`Reported compute cost · ${formatCount(Math.abs(savings.minutes))} ${savings.minutes < 0 ? "extra" : "fewer"} rounded runner-minutes`} /> : null}
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
