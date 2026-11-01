import type { DayMinutes, RunTimePoint } from "@/lib/api/types";
import { formatCount, formatDay, formatDuration } from "@/lib/format";

// Charts are plain HTML: every bar is focusable and has its value as its
// accessible name, the tooltip shows on hover and on keyboard focus, and the
// numbers are also in a table under "Show the numbers".

// Tooltips near the ends open inward so they stay on screen.
const edge = (i: number, n: number) => (i < n / 3 ? "edge-l" : i >= (2 * n) / 3 ? "edge-r" : "");

export function MinutesChart({ days, from }: { days: DayMinutes[]; from: string }) {
  const max = Math.max(1, ...days.map((d) => d.replayedMinutes + d.ranMinutes));
  const pct = (v: number) => `${((v / max) * 100).toFixed(2)}%`;
  return (
    <>
      <ol className={`chart ${days.length > 40 ? "dense" : ""}`} aria-label="CI minutes per day, replayed and ran">
        {days.map((d, i) => {
          const label = `${formatDay(d.day)}: ${formatCount(d.replayedMinutes)} minutes replayed (saved), ${formatCount(d.ranMinutes)} minutes ran`;
          return (
            <li key={d.day} className={`bar ${edge(i, days.length)}`} tabIndex={0} aria-label={label}>
              <span className="tip" aria-hidden="true">
                <strong>{formatDay(d.day)}</strong>
                <br />
                Saved {formatCount(d.replayedMinutes)} min, ran {formatCount(d.ranMinutes)} min
              </span>
              <span className="seg-blue" style={{ height: pct(d.replayedMinutes) }} />
              <span className="seg-ran" style={{ height: pct(d.ranMinutes) }} />
            </li>
          );
        })}
      </ol>
      <div className="axis" aria-hidden="true">
        <span>{from}</span>
        <span>Today</span>
      </div>
      <details className="data-table">
        <summary>Show the numbers</summary>
        <div className="scroll">
          <table>
            <caption className="sr-only">CI minutes per day</caption>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">Replayed (saved), min</th>
                <th scope="col">Ran, min</th>
              </tr>
            </thead>
            <tbody>
              {[...days].reverse().map((d) => (
                <tr key={d.day}>
                  <td>{formatDay(d.day)}</td>
                  <td>{formatCount(d.replayedMinutes)}</td>
                  <td>{formatCount(d.ranMinutes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </>
  );
}

export function RunTimeChart({ runs }: { runs: RunTimePoint[] }) {
  const max = Math.max(1, ...runs.map((r) => r.withoutSeconds));
  return (
    <>
      <ol className="chart short" aria-label="Run time per run, with Quod and without">
        {runs.map((r, i) => {
          const label = `Run #${r.number}: ${formatDuration(r.withQuodSeconds)} with Quod, ${formatDuration(r.withoutSeconds)} without`;
          return (
            <li
              key={r.number}
              className={`runbar ${edge(i, runs.length)}`}
              style={{ height: `${((r.withoutSeconds / max) * 100).toFixed(2)}%` }}
              tabIndex={0}
              aria-label={label}
            >
              <span className="tip" aria-hidden="true">
                <strong>Run #{r.number}</strong>
                <br />
                {formatDuration(r.withQuodSeconds)} with Quod · {formatDuration(r.withoutSeconds)} without
              </span>
              <span className="fill" style={{ height: `${Math.min(100, (r.withQuodSeconds / r.withoutSeconds) * 100).toFixed(2)}%` }} />
            </li>
          );
        })}
      </ol>
      <details className="data-table">
        <summary>Show the numbers</summary>
        <div className="scroll">
          <table>
            <caption className="sr-only">Run time per run</caption>
            <thead>
              <tr>
                <th scope="col">Run</th>
                <th scope="col">With Quod</th>
                <th scope="col">Without (recorded)</th>
              </tr>
            </thead>
            <tbody>
              {[...runs].reverse().map((r) => (
                <tr key={r.number}>
                  <td>#{r.number}</td>
                  <td>{formatDuration(r.withQuodSeconds)}</td>
                  <td>{formatDuration(r.withoutSeconds)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </>
  );
}
