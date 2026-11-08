import type { ObservedCiComparisons as ComparisonData } from "@/lib/observed-ci-comparisons";
import { formatDuration, shortSha } from "@/lib/format";
import { formatRunnerValue, modeledRunnerValue, type RunnerValueRate } from "@/lib/runner-value";
import { rateText } from "@/lib/savings";

export function ObservedCiComparisons({ data, valuationRate }: { data: ComparisonData; valuationRate?: RunnerValueRate }) {
  if (!data.visible) return null;
  return (
    <section className="card" aria-labelledby="observed-ci-h" style={{ overflow: "hidden" }}>
      <div className="card-head"><h2 id="observed-ci-h" className="h2">Observed full-job comparison</h2><span className="chip small">pango07/quod</span></div>
      <p className="muted" style={{ padding: "0 20px", fontSize: 13, lineHeight: 1.6 }}>
        Plain and through-Quod jobs from the same successful manual workflow run, source commit and attempt.
        Times include setup, install and cleanup. Separate runners and cache conditions can affect the difference.
      </p>
      {data.pairs.length ? (
        <div className="data-table" style={{ padding: "0 20px" }}><div className="scroll"><table style={{ minWidth: 960 }}>
          <caption className="sr-only">Observed complete job durations, without estimated baselines</caption>
          <thead><tr><th scope="col">Source</th><th scope="col">Through Quod</th><th scope="col">Plain baseline</th><th scope="col">Observed difference</th><th scope="col">Modeled time value</th><th scope="col">Runner compute cost</th></tr></thead>
          <tbody>{data.pairs.map(pair => {
            const value = modeledRunnerValue(pair.savedSeconds, valuationRate?.usdPerMinute);
            return (
            <tr key={`${pair.runId}-${pair.attempt}`}>
              <td><a className="link-muted mono" href={`https://github.com/pango07/quod/commit/${pair.sha}`}>{shortSha(pair.sha)}</a><br /><a className="link-muted" href={pair.runUrl}>Run {pair.runId} · attempt {pair.attempt}</a><div className="cell-sub"><time dateTime={pair.finishedAt}>{pair.finishedAt.slice(0, 10)} UTC</time></div></td>
              <td><a href={pair.candidate.url}>{pair.candidate.seconds}s</a><div className="cell-sub">{formatDuration(pair.candidate.seconds)}</div></td>
              <td><a href={pair.baseline.url}>{pair.baseline.seconds}s</a><div className="cell-sub">{formatDuration(pair.baseline.seconds)}</div></td>
              <td>{formatDuration(Math.abs(pair.savedSeconds))} {pair.savedSeconds < 0 ? "more" : pair.savedSeconds > 0 ? "less" : "difference"}</td>
              <td>{value === null ? "Measuring" : <>{formatRunnerValue(Math.abs(value))} {value < 0 ? "extra" : value > 0 ? "saved" : "difference"}</>}</td>
              <td>{pair.cost === "public-standard-free" ? "$0.00 each" : "Measuring"}</td>
            </tr>
          ); })}</tbody>
        </table></div></div>
      ) : <p className="muted" style={{ padding: "0 20px" }}>{data.available ? "Waiting for a completed comparison. Run quod-cloud manually with compare enabled; both jobs must succeed in the same attempt." : "GitHub comparison data is temporarily unavailable."}</p>}
      <p className="muted" style={{ padding: "0 20px", fontSize: 13, lineHeight: 1.6 }}>
        {valuationRate ? <>Modeled time value uses {rateText(valuationRate.usdPerMinute)} ({valuationRate.source === "team" ? "your team's rate" : "default valuation rate"}). Saved or extra runner seconds are valued at this current rate, even when compute is free.</> : "Runner price is unavailable; modeled time value is waiting for a price."}
        {" "}This is a valuation of the observed time difference; it is separate from compute charges.
      </p>
      <p className="muted" style={{ padding: "0 20px 16px", fontSize: 13, lineHeight: 1.6 }}>
        Latest 100 repository runs in this range; up to 12 completed manual runs inspected. No historical baseline is inferred.
        {" "}<a className="link-muted" href="https://docs.github.com/en/billing/concepts/product-billing/github-actions">Standard hosted runners in public repositories are free</a>.
        {" "}$0 requires public-repository and standard Ubuntu runner evidence for both jobs; otherwise cost is unknown. Storage and Quod subscription costs are excluded.
      </p>
    </section>
  );
}
