import type { DayMinutes, RunTimePoint } from "@/lib/api/types";
import { formatCount, formatDay, formatDuration } from "@/lib/format";
import { formatSaved, formatSavedMs } from "@/lib/test-runs";

// Charts are plain HTML: every bar is focusable and has its value as its
// accessible name, the tooltip shows on hover and on keyboard focus, and the
// numbers are also in a table under "Show the numbers".

// Tooltips near the ends open inward so they stay on screen.
const edge = (i: number, n: number) => (i < n / 3 ? "edge-l" : i >= (2 * n) / 3 ? "edge-r" : "");

/**
 * CI time per day, saved and ran. `source: "tests"` (from test check runs,
 * lib/test-runs.ts testMinutesPerDay): times are seconds to minutes, shown
 * with formatSavedMs, and saved is an estimate ("≈ saved"). The table shows
 * the same values as the bars.
 */
export function MinutesChart({ days, from, source = "steps" }: { days: DayMinutes[]; from: string; source?: "steps" | "tests" }) {
  const tests = source === "tests";
  // Minimum scale: a whole minute for steps; for tests, the largest day (seconds count).
  const max = Math.max(tests ? Number.EPSILON : 1, ...days.map((d) => d.replayedMinutes + d.ranMinutes));
  const pct = (v: number) => `${((v / max) * 100).toFixed(2)}%`;
  const t = (min: number) => (tests ? formatSavedMs(min * 60_000) : `${formatCount(min)} min`);
  const saved = (min: number) => (tests ? formatSaved(min * 60_000) : `${formatCount(min)} min`);
  return (
    <>
      <ol className={`chart ${days.length > 40 ? "dense" : ""}`} aria-label={tests ? "Test time per day, saved (estimated) and ran" : "CI minutes per day, replayed and ran"}>
        {days.map((d, i) => {
          const label = tests
            ? `${formatDay(d.day)}: ${saved(d.replayedMinutes)} saved (estimated), ${t(d.ranMinutes)} ran`
            : `${formatDay(d.day)}: ${formatCount(d.replayedMinutes)} minutes replayed (saved), ${formatCount(d.ranMinutes)} minutes ran`;
          return (
            <li key={d.day} className={`bar ${edge(i, days.length)}`} tabIndex={0} aria-label={label}>
              <span className="tip" aria-hidden="true">
                <strong>{formatDay(d.day)}</strong>
                <br />
                Saved {saved(d.replayedMinutes)}, ran {t(d.ranMinutes)}
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
            <caption className="sr-only">{tests ? "Test check time per day" : "CI minutes per day"}</caption>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">{tests ? "≈ Saved" : "Replayed (saved), min"}</th>
                <th scope="col">{tests ? "Ran" : "Ran, min"}</th>
              </tr>
            </thead>
            <tbody>
              {[...days].reverse().map((d) => (
                <tr key={d.day}>
                  <td>{formatDay(d.day)}</td>
                  <td>{tests ? saved(d.replayedMinutes) : formatCount(d.replayedMinutes)}</td>
                  <td>{tests ? t(d.ranMinutes) : formatCount(d.ranMinutes)}</td>
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
      <ol className="chart short" aria-label="Step execution per run, observed and without replay estimated">
        {runs.map((r, i) => {
          const label = `Run #${r.number}: ${formatDuration(r.withQuodSeconds)} observed step execution, ${formatDuration(r.withoutSeconds)} without replay estimated`;
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
                {formatDuration(r.withQuodSeconds)} observed · {formatDuration(r.withoutSeconds)} without replay (estimated)
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
            <caption className="sr-only">Step execution per run</caption>
            <thead>
              <tr>
                <th scope="col">Run</th>
                <th scope="col">Observed steps</th>
                <th scope="col">Without replay (estimated)</th>
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
